import AppKit
import Foundation

enum NativeTranscriptionSmokeRunner {
    @MainActor
    static func runSynchronouslyAndExit(_ request: NativeTranscriptionSmokeRequest) -> Never {
        if let pasteRequest = request.pasteRequest {
            switch transcribeSynchronously(request) {
            case let .success(result):
                runWithApplicationSynchronouslyAndExit(
                    request,
                    transcriptionResult: result,
                    pasteRequest: pasteRequest
                )
            case let .failure(error):
                writeFailureIfRequested(error, for: request)
                FileHandle.standardError.writeLine(error.localizedDescription)
                exit(1)
            }
        }

        let semaphore = DispatchSemaphore(value: 0)
        let exitCode = LockedExitCode()

        Task.detached {
            do {
                let output = try await transcribe(request)
                if let outputPath = request.outputPath {
                    try write(output, to: outputPath)
                }
                FileHandle.standardOutput.writeLine(output)
                exitCode.set(0)
            } catch {
                writeFailureIfRequested(error, for: request)
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
        _ request: NativeTranscriptionSmokeRequest,
        transcriptionResult: NativeTranscriptionSmokeResult,
        pasteRequest: NativeTranscriptionPasteSmokeRequest
    ) -> Never {
        let application = NSApplication.shared
        application.setActivationPolicy(.regular)
        application.finishLaunching()

        Task { @MainActor in
            do {
                let pasteOutput: NativePasteSmokeOutput?
                let externalPasteOutput: NativeExternalPasteRoundTripSmokeOutput?
                if pasteRequest.externalRoundTrip {
                    let output = try await NativeExternalPasteRoundTripSmokeRunner.output(
                        for: pasteRequest.externalRoundTripSmokeRequest(text: transcriptionResult.outputText)
                    )
                    pasteOutput = output.paste
                    externalPasteOutput = output
                } else {
                    pasteOutput = try await NativePasteSmokeRunner.output(
                        for: pasteRequest.pasteSmokeRequest(text: transcriptionResult.outputText)
                    )
                    externalPasteOutput = nil
                }
                let output = try outputString(
                    for: transcriptionResult,
                    pasteOutput: pasteOutput,
                    externalPasteOutput: externalPasteOutput
                )
                if let outputPath = request.outputPath {
                    try write(output, to: outputPath)
                }
                FileHandle.standardOutput.writeLine(output)
                exit(0)
            } catch {
                writeFailureIfRequested(error, for: request)
                FileHandle.standardError.writeLine(error.localizedDescription)
                exit(1)
            }
        }

        application.run()
        exit(1)
    }

    private static func transcribe(_ request: NativeTranscriptionSmokeRequest) async throws -> String {
        let result = try await transcriptionResult(for: request)

        guard request.recordHistory ||
            request.pasteRequest != nil ||
            request.useSelectedSettings ||
            request.postProcessRequested
        else {
            return result.outputText
        }

        return try outputString(for: result, pasteOutput: nil)
    }

    private static func outputString(
        for result: NativeTranscriptionSmokeResult,
        pasteOutput: NativePasteSmokeOutput?,
        externalPasteOutput: NativeExternalPasteRoundTripSmokeOutput? = nil
    ) throws -> String {
        let output = NativeTranscriptionSmokeOutput(
            modelID: result.modelID,
            modelDisplayName: result.modelDisplayName,
            language: result.language,
            usedSelectedSettings: result.usedSelectedSettings,
            postProcessRequested: result.postProcessRequested,
            transcriptionText: result.transcriptionText,
            outputText: result.outputText,
            historyEntryID: result.historyEntryID,
            recordingFileName: result.recordingFileName,
            paste: pasteOutput,
            externalPaste: externalPasteOutput
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? result.outputText
    }

    private static func transcribeSynchronously(
        _ request: NativeTranscriptionSmokeRequest
    ) -> Result<NativeTranscriptionSmokeResult, Error> {
        let semaphore = DispatchSemaphore(value: 0)
        let resultBox = LockedTranscriptionSmokeResult()

        Task.detached {
            do {
                resultBox.set(.success(try await transcriptionResult(for: request)))
            } catch {
                resultBox.set(.failure(error))
            }
            semaphore.signal()
        }

        semaphore.wait()
        return resultBox.value ?? .failure(NativeTranscriptionSmokeError.missingSynchronousResult)
    }

    private static func transcriptionResult(for request: NativeTranscriptionSmokeRequest) async throws -> NativeTranscriptionSmokeResult {
        let fileURL = URL(fileURLWithPath: request.filePath)
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: fileURL.path, isDirectory: &isDirectory),
              !isDirectory.boolValue
        else {
            throw NativeTranscriptionSmokeError.audioFileMissing(fileURL.path)
        }

        let paths = try AppPaths.resolve()
        let settingsConfiguration = try await settingsConfiguration(for: request, paths: paths)

        let result = try await AudioFileTranscriptionPipeline.transcribe(
            fileURL: fileURL,
            settings: settingsConfiguration.settings,
            paths: paths,
            credentialStore: settingsConfiguration.credentialStore,
            appleSpeechTranscriptionService: settingsConfiguration.appleSpeechTranscriptionService,
            postProcessRequested: request.postProcessRequested
        )

        if request.recordHistory {
            let historyStore = try HistoryStore(paths: paths)
            let historyFileName = try copyAudioIntoRecordings(sourceURL: fileURL, historyStore: historyStore)
            let entry = try historyStore.saveEntry(
                fileName: historyFileName,
                transcriptionText: "",
                postProcessRequested: request.postProcessRequested
            )
            let updatedEntry = try historyStore.updateTranscription(
                id: entry.id,
                transcriptionText: result.transcriptionText,
                postProcessedText: result.postProcessedText,
                postProcessPrompt: result.postProcessPrompt
            )

            return NativeTranscriptionSmokeResult(
                modelID: settingsConfiguration.settings.selectedModel,
                modelDisplayName: settingsConfiguration.settings.selectedTranscriptionModelDisplayName,
                language: settingsConfiguration.settings.selectedLanguage,
                usedSelectedSettings: request.useSelectedSettings,
                postProcessRequested: request.postProcessRequested,
                transcriptionText: result.transcriptionText,
                outputText: result.outputText,
                historyEntryID: updatedEntry.id,
                recordingFileName: updatedEntry.fileName
            )
        }

        return NativeTranscriptionSmokeResult(
            modelID: settingsConfiguration.settings.selectedModel,
            modelDisplayName: settingsConfiguration.settings.selectedTranscriptionModelDisplayName,
            language: settingsConfiguration.settings.selectedLanguage,
            usedSelectedSettings: request.useSelectedSettings,
            postProcessRequested: request.postProcessRequested,
            transcriptionText: result.transcriptionText,
            outputText: result.outputText,
            historyEntryID: nil,
            recordingFileName: nil
        )
    }

    private static func settingsConfiguration(
        for request: NativeTranscriptionSmokeRequest,
        paths: AppPaths
    ) async throws -> NativeSmokeTranscriptionSettingsConfiguration {
        try await NativeSmokeTranscriptionSettingsResolver.configuration(
            modelID: request.modelID,
            language: request.language,
            useSelectedSettings: request.useSelectedSettings,
            paths: paths
        )
    }

    private static func copyAudioIntoRecordings(sourceURL: URL, historyStore: HistoryStore) throws -> String {
        let fileName = RecordingFileNameFormatter.fileName(for: Date())
        let destinationURL = historyStore.audioFileURL(fileName: fileName)
        try FileManager.default.copyItem(at: sourceURL, to: destinationURL)
        return fileName
    }

    private static func write(_ output: String, to path: String) throws {
        let outputURL = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try output.write(to: outputURL, atomically: true, encoding: .utf8)
    }

    private static func writeFailureIfRequested(_ error: Error, for request: NativeTranscriptionSmokeRequest) {
        guard let outputPath = request.outputPath else {
            return
        }

        do {
            try write(errorOutputString(error, for: request), to: outputPath)
        } catch {
            FileHandle.standardError.writeLine(error.localizedDescription)
        }
    }

    private static func errorOutputString(_ error: Error, for request: NativeTranscriptionSmokeRequest) throws -> String {
        let output = NativeTranscriptionSmokeFailureOutput(
            error: error.localizedDescription,
            modelID: request.modelID,
            language: request.language,
            usedSelectedSettings: request.useSelectedSettings,
            postProcessRequested: request.postProcessRequested
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{\"error\":\"\(error.localizedDescription)\"}"
    }
}

private struct NativeTranscriptionSmokeResult {
    var modelID: String
    var modelDisplayName: String
    var language: String
    var usedSelectedSettings: Bool
    var postProcessRequested: Bool
    var transcriptionText: String
    var outputText: String
    var historyEntryID: Int64?
    var recordingFileName: String?
}

struct NativeTranscriptionSmokeOutput: Encodable {
    var modelID: String? = nil
    var modelDisplayName: String? = nil
    var language: String? = nil
    var usedSelectedSettings: Bool? = nil
    var postProcessRequested: Bool? = nil
    var transcriptionText: String
    var outputText: String
    var historyEntryID: Int64?
    var recordingFileName: String?
    var paste: NativePasteSmokeOutput?
    var externalPaste: NativeExternalPasteRoundTripSmokeOutput? = nil
}

private struct NativeTranscriptionSmokeFailureOutput: Encodable {
    var error: String
    var modelID: String
    var language: String?
    var usedSelectedSettings: Bool
    var postProcessRequested: Bool
}

extension NativeTranscriptionPasteSmokeRequest {
    func pasteSmokeRequest(text: String) -> NativePasteSmokeRequest {
        NativePasteSmokeRequest(
            text: text,
            pasteMethod: pasteMethod,
            clipboardHandling: clipboardHandling,
            pasteDelayMilliseconds: pasteDelayMilliseconds,
            startDelayMilliseconds: startDelayMilliseconds,
            appendTrailingSpace: appendTrailingSpace,
            autoSubmitKey: autoSubmitKey,
            targetWindow: targetWindow,
            activationProcessIdentifier: nil,
            outputPath: nil
        )
    }

    func externalRoundTripSmokeRequest(text: String) -> NativeExternalPasteRoundTripSmokeRequest {
        NativeExternalPasteRoundTripSmokeRequest(
            text: text,
            pasteMethod: pasteMethod,
            clipboardHandling: clipboardHandling,
            pasteDelayMilliseconds: pasteDelayMilliseconds,
            startDelayMilliseconds: startDelayMilliseconds,
            appendTrailingSpace: appendTrailingSpace,
            autoSubmitKey: autoSubmitKey,
            durationMilliseconds: externalRoundTripDurationMilliseconds,
            outputPath: nil
        )
    }
}

private final class LockedExitCode: @unchecked Sendable {
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

private final class LockedTranscriptionSmokeResult: @unchecked Sendable {
    private let lock = NSLock()
    private var storage: Result<NativeTranscriptionSmokeResult, Error>?

    var value: Result<NativeTranscriptionSmokeResult, Error>? {
        lock.withLock { storage }
    }

    func set(_ value: Result<NativeTranscriptionSmokeResult, Error>) {
        lock.withLock {
            storage = value
        }
    }
}

private enum NativeTranscriptionSmokeError: LocalizedError {
    case audioFileMissing(String)
    case unsupportedModel(String)
    case missingSynchronousResult

    var errorDescription: String? {
        switch self {
        case let .audioFileMissing(path):
            "Audio file not found: \(path)"
        case let .unsupportedModel(modelID):
            "Local transcription model '\(modelID)' is not available in the native Swift app."
        case .missingSynchronousResult:
            "The transcription smoke did not return a result."
        }
    }
}
