import AppKit
import Foundation

enum NativeRecordTranscriptionSmokeRunner {
    @MainActor
    static func runSynchronouslyAndExit(_ request: NativeRecordTranscriptionSmokeRequest) -> Never {
        let preparedRecording: PreparedNativeRecordTranscriptionSmoke
        do {
            preparedRecording = try record(request)
        } catch {
            FileHandle.standardError.writeLine(error.localizedDescription)
            exit(1)
        }

        if let pasteRequest = request.pasteRequest {
            switch transcribeSynchronously(request, preparedRecording: preparedRecording) {
            case let .success(output):
                runWithApplicationSynchronouslyAndExit(
                    request,
                    transcriptionOutput: output,
                    pasteRequest: pasteRequest
                )
            case let .failure(error):
                FileHandle.standardError.writeLine(error.localizedDescription)
                exit(1)
            }
        }

        let semaphore = DispatchSemaphore(value: 0)
        let exitCode = LockedRecordTranscriptionExitCode()

        Task.detached {
            do {
                let output = try await transcribe(request, preparedRecording: preparedRecording)
                let outputString = try outputString(for: output)
                if let outputPath = request.outputJSONPath {
                    try write(outputString, to: outputPath)
                }
                FileHandle.standardOutput.writeLine(outputString)
                exitCode.set(0)
            } catch {
                FileHandle.standardError.writeLine(error.localizedDescription)
                exitCode.set(1)
            }
            semaphore.signal()
        }

        semaphore.wait()
        exit(Int32(exitCode.value))
    }

    @MainActor
    private static func runWithApplicationSynchronouslyAndExit(
        _ request: NativeRecordTranscriptionSmokeRequest,
        transcriptionOutput: NativeRecordTranscriptionSmokeOutput,
        pasteRequest: NativeTranscriptionPasteSmokeRequest
    ) -> Never {
        let application = NSApplication.shared
        application.setActivationPolicy(.regular)
        application.finishLaunching()

        Task { @MainActor in
            do {
                var output = transcriptionOutput
                if pasteRequest.externalRoundTrip {
                    let externalPasteOutput = try await NativeExternalPasteRoundTripSmokeRunner.output(
                        for: pasteRequest.externalRoundTripSmokeRequest(text: transcriptionOutput.outputText)
                    )
                    output.paste = externalPasteOutput.paste
                    output.externalPaste = externalPasteOutput
                } else {
                    output.paste = try await NativePasteSmokeRunner.output(
                        for: pasteRequest.pasteSmokeRequest(text: transcriptionOutput.outputText)
                    )
                }
                let outputString = try outputString(for: output)
                if let outputPath = request.outputJSONPath {
                    try write(outputString, to: outputPath)
                }
                FileHandle.standardOutput.writeLine(outputString)
                exit(0)
            } catch {
                FileHandle.standardError.writeLine(error.localizedDescription)
                exit(1)
            }
        }

        application.run()
        exit(1)
    }

    @MainActor
    private static func record(_ request: NativeRecordTranscriptionSmokeRequest) throws -> PreparedNativeRecordTranscriptionSmoke {
        let permission = PermissionService().snapshot().microphone
        guard permission == .granted else {
            throw NativeRecordTranscriptionSmokeError.microphonePermission(permission)
        }

        let outputURL = URL(fileURLWithPath: request.outputPath)
        let outputDirectory = outputURL.deletingLastPathComponent()
        try FileManager.default.createDirectory(
            at: outputDirectory,
            withIntermediateDirectories: true
        )

        let stats = LockedRecordTranscriptionStats()
        let audioCaptureService = AudioCaptureService()
        try audioCaptureService.start(selectedMicrophoneName: request.microphoneName) { level in
            stats.observe(level)
        }
        Thread.sleep(forTimeInterval: TimeInterval(request.durationMilliseconds) / 1_000)
        let capturedRecording = try audioCaptureService.stop()

        guard capturedRecording.hasAudibleSignal else {
            throw NativeRecordTranscriptionSmokeError.silentRecording
        }

        let recording = capturedRecording.preparedForTranscriptionInput()
        guard recording.isEmpty == false else {
            throw NativeRecordTranscriptionSmokeError.silentRecording
        }
        try WAVFileWriter.write(recording, to: outputURL)

        let attributes = try FileManager.default.attributesOfItem(atPath: outputURL.path)
        let byteCount = (attributes[.size] as? NSNumber)?.int64Value ?? 0
        let observedStats = stats.snapshot()

        return PreparedNativeRecordTranscriptionSmoke(
            fileURL: outputURL,
            capturedSampleCount: capturedRecording.samples.count,
            processedSampleCount: recording.samples.count,
            sampleRate: recording.sampleRate,
            durationSeconds: recording.duration,
            maxLevel: observedStats.maxLevel,
            levelObservationCount: observedStats.observationCount,
            byteCount: byteCount
        )
    }

