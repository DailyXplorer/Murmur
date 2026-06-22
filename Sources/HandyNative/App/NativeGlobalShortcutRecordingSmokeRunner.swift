import AppKit
import Foundation

enum NativeGlobalShortcutRecordingSmokeRunner {
    @MainActor
    static func runSynchronouslyAndExit(_ request: NativeGlobalShortcutRecordingSmokeRequest) -> Never {
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)
        application.finishLaunching()

        Task { @MainActor in
            do {
                let output = try await output(for: request)
                let outputString = try outputString(for: output)
                if let outputPath = request.outputPath {
                    try write(outputString, to: outputPath)
                } else {
                    FileHandle.standardOutput.writeLine(outputString)
                }
                exit(output.success ? 0 : 1)
            } catch {
                FileHandle.standardError.writeLine(error.localizedDescription)
                exit(1)
            }
        }

        application.run()
        exit(1)
    }

    @MainActor
    static func output(
        for request: NativeGlobalShortcutRecordingSmokeRequest
    ) async throws -> NativeGlobalShortcutRecordingSmokeOutput {
        guard let descriptor = GlobalShortcutDescriptor.parse(request.binding) else {
            throw NativeGlobalShortcutRecordingSmokeError.invalidBinding(request.binding)
        }

        let permissionSnapshot = PermissionService().snapshot()
        guard permissionSnapshot.accessibilityTrusted,
              permissionSnapshot.microphone == .granted
        else {
            return NativeGlobalShortcutRecordingSmokeOutput(
                success: false,
                requestedBindingID: request.bindingID,
                requestedBinding: request.binding,
                keyCode: Int(descriptor.keyCode),
                requiredFlagsRawValue: Int(descriptor.requiredFlags.rawValue),
                accessibilityTrusted: permissionSnapshot.accessibilityTrusted,
                microphonePermission: permissionSnapshot.microphone.rawValue,
                pushToTalk: true,
                holdDurationMilliseconds: request.durationMilliseconds,
                eventTapRunning: false,
                keyDownPostSucceeded: false,
                keyUpPostSucceeded: false,
                pressedBindingIDs: [],
                releasedBindingIDs: [],
                startedRecording: false,
                stoppedRecording: false,
                recordingSampleCount: nil,
                recordingSampleRate: nil,
                recordingDurationSeconds: nil,
                recordingMaxLevel: nil,
                levelObservationCount: nil,
                recordingHasAudibleSignal: nil,
                recordingOutputPath: request.recordingOutputPath,
                recordingByteCount: nil,
                errorMessage: NativeGlobalShortcutRecordingSmokeError.permissionMissing(
                    accessibilityTrusted: permissionSnapshot.accessibilityTrusted,
                    microphone: permissionSnapshot.microphone
                ).localizedDescription
            )
        }

        let controller = NativeGlobalShortcutRecordingSmokeController(request: request)
        let shortcutService = GlobalShortcutService()
        try shortcutService.start(
            registrations: [
                GlobalShortcutRegistration(
                    bindingID: request.bindingID,
                    descriptor: descriptor
                )
            ],
            onPressed: { bindingID in
                Task { @MainActor in
                    controller.handlePressed(bindingID)
                }
            },
            onReleased: { bindingID in
                Task { @MainActor in
                    controller.handleReleased(bindingID)
                }
            }
        )
        defer {
            controller.cancelIfNeeded()
            shortcutService.stop()
        }

        try await Task.sleep(for: .milliseconds(150))
        try postKey(descriptor, keyDown: true)
        let startedRecording = await controller.waitUntilRecordingStarts(timeoutMilliseconds: 2_000)
        try await Task.sleep(for: .milliseconds(request.durationMilliseconds))
        try postKey(descriptor, keyDown: false)
        let stoppedRecording = await controller.waitUntilRecordingStops(timeoutMilliseconds: 3_000)

        var snapshot = controller.snapshot()
        var recordingByteCount: Int64?
        if let recording = snapshot.recording,
           let recordingOutputPath = request.recordingOutputPath {
            let recordingOutputURL = URL(fileURLWithPath: recordingOutputPath)
            try WAVFileWriter.write(recording, to: recordingOutputURL)
            let attributes = try FileManager.default.attributesOfItem(atPath: recordingOutputURL.path)
            recordingByteCount = (attributes[.size] as? NSNumber)?.int64Value ?? 0
            snapshot.recordingOutputPath = recordingOutputURL.path
        }

        let recording = snapshot.recording
        var transcriptionOutput: NativeRecordTranscriptionSmokeOutput?
        var errorMessage = snapshot.errorMessage
        if request.transcribeAfterRecording, errorMessage == nil {
            do {
                transcriptionOutput = try await transcribe(
                    request: request,
                    recording: try requiredRecording(recording),
                    maxLevel: snapshot.maxLevel,
                    levelObservationCount: snapshot.levelObservationCount
                )
                snapshot.recordingOutputPath = transcriptionOutput?.outputPath ?? snapshot.recordingOutputPath
                recordingByteCount = transcriptionOutput?.byteCount ?? recordingByteCount
            } catch {
                errorMessage = error.localizedDescription
            }
        }

        let transcriptionSucceeded = request.transcribeAfterRecording == false ||
            (transcriptionOutput?.outputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false &&
                pasteSucceeded(transcriptionOutput))
        let success = shortcutService.isRunning &&
            startedRecording &&
            stoppedRecording &&
            errorMessage == nil &&
            (recording?.isEmpty == false) &&
            transcriptionSucceeded

        return NativeGlobalShortcutRecordingSmokeOutput(
            success: success,
            requestedBindingID: request.bindingID,
            requestedBinding: request.binding,
            keyCode: Int(descriptor.keyCode),
            requiredFlagsRawValue: Int(descriptor.requiredFlags.rawValue),
            accessibilityTrusted: true,
            microphonePermission: permissionSnapshot.microphone.rawValue,
            pushToTalk: true,
            holdDurationMilliseconds: request.durationMilliseconds,
            eventTapRunning: shortcutService.isRunning,
            keyDownPostSucceeded: true,
            keyUpPostSucceeded: true,
            pressedBindingIDs: snapshot.pressedBindingIDs,
            releasedBindingIDs: snapshot.releasedBindingIDs,
            startedRecording: startedRecording,
            stoppedRecording: stoppedRecording,
            recordingSampleCount: recording?.samples.count,
            recordingSampleRate: recording?.sampleRate,
            recordingDurationSeconds: recording?.duration,
            recordingMaxLevel: snapshot.maxLevel,
            levelObservationCount: snapshot.levelObservationCount,
            recordingHasAudibleSignal: recording?.hasAudibleSignal,
            recordingOutputPath: snapshot.recordingOutputPath ?? request.recordingOutputPath,
            recordingByteCount: recordingByteCount,
            transcription: transcriptionOutput,
            errorMessage: errorMessage
        )
    }

    private static func requiredRecording(_ recording: AudioRecording?) throws -> AudioRecording {
        guard let recording,
              recording.isEmpty == false
        else {
            throw NativeGlobalShortcutRecordingSmokeError.missingRecording
        }
        guard recording.hasAudibleSignal else {
            throw NativeGlobalShortcutRecordingSmokeError.silentRecording
        }
        return recording
    }

    private static func transcribe(
        request: NativeGlobalShortcutRecordingSmokeRequest,
        recording capturedRecording: AudioRecording,
        maxLevel: Float,
        levelObservationCount: Int
    ) async throws -> NativeRecordTranscriptionSmokeOutput {
        let recording = capturedRecording.preparedForTranscriptionInput()
        guard recording.isEmpty == false else {
            throw NativeGlobalShortcutRecordingSmokeError.silentRecording
        }

        let outputURL = try transcriptionRecordingURL(for: request)
        let shouldRemoveTemporaryRecording = request.recordingOutputPath == nil
        defer {
            if shouldRemoveTemporaryRecording {
                try? FileManager.default.removeItem(at: outputURL)
            }
        }

        try WAVFileWriter.write(recording, to: outputURL)
        let attributes = try FileManager.default.attributesOfItem(atPath: outputURL.path)
        let byteCount = (attributes[.size] as? NSNumber)?.int64Value ?? 0
        let paths = try AppPaths.resolve()
        let settingsConfiguration = try await settingsConfiguration(for: request, paths: paths)
        let result = try await AudioFileTranscriptionPipeline.transcribe(
            fileURL: outputURL,
            settings: settingsConfiguration.settings,
            paths: paths,
            credentialStore: settingsConfiguration.credentialStore,
            appleSpeechTranscriptionService: settingsConfiguration.appleSpeechTranscriptionService,
            postProcessRequested: request.postProcessRequested
        )

        let historyResult: NativeGlobalShortcutRecordingHistoryResult?
        if request.recordHistory {
            historyResult = try saveHistory(
                fileURL: outputURL,
                result: result,
                postProcessRequested: request.postProcessRequested,
                paths: paths
            )
        } else {
            historyResult = nil
        }

        var output = NativeRecordTranscriptionSmokeOutput(
            outputPath: outputURL.path,
            requestedDurationMilliseconds: request.durationMilliseconds,
            capturedSampleCount: capturedRecording.samples.count,
            processedSampleCount: recording.samples.count,
            sampleRate: recording.sampleRate,
            durationSeconds: recording.duration,
            maxLevel: maxLevel,
            levelObservationCount: levelObservationCount,
            byteCount: byteCount,
            microphoneName: request.microphoneName,
            modelID: settingsConfiguration.settings.selectedModel,
            modelDisplayName: settingsConfiguration.settings.selectedTranscriptionModelDisplayName,
            language: settingsConfiguration.settings.selectedLanguage,
            usedSelectedSettings: request.useSelectedSettings,
            postProcessRequested: request.postProcessRequested,
            transcriptionText: result.transcriptionText,
            outputText: result.outputText,
            historyEntryID: historyResult?.entryID,
            recordingFileName: historyResult?.recordingFileName,
            paste: nil
        )

        if let pasteRequest = request.pasteRequest {
            if pasteRequest.externalRoundTrip {
                let externalPasteOutput = try await NativeExternalPasteRoundTripSmokeRunner.output(
                    for: pasteRequest.externalRoundTripSmokeRequest(text: output.outputText)
                )
                output.paste = externalPasteOutput.paste
                output.externalPaste = externalPasteOutput
            } else {
                output.paste = try await NativePasteSmokeRunner.output(
                    for: pasteRequest.pasteSmokeRequest(text: output.outputText)
                )
            }
        }

        return output
    }

    private static func pasteSucceeded(_ output: NativeRecordTranscriptionSmokeOutput?) -> Bool {
        guard let output else {
            return false
        }
        if let externalPaste = output.externalPaste {
            return externalPaste.success
        }
        if let paste = output.paste {
            return paste.success && (paste.targetWindow == false || paste.targetMatchesPreparedText)
        }
        return true
    }

    private static func transcriptionRecordingURL(
        for request: NativeGlobalShortcutRecordingSmokeRequest
    ) throws -> URL {
        if let recordingOutputPath = request.recordingOutputPath {
            return URL(fileURLWithPath: recordingOutputPath)
        }

        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("handy-shortcut-record-transcribe-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory.appendingPathComponent("recording.wav")
    }

    private static func settingsConfiguration(
        for request: NativeGlobalShortcutRecordingSmokeRequest,
        paths: AppPaths
    ) async throws -> NativeSmokeTranscriptionSettingsConfiguration {
        try await NativeSmokeTranscriptionSettingsResolver.configuration(
            modelID: request.modelID,
            language: request.language,
            useSelectedSettings: request.useSelectedSettings,
            paths: paths
        )
    }

    private static func saveHistory(
        fileURL: URL,
        result: ProcessedAudioTranscription,
        postProcessRequested: Bool,
        paths: AppPaths
    ) throws -> NativeGlobalShortcutRecordingHistoryResult {
        let historyStore = try HistoryStore(paths: paths)
        let fileName = RecordingFileNameFormatter.fileName(for: Date())
        let destinationURL = historyStore.audioFileURL(fileName: fileName)
        try FileManager.default.copyItem(at: fileURL, to: destinationURL)
        let entry = try historyStore.saveEntry(
            fileName: fileName,
            transcriptionText: "",
            postProcessRequested: postProcessRequested
        )
        let updatedEntry = try historyStore.updateTranscription(
            id: entry.id,
            transcriptionText: result.transcriptionText,
            postProcessedText: result.postProcessedText,
            postProcessPrompt: result.postProcessPrompt
        )
        return NativeGlobalShortcutRecordingHistoryResult(
            entryID: updatedEntry.id,
            recordingFileName: updatedEntry.fileName
        )
    }

    private static func postKey(_ descriptor: GlobalShortcutDescriptor, keyDown: Bool) throws {
        guard
            let source = CGEventSource(stateID: .hidSystemState),
            let event = CGEvent(
                keyboardEventSource: source,
                virtualKey: descriptor.keyCode,
                keyDown: keyDown
            )
        else {
            throw NativeGlobalShortcutRecordingSmokeError.eventCreationFailed
        }

        event.flags = descriptor.requiredFlags
        event.post(tap: .cgSessionEventTap)
    }

    private static func write(_ output: String, to path: String) throws {
        let outputURL = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try output.write(to: outputURL, atomically: true, encoding: .utf8)
    }

    private static func outputString(for output: NativeGlobalShortcutRecordingSmokeOutput) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

struct NativeGlobalShortcutRecordingSmokeOutput: Encodable {
    var success: Bool
    var requestedBindingID: String
    var requestedBinding: String
    var keyCode: Int
    var requiredFlagsRawValue: Int
    var accessibilityTrusted: Bool
    var microphonePermission: String
    var pushToTalk: Bool
    var holdDurationMilliseconds: Int
    var eventTapRunning: Bool
    var keyDownPostSucceeded: Bool
    var keyUpPostSucceeded: Bool
    var pressedBindingIDs: [String]
    var releasedBindingIDs: [String]
    var startedRecording: Bool
    var stoppedRecording: Bool
    var recordingSampleCount: Int?
    var recordingSampleRate: Double?
    var recordingDurationSeconds: TimeInterval?
    var recordingMaxLevel: Float?
    var levelObservationCount: Int?
    var recordingHasAudibleSignal: Bool?
    var recordingOutputPath: String?
    var recordingByteCount: Int64?
    var transcription: NativeRecordTranscriptionSmokeOutput? = nil
    var errorMessage: String?
}

private enum NativeGlobalShortcutRecordingSmokeError: LocalizedError {
    case invalidBinding(String)
    case eventCreationFailed
    case permissionMissing(accessibilityTrusted: Bool, microphone: PermissionSnapshot.Microphone)
    case missingRecording
    case silentRecording
    case unsupportedModel(String)

    var errorDescription: String? {
        switch self {
        case let .invalidBinding(binding):
            "Invalid global shortcut recording smoke binding: \(binding)."
        case .eventCreationFailed:
            "Unable to create a keyboard event for the global shortcut recording smoke."
        case let .permissionMissing(accessibilityTrusted, microphone):
            "Global shortcut recording smoke requires Accessibility trust and microphone permission; accessibilityTrusted=\(accessibilityTrusted), microphone=\(microphone.rawValue)."
        case .missingRecording:
            "The global shortcut recording smoke did not produce a recording."
        case .silentRecording:
            "The global shortcut record/transcribe smoke did not detect an audible signal."
        case let .unsupportedModel(modelID):
            "Local transcription model '\(modelID)' is not available in the native Swift app."
        }
    }
}

private struct NativeGlobalShortcutRecordingHistoryResult {
    var entryID: Int64
    var recordingFileName: String
}

@MainActor
private final class NativeGlobalShortcutRecordingSmokeController {
    private let request: NativeGlobalShortcutRecordingSmokeRequest
    private let audioCaptureService = AudioCaptureService()
    private let stats = LockedShortcutRecordingSmokeStats()
    private var coordinator = RecordingCoordinator()
    private var activeRecordingShortcutID: String?
    private var activeRecordingPostProcessRequested = false
    private var pressedBindingIDs: [String] = []
    private var releasedBindingIDs: [String] = []
    private var recording: AudioRecording?
    private var errorMessage: String?
    private var recordingOutputPath: String?
    private var didStartRecording = false
    private var didStopRecording = false

    init(request: NativeGlobalShortcutRecordingSmokeRequest) {
        self.request = request
    }

    func handlePressed(_ bindingID: String) {
        pressedBindingIDs.append(bindingID)
        perform(
            GlobalShortcutActionRouter.action(
                for: .pressed,
                bindingID: bindingID,
                context: actionContext
            )
        )
    }

    func handleReleased(_ bindingID: String) {
        releasedBindingIDs.append(bindingID)
        perform(
            GlobalShortcutActionRouter.action(
                for: .released,
                bindingID: bindingID,
                context: actionContext
            )
        )
    }

    func waitUntilRecordingStarts(timeoutMilliseconds: Int) async -> Bool {
        let deadline = Date().addingTimeInterval(TimeInterval(timeoutMilliseconds) / 1_000)
        while Date() < deadline {
            if didStartRecording {
                return true
            }
            if errorMessage != nil {
                return false
            }
            try? await Task.sleep(for: .milliseconds(50))
        }
        return didStartRecording
    }

    func waitUntilRecordingStops(timeoutMilliseconds: Int) async -> Bool {
        let deadline = Date().addingTimeInterval(TimeInterval(timeoutMilliseconds) / 1_000)
        while Date() < deadline {
            if didStopRecording {
                return true
            }
            if errorMessage != nil {
                return false
            }
            try? await Task.sleep(for: .milliseconds(50))
        }
        return didStopRecording
    }

    func cancelIfNeeded() {
        guard coordinator.state.isRecording else {
            return
        }
        audioCaptureService.cancel()
        coordinator.cancel()
        activeRecordingShortcutID = nil
        activeRecordingPostProcessRequested = false
    }

    func snapshot() -> NativeShortcutRecordingSmokeSnapshot {
        let levelSnapshot = stats.snapshot()
        return NativeShortcutRecordingSmokeSnapshot(
            pressedBindingIDs: pressedBindingIDs,
            releasedBindingIDs: releasedBindingIDs,
            recording: recording,
            maxLevel: levelSnapshot.maxLevel,
            levelObservationCount: levelSnapshot.observationCount,
            errorMessage: errorMessage,
            recordingOutputPath: recordingOutputPath
        )
    }

    private var actionContext: GlobalShortcutActionContext {
        GlobalShortcutActionContext(
            pushToTalk: true,
            recordingState: coordinator.state,
            activeRecordingShortcutID: activeRecordingShortcutID
        )
    }

    private func perform(_ shortcutAction: GlobalShortcutAction) {
        switch shortcutAction {
        case .none:
            return
        case let .startRecording(postProcessRequested, shortcutID):
            startRecording(postProcessRequested: postProcessRequested, shortcutID: shortcutID)
        case .stopRecording:
            stopRecording()
        case .cancelRecording:
            cancelIfNeeded()
        }
    }

    private func startRecording(postProcessRequested: Bool, shortcutID: String) {
        guard coordinator.start() else {
            return
        }

        do {
            activeRecordingShortcutID = shortcutID
            activeRecordingPostProcessRequested = postProcessRequested
            try audioCaptureService.start(selectedMicrophoneName: request.microphoneName) { [stats] level in
                stats.observe(level)
            }
            didStartRecording = true
        } catch {
            errorMessage = error.localizedDescription
            coordinator.cancel()
            activeRecordingShortcutID = nil
            activeRecordingPostProcessRequested = false
        }
    }

    private func stopRecording() {
        guard coordinator.stop() else {
            return
        }

        do {
            recording = try audioCaptureService.stop()
            coordinator.finishProcessing()
            activeRecordingShortcutID = nil
            activeRecordingPostProcessRequested = false
            didStopRecording = true
        } catch {
            errorMessage = error.localizedDescription
            coordinator.cancel()
            activeRecordingShortcutID = nil
            activeRecordingPostProcessRequested = false
        }
    }
}

private struct NativeShortcutRecordingSmokeSnapshot {
    var pressedBindingIDs: [String]
    var releasedBindingIDs: [String]
    var recording: AudioRecording?
    var maxLevel: Float
    var levelObservationCount: Int
    var errorMessage: String?
    var recordingOutputPath: String?
}

private final class LockedShortcutRecordingSmokeStats: @unchecked Sendable {
    private let lock = NSLock()
    private var maxObservedLevel: Float = 0
    private var observations = 0

    func observe(_ level: Float) {
        lock.withLock {
            maxObservedLevel = max(maxObservedLevel, level)
            observations += 1
        }
    }

    func snapshot() -> (maxLevel: Float, observationCount: Int) {
        lock.withLock {
            (maxObservedLevel, observations)
        }
    }
}
