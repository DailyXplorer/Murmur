import Foundation

enum NativeModelCacheSmokeRunner {
    static func runSynchronouslyAndExit(_ request: NativeModelCacheSmokeRequest) -> Never {
        let semaphore = DispatchSemaphore(value: 0)
        let exitCode = LockedModelCacheExitCode()

        Task.detached {
            do {
                let output = try await run(request)
                FileHandle.standardOutput.writeLine(output)
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

    private static func run(_ request: NativeModelCacheSmokeRequest) async throws -> String {
        guard let model = LocalTranscriptionModel.model(for: request.modelID) else {
            throw NativeModelCacheSmokeError.unsupportedModel(request.modelID)
        }

        let paths = try AppPaths.resolve()
        let before = LocalModelStorageService.state(for: model, paths: paths)

        switch request.operation {
        case .status:
            break
        case .download:
            try await WhisperKitTranscriptionService().prepare(
                model: model,
                settings: .defaults,
                paths: paths
            )
        case .delete:
            try LocalModelStorageService.delete(model: model, paths: paths)
        }

        let after = LocalModelStorageService.state(for: model, paths: paths)

        let output = NativeModelCacheSmokeOutput(
            modelID: model.id,
            operation: request.operation.rawValue,
            wasDownloaded: before.isDownloaded,
            isDownloaded: after.isDownloaded,
            byteCountBefore: before.byteCount,
            byteCountAfter: after.byteCount
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

private final class LockedModelCacheExitCode: @unchecked Sendable {
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

private struct NativeModelCacheSmokeOutput: Encodable {
    var modelID: String
    var operation: String
    var wasDownloaded: Bool
    var isDownloaded: Bool
    var byteCountBefore: Int64
    var byteCountAfter: Int64
}

private enum NativeModelCacheSmokeError: LocalizedError {
    case unsupportedModel(String)

    var errorDescription: String? {
        switch self {
        case let .unsupportedModel(modelID):
            "Local transcription model '\(modelID)' is not available in the native Swift app."
        }
    }
}