    private static func transcribe(
        _ request: NativeRecordTranscriptionSmokeRequest,
        preparedRecording: PreparedNativeRecordTranscriptionSmoke
    ) async throws -> NativeRecordTranscriptionSmokeOutput {
        let paths = try AppPaths.resolve()
        let settingsConfiguration = try await settingsConfiguration(for: request, paths: paths)

        let result = try await AudioFileTranscriptionPipeline.transcribe(
            fileURL: preparedRecording.fileURL,
            settings: settingsConfiguration.settings,
            paths: paths,
            credentialStore: settingsConfiguration.credentialStore,
            appleSpeechTranscriptionService: settingsConfiguration.appleSpeechTranscriptionService,
            postProcessRequested: request.postProcessRequested
        )

        let historyResult: NativeRecordTranscriptionHistoryResult?
        if request.recordHistory {
            historyResult = try saveHistory(
                fileURL: preparedRecording.fileURL,
                result: result,
                postProcessRequested: request.postProcessRequested,
                paths: paths
            )
        } else {
            historyResult = nil
        }

        return NativeRecordTranscriptionSmokeOutput(
            outputPath: preparedRecording.fileURL.path,
            requestedDurationMilliseconds: request.durationMilliseconds,
            capturedSampleCount: preparedRecording.capturedSampleCount,
            processedSampleCount: preparedRecording.processedSampleCount,
            sampleRate: preparedRecording.sampleRate,
            durationSeconds: preparedRecording.durationSeconds,
            maxLevel: preparedRecording.maxLevel,
            levelObservationCount: preparedRecording.levelObservationCount,
            byteCount: preparedRecording.byteCount,
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
    }

    private static func settingsConfiguration(
        for request: NativeRecordTranscriptionSmokeRequest,
        paths: AppPaths
    ) async throws -> NativeSmokeTranscriptionSettingsConfiguration {
        try await NativeSmokeTranscriptionSettingsResolver.configuration(
            modelID: request.modelID,
            language: request.language,
            useSelectedSettings: request.useSelectedSettings,
            paths: paths
        )
    }

    private static func outputString(for output: NativeRecordTranscriptionSmokeOutput) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }

    private static func transcribeSynchronously(
        _ request: NativeRecordTranscriptionSmokeRequest,
        preparedRecording: PreparedNativeRecordTranscriptionSmoke
    ) -> Result<NativeRecordTranscriptionSmokeOutput, Error> {
        let semaphore = DispatchSemaphore(value: 0)
        let resultBox = LockedRecordTranscriptionSmokeOutput()

        Task.detached {
            do {
                resultBox.set(.success(try await transcribe(request, preparedRecording: preparedRecording)))
            } catch {
                resultBox.set(.failure(error))
            }
            semaphore.signal()
        }

        semaphore.wait()
        return resultBox.value ?? .failure(NativeRecordTranscriptionSmokeError.missingSynchronousResult)
    }

    private static func write(_ output: String, to path: String) throws {
        let outputURL = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try output.write(to: outputURL, atomically: true, encoding: .utf8)
    }

    private static func saveHistory(
        fileURL: URL,
        result: ProcessedAudioTranscription,
        postProcessRequested: Bool,
        paths: AppPaths
    ) throws -> NativeRecordTranscriptionHistoryResult {
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
        return NativeRecordTranscriptionHistoryResult(
            entryID: updatedEntry.id,
            recordingFileName: updatedEntry.fileName
        )
    }
}

private struct PreparedNativeRecordTranscriptionSmoke {
    var fileURL: URL
    var capturedSampleCount: Int
    var processedSampleCount: Int
    var sampleRate: Double
    var durationSeconds: TimeInterval
    var maxLevel: Float
    var levelObservationCount: Int
    var byteCount: Int64
}

private struct NativeRecordTranscriptionHistoryResult {
    var entryID: Int64
    var recordingFileName: String
}

struct NativeRecordTranscriptionSmokeOutput: Encodable {
    var outputPath: String
    var requestedDurationMilliseconds: Int
    var capturedSampleCount: Int
    var processedSampleCount: Int
    var sampleRate: Double
    var durationSeconds: TimeInterval
    var maxLevel: Float
    var levelObservationCount: Int
    var byteCount: Int64
    var microphoneName: String?
    var modelID: String
    var modelDisplayName: String? = nil
    var language: String?
    var usedSelectedSettings: Bool? = nil
    var postProcessRequested: Bool? = nil
    var transcriptionText: String
    var outputText: String
    var historyEntryID: Int64?
    var recordingFileName: String?
    var paste: NativePasteSmokeOutput?
    var externalPaste: NativeExternalPasteRoundTripSmokeOutput? = nil
}

private final class LockedRecordTranscriptionStats: @unchecked Sendable {
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

private final class LockedRecordTranscriptionExitCode: @unchecked Sendable {
    private let lock = NSLock()
    private var storage = 1

    var value: Int {
        lock.withLock { storage }
    }

    func set(_ value: Int) {
        lock.withLock {
            storage = value
        }
    }
}

private final class LockedRecordTranscriptionSmokeOutput: @unchecked Sendable {
    private let lock = NSLock()
    private var storage: Result<NativeRecordTranscriptionSmokeOutput, Error>?

    var value: Result<NativeRecordTranscriptionSmokeOutput, Error>? {
        lock.withLock { storage }
    }

    func set(_ value: Result<NativeRecordTranscriptionSmokeOutput, Error>) {
        lock.withLock {
            storage = value
        }
    }
}

private enum NativeRecordTranscriptionSmokeError: LocalizedError {
    case microphonePermission(PermissionSnapshot.Microphone)
    case silentRecording
    case unsupportedModel(String)
    case missingSynchronousResult

    var errorDescription: String? {
        switch self {
        case let .microphonePermission(status):
            "Microphone permission is \(status.rawValue); grant microphone access before running --smoke-record-transcribe."
        case .silentRecording:
            "The native record/transcribe smoke did not detect an audible signal."
        case let .unsupportedModel(modelID):
            "Local transcription model '\(modelID)' is not available in the native Swift app."
        case .missingSynchronousResult:
            "The native record/transcribe smoke did not return a result."
        }
    }
}
