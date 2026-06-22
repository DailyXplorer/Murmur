import Foundation

struct LocalModelStorageState: Equatable {
    var modelID: String
    var isDownloaded: Bool
    var byteCount: Int64
    var directories: [URL]

    var sizeLabel: String? {
        guard isDownloaded else {
            return nil
        }
        return ByteCountFormatter.string(fromByteCount: byteCount, countStyle: .file)
    }
}

enum LocalModelStorageService {
    static func states(paths: AppPaths, models: [LocalTranscriptionModel] = LocalTranscriptionModel.catalog) -> [String: LocalModelStorageState] {
        Dictionary(uniqueKeysWithValues: models.map { model in
            (model.id, state(for: model, paths: paths))
        })
    }

    static func state(for model: LocalTranscriptionModel, paths: AppPaths) -> LocalModelStorageState {
        let directories = modelDirectories(for: model, paths: paths)
        let completeDirectories = directories.filter(isCompleteWhisperKitModelDirectory)
        let byteCount = directories.reduce(Int64(0)) { partial, directory in
            partial + directoryByteCount(directory)
        }

        return LocalModelStorageState(
            modelID: model.id,
            isDownloaded: !completeDirectories.isEmpty,
            byteCount: byteCount,
            directories: directories
        )
    }

    static func downloadedModelDirectory(for model: LocalTranscriptionModel, paths: AppPaths) -> URL? {
        modelDirectories(for: model, paths: paths)
            .first(where: isCompleteWhisperKitModelDirectory)
    }

    static func delete(model: LocalTranscriptionModel, paths: AppPaths) throws {
        for directory in modelDirectories(for: model, paths: paths) {
            try FileManager.default.removeItem(at: directory)
        }
    }

    static func cacheDirectory(paths: AppPaths) -> URL {
        paths.modelsDirectory.appendingPathComponent("whisperkit-coreml", isDirectory: true)
    }

    private static func modelRepositoryDirectory(paths: AppPaths) -> URL {
        cacheDirectory(paths: paths)
            .appendingPathComponent("models", isDirectory: true)
            .appendingPathComponent("argmaxinc", isDirectory: true)
            .appendingPathComponent("whisperkit-coreml", isDirectory: true)
    }

    private static func modelDirectories(for model: LocalTranscriptionModel, paths: AppPaths) -> [URL] {
        let repositoryDirectory = modelRepositoryDirectory(paths: paths)
        guard FileManager.default.fileExists(atPath: repositoryDirectory.path) else {
            return []
        }

        let hints = model.cacheNameHints
        let children = (try? FileManager.default.contentsOfDirectory(
            at: repositoryDirectory,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        )) ?? []

        return children
            .filter { url in
                (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true
            }
            .filter { url in
                let name = url.lastPathComponent
                return hints.contains { name.contains($0) }
            }
    }

    private static func isCompleteWhisperKitModelDirectory(_ directory: URL) -> Bool {
        let fileManager = FileManager.default
        guard fileManager.fileExists(atPath: directory.appendingPathComponent("config.json").path),
              fileManager.fileExists(atPath: directory.appendingPathComponent("generation_config.json").path)
        else {
            return false
        }

        return ["AudioEncoder.mlmodelc", "MelSpectrogram.mlmodelc", "TextDecoder.mlmodelc"].allSatisfy { component in
            isCompleteCompiledModelComponent(directory.appendingPathComponent(component, isDirectory: true))
        }
    }

    private static func isCompleteCompiledModelComponent(_ directory: URL) -> Bool {
        let fileManager = FileManager.default
        return fileManager.fileExists(atPath: directory.appendingPathComponent("metadata.json").path) &&
            fileManager.fileExists(atPath: directory.appendingPathComponent("model.mil").path) &&
            fileManager.fileExists(atPath: directory.appendingPathComponent("coremldata.bin").path) &&
            fileManager.fileExists(atPath: directory.appendingPathComponent("weights/weight.bin").path)
    }

    private static func directoryByteCount(_ url: URL) -> Int64 {
        guard let enumerator = FileManager.default.enumerator(
            at: url,
            includingPropertiesForKeys: [.totalFileAllocatedSizeKey, .fileAllocatedSizeKey, .isRegularFileKey],
            options: [.skipsHiddenFiles]
        ) else {
            return 0
        }

        return enumerator.reduce(Int64(0)) { partial, element in
            guard let fileURL = element as? URL,
                  let values = try? fileURL.resourceValues(forKeys: [.totalFileAllocatedSizeKey, .fileAllocatedSizeKey, .isRegularFileKey]),
                  values.isRegularFile == true
            else {
                return partial
            }

            return partial + Int64(values.totalFileAllocatedSize ?? values.fileAllocatedSize ?? 0)
        }
    }
}
