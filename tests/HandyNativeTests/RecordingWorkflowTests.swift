import Foundation
@testable import HandyNative
import XCTest

@MainActor
final class RecordingWorkflowTests: XCTestCase {
    func testStartRunsCaptureBeforeStartFeedback() throws {
        let eventLog = RecordingWorkflowEventLog()
        let workflow = Self.makeWorkflow(eventLog: eventLog)
        var settings = AppSettings.defaults
        settings.audioFeedback = true

        try workflow.start(
            settings: settings,
            paths: Self.makePaths(),
            selectedMicrophoneName: "Studio Mic"
        ) { _ in }

        XCTAssertEqual(eventLog.events, ["capture.start:Studio Mic", "voice-processing:true", "feedback.start"])
    }

    func testStartPassesDisabledAppleVoiceProcessingConfigurationFromSettings() throws {
        let eventLog = RecordingWorkflowEventLog()
        let workflow = Self.makeWorkflow(eventLog: eventLog)
        var settings = AppSettings.defaults
        settings.appleVoiceProcessingEnabled = false

        try workflow.start(
            settings: settings,
            paths: Self.makePaths(),
            selectedMicrophoneName: nil
        ) { _ in }

        XCTAssertEqual(eventLog.events, ["capture.start:default", "voice-processing:false", "feedback.start"])
    }

    func testStopRunsThroughTrailingBufferBeforeCaptureAndEffects() async throws {
        let eventLog = RecordingWorkflowEventLog()
        let sequencer = RecordingStopSequencer { milliseconds in
            eventLog.append("sleep:\(milliseconds)")
        }
        let workflow = Self.makeWorkflow(eventLog: eventLog, stopSequencer: sequencer)
        var settings = AppSettings.defaults
        settings.audioFeedback = true
        settings.alwaysOnMicrophone = true
        settings.extraRecordingBufferMilliseconds = 125

        _ = try await workflow.stopAfterTrailingBuffer(settings: settings, paths: Self.makePaths())

        XCTAssertEqual(
            eventLog.events,
            ["sleep:125", "capture.stop:true:false", "mute.remove", "feedback.stop"]
        )
    }

    func testMuteAfterStartFeedbackOnlyAppliesWhenRecordingStillActive() async {
        let eventLog = RecordingWorkflowEventLog()
        let workflow = Self.makeWorkflow(
            eventLog: eventLog,
            startFeedbackDelaySleep: { milliseconds in
                eventLog.append("delay:\(milliseconds)")
            }
        )

        await workflow.applyMuteAfterStartFeedback(settings: AppSettings.defaults) {
            true
        }

        XCTAssertEqual(eventLog.events, ["delay:250", "mute.apply"])
    }

    func testCancelStopsCaptureAndReleasesMute() {
        let eventLog = RecordingWorkflowEventLog()
        let workflow = Self.makeWorkflow(eventLog: eventLog)
        var settings = AppSettings.defaults
        settings.alwaysOnMicrophone = true
        settings.lazyStreamClose = true

        workflow.cancel(settings: settings)

        XCTAssertEqual(eventLog.events, ["capture.cancel:true:true", "mute.remove"])
    }

    private static func makeWorkflow(
        eventLog: RecordingWorkflowEventLog,
        stopSequencer: RecordingStopSequencer = RecordingStopSequencer(),
        startFeedbackDelaySleep: @escaping @MainActor @Sendable (Int) async throws -> Void = { _ in }
    ) -> RecordingWorkflow {
        RecordingWorkflow(
            audioCaptureService: RecordingWorkflowFakeAudioCaptureService(eventLog: eventLog),
            audioFeedbackService: RecordingWorkflowFakeAudioFeedbackService(eventLog: eventLog),
            systemAudioMuteService: RecordingWorkflowFakeSystemAudioMuteService(eventLog: eventLog),
            stopSequencer: stopSequencer,
            startFeedbackDelaySleep: startFeedbackDelaySleep
        )
    }

    private static func makePaths() -> AppPaths {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("RecordingWorkflowTests-\(UUID().uuidString)", isDirectory: true)
        return AppPaths(
            appDataDirectory: root.appendingPathComponent("app-data", isDirectory: true),
            recordingsDirectory: root.appendingPathComponent("recordings", isDirectory: true),
            modelsDirectory: root.appendingPathComponent("models", isDirectory: true),
            logsDirectory: root.appendingPathComponent("logs", isDirectory: true)
        )
    }
}

@MainActor
private final class RecordingWorkflowEventLog {
    private(set) var events: [String] = []

    func append(_ event: String) {
        events.append(event)
    }
}

@MainActor
private final class RecordingWorkflowFakeAudioCaptureService: AudioCaptureServicing {
    var isRecording = true
    var voiceProcessingStatus: AudioInputVoiceProcessingStatus = .notConfigured
    private let eventLog: RecordingWorkflowEventLog

    init(eventLog: RecordingWorkflowEventLog) {
        self.eventLog = eventLog
    }

    func start(
        selectedMicrophoneName: String?,
        voiceProcessingConfiguration: AudioInputVoiceProcessingConfiguration,
        onLevel _: @escaping @Sendable (Float) -> Void
    ) throws {
        eventLog.append("capture.start:\(selectedMicrophoneName ?? "default")")
        eventLog.append("voice-processing:\(voiceProcessingConfiguration.isEnabled)")
        voiceProcessingStatus = voiceProcessingConfiguration.isEnabled ? .enabled(automaticGainControlEnabled: true) : .disabled
    }

    func stop(keepStreamOpen: Bool, lazyClose: Bool) throws -> AudioRecording {
        eventLog.append("capture.stop:\(keepStreamOpen):\(lazyClose)")
        return AudioRecording(
            samples: Array(repeating: 0.02, count: 16_000),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 1)
        )
    }

    func cancel(keepStreamOpen: Bool, lazyClose: Bool) {
        eventLog.append("capture.cancel:\(keepStreamOpen):\(lazyClose)")
    }

    func openIdleStream(
        selectedMicrophoneName _: String?,
        voiceProcessingConfiguration: AudioInputVoiceProcessingConfiguration
    ) throws {
        voiceProcessingStatus = voiceProcessingConfiguration.isEnabled ? .enabled(automaticGainControlEnabled: true) : .disabled
    }

    func closeIdleStream() {}
}

@MainActor
private final class RecordingWorkflowFakeAudioFeedbackService: AudioFeedbackPlaying {
    private let eventLog: RecordingWorkflowEventLog

    init(eventLog: RecordingWorkflowEventLog) {
        self.eventLog = eventLog
    }

    func play(_ sound: AudioFeedbackSound, settings _: AppSettings, paths _: AppPaths) {
        eventLog.append("feedback.\(sound.rawValue)")
    }
}

@MainActor
private final class RecordingWorkflowFakeSystemAudioMuteService: SystemAudioMuting {
    private let eventLog: RecordingWorkflowEventLog

    init(eventLog: RecordingWorkflowEventLog) {
        self.eventLog = eventLog
    }

    func applyMuteIfNeeded(settings _: AppSettings) {
        eventLog.append("mute.apply")
    }

    func removeMuteIfNeeded() {
        eventLog.append("mute.remove")
    }
}
