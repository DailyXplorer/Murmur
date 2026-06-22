import Foundation

struct UpdateInfo: Equatable, Sendable {
    var version: String
    var notes: String?
    var releaseURL: URL
}

struct UpdateInstallationProgress: Equatable, Sendable {
    var downloadedBytes: Int64
    var totalBytes: Int64?

    var percent: Int? {
        guard let totalBytes,
              totalBytes > 0
        else {
            return nil
        }

        let percent = Double(downloadedBytes) / Double(totalBytes) * 100
        return min(100, max(0, Int(percent.rounded())))
    }
}

struct DownloadedUpdateArtifact: Equatable, Sendable {
    var artifactURL: URL
    var preparedAppBundleURL: URL?
    var installerScriptURL: URL?

    var canInstallAndRelaunch: Bool {
        preparedAppBundleURL != nil && installerScriptURL != nil
    }
}

enum UpdateCheckState: Equatable {
    case disabled
    case idle
    case checking
    case upToDate
    case available(UpdateInfo)
    case downloading(UpdateInfo, UpdateInstallationProgress?)
    case downloaded(UpdateInfo, DownloadedUpdateArtifact)
    case failed(String)

    var isChecking: Bool {
        if case .checking = self {
            return true
        }
        return false
    }

    var isDownloading: Bool {
        if case .downloading = self {
            return true
        }
        return false
    }

    var isBusy: Bool {
        isChecking || isDownloading
    }

    var availableInfo: UpdateInfo? {
        switch self {
        case let .available(info), let .downloading(info, _), let .downloaded(info, _):
            return info
        default:
            return nil
        }
    }
}
