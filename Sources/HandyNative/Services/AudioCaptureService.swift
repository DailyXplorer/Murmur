import AudioToolbox
import AVFoundation
import CoreAudio
import Foundation

enum AudioCaptureError: LocalizedError {
    case alreadyRecording
    case notRecording
    case unavailableInput

    var errorDescription: String? {
        switch self {
        case .alreadyRecording: "Recording is already running."
        case .notRecording: "No recording is active."
        case .unavailableInput: "No microphone input is available."
        }
    }
}

@MainActor
final class AudioCaptureService {
    private var engine: AVAudioEngine?
    private var accumulator: AudioSampleAccumulator?
    private var activeMicrophoneName: String?
    private var activeVoiceProcessingConfiguration: AudioInputVoiceProcessingConfiguration?
    private var lazyCloseTask: Task<Void, Never>?
    private var lazyCloseToken = UUID()

    private(set) var voiceProcessingStatus: AudioInputVoiceProcessingStatus = .notConfigured

    init() {}

    var isRecording: Bool {
        accumulator != nil
    }

    func start(
        selectedMicrophoneName: String?,
        voiceProcessingConfiguration: AudioInputVoiceProcessingConfiguration = .handyDefault,
        onLevel: @escaping @Sendable (Float) -> Void
    ) throws {
        guard accumulator == nil else {
            throw AudioCaptureError.alreadyRecording
        }
        lazyCloseTask?.cancel()

        if engine != nil,
           activeMicrophoneName != selectedMicrophoneName ||
            activeVoiceProcessingConfiguration != voiceProcessingConfiguration {
            closeEngine()
        }

        let engine = try ensureEngine(
            selectedMicrophoneName: selectedMicrophoneName,
            voiceProcessingConfiguration: voiceProcessingConfiguration
        )
        let inputNode = engine.inputNode
        let inputFormat = inputNode.outputFormat(forBus: 0)
        guard inputFormat.channelCount > 0, inputFormat.sampleRate > 0 else {
            throw AudioCaptureError.unavailableInput
        }

        let accumulator = AudioSampleAccumulator(sampleRate: inputFormat.sampleRate)
        Self.installCaptureTap(
            on: inputNode,
            format: inputFormat,
            accumulator: accumulator,
            onLevel: onLevel
        )

        do {
            if engine.isRunning == false {
                try engine.start()
            }
        } catch {
            inputNode.removeTap(onBus: 0)
            throw error
        }

        self.engine = engine
        self.accumulator = accumulator
        activeMicrophoneName = selectedMicrophoneName
        activeVoiceProcessingConfiguration = voiceProcessingConfiguration
    }

    func stop(keepStreamOpen: Bool = false, lazyClose: Bool = false) throws -> AudioRecording {
        guard let engine, let accumulator else {
            throw AudioCaptureError.notRecording
        }

        engine.inputNode.removeTap(onBus: 0)
        self.accumulator = nil
        closeOrScheduleEngineClose(keepStreamOpen: keepStreamOpen, lazyClose: lazyClose)

        return accumulator.recording()
    }

    func cancel(keepStreamOpen: Bool = false, lazyClose: Bool = false) {
        if accumulator != nil {
            engine?.inputNode.removeTap(onBus: 0)
        }
        accumulator = nil
        closeOrScheduleEngineClose(keepStreamOpen: keepStreamOpen, lazyClose: lazyClose)
    }

    func openIdleStream(
        selectedMicrophoneName: String?,
        voiceProcessingConfiguration: AudioInputVoiceProcessingConfiguration = .handyDefault
    ) throws {
        guard accumulator == nil else {
            return
        }

        _ = try ensureEngine(
            selectedMicrophoneName: selectedMicrophoneName,
            voiceProcessingConfiguration: voiceProcessingConfiguration
        )
    }

    func closeIdleStream() {
        guard accumulator == nil else {
            return
        }

        closeEngine()
    }

    private func closeOrScheduleEngineClose(keepStreamOpen: Bool, lazyClose: Bool) {
        lazyCloseTask?.cancel()
        guard keepStreamOpen == false else {
            return
        }

        guard lazyClose, engine != nil else {
            closeEngine()
            return
        }

        let token = UUID()
        lazyCloseToken = token
        lazyCloseTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(30))
            await MainActor.run {
                guard let self,
                      self.lazyCloseToken == token,
                      self.accumulator == nil
                else {
                    return
                }

                self.closeEngine()
            }
        }
    }

    private func ensureEngine(
        selectedMicrophoneName: String?,
        voiceProcessingConfiguration: AudioInputVoiceProcessingConfiguration
    ) throws -> AVAudioEngine {
        if let engine,
           activeMicrophoneName == selectedMicrophoneName,
           activeVoiceProcessingConfiguration == voiceProcessingConfiguration {
            if engine.isRunning == false {
                try engine.start()
            }
            return engine
        }

        closeEngine()

        let engine = AVAudioEngine()
        let inputNode = engine.inputNode
        let selectedDeviceID = AudioDeviceService.inputDeviceID(named: selectedMicrophoneName)
        voiceProcessingStatus = try AudioInputVoiceProcessingInputPreparation.prepare(
            selectedDeviceID: selectedDeviceID,
            setInputDevice: { deviceID in
                try Self.setInputDevice(deviceID, on: inputNode)
            },
            configureVoiceProcessing: {
                try AudioInputVoiceProcessingConfigurator.configure(
                    inputNode,
                    configuration: voiceProcessingConfiguration
                )
            }
        )
        let inputFormat = inputNode.outputFormat(forBus: 0)
        guard inputFormat.channelCount > 0, inputFormat.sampleRate > 0 else {
            throw AudioCaptureError.unavailableInput
        }

        try engine.start()
        self.engine = engine
        activeMicrophoneName = selectedMicrophoneName
        activeVoiceProcessingConfiguration = voiceProcessingConfiguration
        return engine
    }

    private func closeEngine() {
        lazyCloseTask?.cancel()
        engine?.stop()
        engine = nil
        activeMicrophoneName = nil
        activeVoiceProcessingConfiguration = nil
        voiceProcessingStatus = .notConfigured
    }

    private static func setInputDevice(_ deviceID: AudioDeviceID, on inputNode: AVAudioInputNode) throws {
        guard let audioUnit = inputNode.audioUnit else {
            return
        }

        var mutableDeviceID = deviceID
        let status = AudioUnitSetProperty(
            audioUnit,
            kAudioOutputUnitProperty_CurrentDevice,
            kAudioUnitScope_Global,
            0,
            &mutableDeviceID,
            UInt32(MemoryLayout<AudioDeviceID>.size)
        )

        if status != noErr {
            throw NSError(
                domain: NSOSStatusErrorDomain,
                code: Int(status),
                userInfo: [NSLocalizedDescriptionKey: "Unable to use selected microphone."]
            )
        }
    }

    nonisolated private static func installCaptureTap(
        on inputNode: AVAudioInputNode,
        format: AVAudioFormat,
        accumulator: AudioSampleAccumulator,
        onLevel: @escaping @Sendable (Float) -> Void
    ) {
        inputNode.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in
            let level = accumulator.append(buffer)
            onLevel(level)
        }
    }
}
