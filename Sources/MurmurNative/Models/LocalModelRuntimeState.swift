import Foundation

struct LocalModelRuntimeState: Equatable {
    var modelID: String
    var isLoaded: Bool
}

enum LocalModelRuntimePresentation {
    static func status(
        downloadState: LocalModelDownloadState?,
        runtimeState: LocalModelRuntimeState?,
        isActive: Bool,
        storageState: LocalModelStorageState
    ) -> String {
        if let downloadState {
            return downloadState.statusLabel
        }
        if runtimeState?.isLoaded == true {
            return "Loaded"
        }
        if isActive && storageState.isDownloaded {
            return "Active"
        }
        if storageState.isDownloaded {
            return "Downloaded"
        }
        return "Download"
    }
}
