import Foundation

enum NativeModelRuntimeSmokeRunner {
    static func runSynchronouslyAndExit(_ request: NativeModelRuntimeSmokeRequest) -> Never {
        let semaphore = DispatchSemaphore(value: 0)
        let exitCode = LockedModelRuntimeExitCode()

        Task.detached {
            do {
                let output = try await run(request)
                try write(output: output, to: request.outputPath)
                if request.outputPath == nil {
                    FileHandle.standardOutput.writeLine(try outputString(for: output))
                }
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

    private static func run(_ request: NativeModelRuntimeSmokeRequest) async throws -> NativeModelRuntimeSmokeOutput {
        guard let model = LocalTranscriptionModel.model(for: request.modelID) else {
            throw NativeModelRuntimeSmokeError.unsupportedModel(request.modelID)
        }

        let paths = try AppPaths.resolve()
        let storageBefore = LocalModelStorageService.state(for: model, paths: paths)
        var settings = AppSettings.defaults
        settings.modelUnloadTimeout = request.unloadTimeout

        let service = WhisperKitTranscriptionService()
        let loadedBefore = await sortedLoadedModelIDs(from: service)
        try await service.prepare(
            model: model,
            settings: settings,
            paths: paths,
            unloadTimeout: request.unloadTimeout
        )
        let loadedAfterPrepare = await sortedLoadedModelIDs(from: service)

        if request.waitMilliseconds > 0 {
            try await Task.sleep(for: .milliseconds(request.waitMilliseconds))
        }
        let loadedAfterWait = await sortedLoadedModelIDs(from: service)

        let loadedAfterExplicitUnload: [String]?
        if request.explicitUnload {
            await service.unloadAll()
            loadedAfterExplicitUnload = await sortedLoadedModelIDs(from: service)
        } else {
            loadedAfterExplicitUnload = nil
        }

        let storageAfter = LocalModelStorageService.state(for: model, paths: paths)
        return NativeModelRuntimeSmokeOutput(
            modelID: model.id,
            modelName: model.name,
            unloadTimeout: request.unloadTimeout.rawValue,
            unloadDelaySeconds: request.unloadTimeout.unloadDelaySeconds.map(Int.init),
            waitMilliseconds: request.waitMilliseconds,
            explicitUnload: request.explicitUnload,
            wasDownloaded: storageBefore.isDownloaded,
            isDownloaded: storageAfter.isDownloaded,
            byteCountBefore: storageBefore.byteCount,
            byteCountAfter: storageAfter.byteCount,
            loadedBefore: loadedBefore.contains(model.id),
            loadedAfterPrepare: loadedAfterPrepare.contains(model.id),
            loadedAfterWait: loadedAfterWait.contains(model.id),
            loadedAfterExplicitUnload: loadedAfterExplicitUnload.map { $0.contains(model.id) },
            loadedModelIDsBefore: loadedBefore,
            loadedModelIDsAfterPrepare: loadedAfterPrepare,
            loadedModelIDsAfterWait: loadedAfterWait,
            loadedModelIDsAfterExplicitUnload: loadedAfterExplicitUnload
        )
    }

    private static func sortedLoadedModelIDs(from service: WhisperKitTranscriptionService) async -> [String] {
        (await service.loadedModelIDs()).sorted()
    }

    private static func write(output: NativeModelRuntimeSmokeOutput, to path: String?) throws {
        guard let path else {
            return
        }

        let outputURL = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try outputString(for: output).write(to: outputURL, atomically: true, encoding: .utf8)
    }

    private static func outputString(for output: NativeModelRuntimeSmokeOutput) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

struct NativeModelRuntimeSmokeOutput: Encodable {
    var modelID: String
    var modelName: String
    var unloadTimeout: String
    var unloadDelaySeconds: Int?
    var waitMilliseconds: Int
    var explicitUnload: Bool
    var wasDownloaded: Bool
    var isDownloaded: Bool
    var byteCountBefore: Int64
    var byteCountAfter: Int64
    var loadedBefore: Bool
    var loadedAfterPrepare: Bool
    var loadedAfterWait: Bool
    var loadedAfterExplicitUnload: Bool?
    var loadedModelIDsBefore: [String]
    var loadedModelIDsAfterPrepare: [String]
    var loadedModelIDsAfterWait: [String]
    var loadedModelIDsAfterExplicitUnload: [String]?
}

private final class LockedModelRuntimeExitCode: @unchecked Sendable {
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

private enum NativeModelRuntimeSmokeError: LocalizedError {
    case unsupportedModel(String)

    var errorDescription: String? {
        switch self {
        case let .unsupportedModel(modelID):
            "Local transcription model '\(modelID)' is not available in the native Swift app."
        }
    }
}
