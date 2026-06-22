import Foundation

enum UpdateCheckServiceError: LocalizedError {
    case invalidManifestURL
    case httpStatus(Int)
    case missingVersion

    var errorDescription: String? {
        switch self {
        case .invalidManifestURL:
            "Invalid update manifest URL."
        case let .httpStatus(status):
            "Update check failed with status \(status)."
        case .missingVersion:
            "Update manifest did not include a version."
        }
    }
}

struct UpdateCheckResult: Equatable {
    var currentVersion: String
    var latestVersion: String
    var update: UpdateInfo?

    var isUpdateAvailable: Bool {
        update != nil
    }
}

enum UpdateCheckService {
    static let defaultManifestURL: URL? = nil
    static let defaultReleaseURL: URL? = nil
    static let fallbackVersion = "0.1.0"

    static func currentBundleVersion(bundle: Bundle = .main) -> String {
        bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? fallbackVersion
    }

    static func check(
        currentVersion: String = currentBundleVersion(),
        manifestURL: URL? = defaultManifestURL,
        urlSession: URLSession = .shared
    ) async throws -> UpdateCheckResult {
        guard let manifestURL else {
            throw UpdateCheckServiceError.invalidManifestURL
        }

        var request = URLRequest(url: manifestURL)
        request.httpMethod = "GET"
        request.cachePolicy = .reloadIgnoringLocalCacheData
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        let (data, response) = try await urlSession.data(for: request)
        if let httpResponse = response as? HTTPURLResponse,
           !(200..<300).contains(httpResponse.statusCode) {
            throw UpdateCheckServiceError.httpStatus(httpResponse.statusCode)
        }

        return try result(from: data, currentVersion: currentVersion)
    }

    static func result(from data: Data, currentVersion: String) throws -> UpdateCheckResult {
        let manifest = try JSONDecoder().decode(UpdateManifest.self, from: data)
        let latestVersion = manifest.version.trimmingCharacters(in: .whitespacesAndNewlines)
        guard latestVersion.isEmpty == false else {
            throw UpdateCheckServiceError.missingVersion
        }

        let update: UpdateInfo?
        if isVersion(latestVersion, newerThan: currentVersion) {
            update = UpdateInfo(
                version: latestVersion,
                notes: manifest.notes,
                releaseURL: manifest.bestReleaseURL ?? safeDefaultReleaseURL
            )
        } else {
            update = nil
        }

        return UpdateCheckResult(
            currentVersion: currentVersion,
            latestVersion: latestVersion,
            update: update
        )
    }

    static func isVersion(_ candidate: String, newerThan current: String) -> Bool {
        compareVersions(candidate, current) == .orderedDescending
    }

    static func compareVersions(_ lhs: String, _ rhs: String) -> ComparisonResult {
        let left = ParsedVersion(lhs)
        let right = ParsedVersion(rhs)
        let maxCount = max(left.parts.count, right.parts.count)

        for index in 0..<maxCount {
            let leftPart = index < left.parts.count ? left.parts[index] : 0
            let rightPart = index < right.parts.count ? right.parts[index] : 0
            if leftPart < rightPart {
                return .orderedAscending
            }
            if leftPart > rightPart {
                return .orderedDescending
            }
        }

        switch (left.prerelease, right.prerelease) {
        case (nil, nil):
            return .orderedSame
        case (nil, _?):
            return .orderedDescending
        case (_?, nil):
            return .orderedAscending
        case let (leftPrerelease?, rightPrerelease?):
            return leftPrerelease.localizedStandardCompare(rightPrerelease)
        }
    }
}

private var safeDefaultReleaseURL: URL {
    if let url = UpdateCheckService.defaultReleaseURL {
        return url
    }

    var components = URLComponents()
    components.scheme = "https"
    components.host = "github.com"
    components.path = "/"
    return components.url ?? URL(fileURLWithPath: "/")
}

private struct ParsedVersion {
    var parts: [Int]
    var prerelease: String?

    init(_ rawVersion: String) {
        let version = rawVersion
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingPrefix("v")
            .split(separator: "+", maxSplits: 1, omittingEmptySubsequences: false)
            .first
            .map(String.init) ?? ""

        let pieces = version.split(separator: "-", maxSplits: 1, omittingEmptySubsequences: false)
        parts = (pieces.first.map(String.init) ?? "")
            .split(separator: ".")
            .map { component in
                let digits = component.prefix { $0.isNumber }
                return Int(digits) ?? 0
            }

        if pieces.count > 1 {
            prerelease = String(pieces[1])
        }
    }
}

private struct UpdateManifest: Decodable {
    var version: String
    var notes: String?
    private var platforms: [String: Platform]?

    var bestReleaseURL: URL? {
        guard let platforms else {
            return nil
        }

        let preferredPlatforms = [
            "darwin-aarch64",
            "darwin-aarch64-app",
            "darwin-x86_64",
            "darwin-x86_64-app",
            "darwin-universal",
            "macos-aarch64",
            "macos-aarch64-app",
            "macos-x86_64",
            "macos-x86_64-app",
            "macos-universal",
        ]

        for platform in preferredPlatforms {
            if let url = platforms[platform]?.url {
                return url
            }
        }

        return platforms.values.compactMap(\.url).first
    }

    private struct Platform: Decodable {
        var url: URL?
    }
}

private extension String {
    func trimmingPrefix(_ prefix: String) -> String {
        hasPrefix(prefix) ? String(dropFirst(prefix.count)) : self
    }
}
