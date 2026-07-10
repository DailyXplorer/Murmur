import Foundation
import CoreML
@preconcurrency import WhisperKit

actor WhisperKitTranscriptionService {
    private struct PipelineKey: Hashable {
        var modelID: String
        var accelerator: WhisperAcceleratorSetting
    }

    private var pipelineTasks: [PipelineKey: Task<WhisperKit, Error>] = [:]
    private var scheduledUnloadTasks: [PipelineKey: Task<Void, Never>] = [:]
    private var unloadTokens: [PipelineKey: UUID] = [:]

    func prepare(
        model: LocalTranscriptionModel,
        settings: AppSettings,
        paths: AppPaths,
        unloadTimeout: ModelUnloadTimeout? = nil
    ) async throws {
        let key = PipelineKey(modelID: model.id, accelerator: settings.whisperAccelerator)
        guard let modelDirectory = LocalModelStorageService.downloadedModelDirectory(for: model, paths: paths) else {
            throw WhisperKitTranscriptionServiceError.modelNotDownloaded(model.name)
        }
        _ = try await pipeline(for: model, settings: settings, paths: paths, modelDirectory: modelDirectory)
        if let unloadTimeout {
            scheduleUnload(for: key, timeout: unloadTimeout)
        } else {
            cancelScheduledUnload(for: key)
        }
    }

    func download(
        model: LocalTranscriptionModel,
        paths: AppPaths,
        progress: (@Sendable (Double) -> Void)? = nil
    ) async throws {
        try FileManager.default.createDirectory(
            at: whisperKitCacheDirectory(paths: paths),
            withIntermediateDirectories: true
        )
        _ = try await WhisperKit.download(
            variant: model.whisperKitModelID,
            downloadBase: whisperKitCacheDirectory(paths: paths),
            progressCallback: { downloadProgress in
                progress?(downloadProgress.fractionCompleted)
            }
        )
    }

    func unloadAll() {
        scheduledUnloadTasks.values.forEach { $0.cancel() }
        pipelineTasks.values.forEach { $0.cancel() }
        pipelineTasks.removeAll()
        scheduledUnloadTasks.removeAll()
        unloadTokens.removeAll()
    }

    func loadedModelIDs() -> Set<String> {
        Set(pipelineTasks.keys.map(\.modelID))
    }

    func transcribe(
        fileURL: URL,
        model: LocalTranscriptionModel,
        settings: AppSettings,
        paths: AppPaths
    ) async throws -> String {
        let key = PipelineKey(modelID: model.id, accelerator: settings.whisperAccelerator)
        guard let modelDirectory = LocalModelStorageService.downloadedModelDirectory(for: model, paths: paths) else {
            throw WhisperKitTranscriptionServiceError.modelNotDownloaded(model.name)
        }

        let pipeline = try await pipeline(for: model, settings: settings, paths: paths, modelDirectory: modelDirectory)
        let options = DecodingOptions(
            task: settings.translateToEnglish && model.supportsTranslation ? .translate : .transcribe,
            language: Self.whisperLanguageCode(from: settings.selectedLanguage),
            temperatureFallbackCount: 2,
            skipSpecialTokens: true,
            withoutTimestamps: true
        )

        let results = try await pipeline.transcribe(
            audioPath: fileURL.path,
            decodeOptions: options
        )
        let text = results
            .map(\.text)
            .joined(separator: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)

        guard text.isEmpty == false else {
            throw WhisperKitTranscriptionServiceError.emptyResult
        }

        scheduleUnload(for: key, timeout: settings.modelUnloadTimeout)
        return text
    }

    private func pipeline(
        for model: LocalTranscriptionModel,
        settings: AppSettings,
        paths: AppPaths,
        modelDirectory: URL
    ) async throws -> WhisperKit {
        let key = PipelineKey(modelID: model.id, accelerator: settings.whisperAccelerator)
        if let existing = pipelineTasks[key] {
            return try await existing.value
        }

        try FileManager.default.createDirectory(
            at: whisperKitCacheDirectory(paths: paths),
            withIntermediateDirectories: true
        )

        let config = WhisperKitConfig(
            model: model.whisperKitModelID,
            downloadBase: whisperKitCacheDirectory(paths: paths),
            modelFolder: modelDirectory.path,
            tokenizerFolder: whisperKitCacheDirectory(paths: paths),
            computeOptions: settings.whisperAccelerator.modelComputeOptions,
            verbose: false,
            prewarm: true,
            download: false
        )
        let task = Task { try await WhisperKit(config) }
        pipelineTasks[key] = task
        do {
            return try await task.value
        } catch {
            pipelineTasks[key] = nil
            throw error
        }
    }

    private func cancelScheduledUnload(for key: PipelineKey) {
        scheduledUnloadTasks[key]?.cancel()
        scheduledUnloadTasks[key] = nil
        unloadTokens[key] = nil
    }

    private func scheduleUnload(for key: PipelineKey, timeout: ModelUnloadTimeout) {
        scheduledUnloadTasks[key]?.cancel()

        guard let delaySeconds = timeout.unloadDelaySeconds else {
            scheduledUnloadTasks[key] = nil
            unloadTokens[key] = nil
            return
        }

        guard delaySeconds > 0 else {
            pipelineTasks[key]?.cancel()
            pipelineTasks[key] = nil
            scheduledUnloadTasks[key] = nil
            unloadTokens[key] = nil
            return
        }

        let token = UUID()
        unloadTokens[key] = token
        scheduledUnloadTasks[key] = Task {
            try? await Task.sleep(for: .seconds(delaySeconds))
            if Task.isCancelled == false {
                self.unloadIfCurrent(key: key, token: token)
            }
        }
    }

    private func unloadIfCurrent(key: PipelineKey, token: UUID) {
        guard unloadTokens[key] == token else {
            return
        }

        pipelineTasks[key]?.cancel()
        pipelineTasks[key] = nil
        scheduledUnloadTasks[key] = nil
        unloadTokens[key] = nil
    }

    private func whisperKitCacheDirectory(paths: AppPaths) -> URL {
        paths.modelsDirectory.appendingPathComponent("whisperkit-coreml", isDirectory: true)
    }

    private static func whisperLanguageCode(from selectedLanguage: String) -> String? {
        let trimmed = selectedLanguage.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.isEmpty == false, trimmed != "auto" else {
            return nil
        }

        switch trimmed {
        case "zh-Hans", "zh-Hant":
            return "zh"
        default:
            return trimmed.split(separator: "-").first.map(String.init) ?? trimmed
        }
    }
}

private extension WhisperAcceleratorSetting {
    var modelComputeOptions: ModelComputeOptions {
        switch self {
        case .auto:
            ModelComputeOptions()
        case .cpu:
            ModelComputeOptions(
                melCompute: .cpuOnly,
                audioEncoderCompute: .cpuOnly,
                textDecoderCompute: .cpuOnly
            )
        case .gpu:
            ModelComputeOptions(
                melCompute: .cpuAndGPU,
                audioEncoderCompute: .cpuAndGPU,
                textDecoderCompute: .cpuAndGPU
            )
        }
    }
}

enum WhisperKitTranscriptionServiceError: LocalizedError {
    case emptyResult
    case modelNotDownloaded(String)

    var errorDescription: String? {
        switch self {
        case .emptyResult:
            "WhisperKit returned an empty transcription."
        case let .modelNotDownloaded(modelName):
            "\(modelName) is not downloaded yet. Download it from Models before transcribing."
        }
    }
}
