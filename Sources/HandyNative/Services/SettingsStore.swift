import Foundation

struct SettingsStore {
    struct LoadResult: Equatable {
        var settings: AppSettings
        var warningMessage: String?
    }

    private let encoder: JSONEncoder
    private let decoder: JSONDecoder
    private let settingsURL: URL
    private let legacySettingsURL: URL
    private let legacyCredentialMigrationMarkerURL: URL
    private let paths: AppPaths
    private let credentialStore: any PostProcessCredentialStoring
    private let now: () -> Date

    init(
        paths: AppPaths? = try? AppPaths.resolve(),
        credentialStore: (any PostProcessCredentialStoring)? = nil,
        now: @escaping () -> Date = Date.init
    ) {
        let resolvedPaths = paths ?? AppPaths(
            appDataDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("HandyNative", isDirectory: true),
            recordingsDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("HandyNative/recordings", isDirectory: true),
            modelsDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("HandyNative/models", isDirectory: true),
            logsDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("HandyNative/logs", isDirectory: true)
        )

        encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        decoder = JSONDecoder()
        self.paths = resolvedPaths
        settingsURL = resolvedPaths.appDataDirectory.appendingPathComponent("native_settings.json")
        legacySettingsURL = resolvedPaths.appDataDirectory.appendingPathComponent("settings_store.json")
        legacyCredentialMigrationMarkerURL = resolvedPaths.appDataDirectory.appendingPathComponent(".legacy_api_credentials_imported")
        self.credentialStore = credentialStore ?? LocalPostProcessCredentialStore(paths: resolvedPaths)
        self.now = now
    }

    func load() -> AppSettings {
        loadResult().settings
    }

    func loadResult() -> LoadResult {
        migrateLegacyCredentialsIfNeeded()

        guard let data = try? Data(contentsOf: settingsURL) else {
            return LoadResult(settings: defaultSettingsForCurrentState(), warningMessage: nil)
        }

        guard var settings = try? decoder.decode(AppSettings.self, from: data) else {
            let backupMessage = backUpInvalidSettingsFile()
            return LoadResult(
                settings: defaultSettingsForCurrentState(),
                warningMessage: backupMessage
            )
        }

        if !hasNativeOnboardingCompletedValue(data) {
            settings.nativeOnboardingCompleted = true
        }
        settings.ensureTranscriptionAPIDefaults()
        settings.ensurePostProcessDefaults()
        if let normalizedData = try? encoder.encode(settings),
           normalizedData != data {
            try? normalizedData.write(to: settingsURL, options: [.atomic])
        }
        return LoadResult(settings: settings, warningMessage: nil)
    }

    func save(_ settings: AppSettings) {
        guard let data = try? encoder.encode(settings) else {
            return
        }

        try? FileManager.default.createDirectory(
            at: settingsURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try? data.write(to: settingsURL, options: [.atomic])
    }

    private func hasExistingHandyState() -> Bool {
        FileManager.default.fileExists(atPath: legacySettingsURL.path) ||
            LocalModelStorageService.states(paths: paths).values.contains { $0.isDownloaded }
    }

    private func defaultSettingsForCurrentState() -> AppSettings {
        var settings = AppSettings.defaults
        settings.nativeOnboardingCompleted = hasExistingHandyState()
        return settings
    }

    private func hasNativeOnboardingCompletedValue(_ data: Data) -> Bool {
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return false
        }
        return object.keys.contains("nativeOnboardingCompleted")
    }

    private func backUpInvalidSettingsFile() -> String {
        let backupURL = invalidSettingsBackupURL()
        do {
            try FileManager.default.moveItem(at: settingsURL, to: backupURL)
            return "Handy could not read its settings, so defaults were loaded. The invalid settings file was moved to \(backupURL.lastPathComponent)."
        } catch {
            return "Handy could not read its settings, so defaults were loaded. The invalid settings file could not be backed up: \(error.localizedDescription)"
        }
    }

    private func invalidSettingsBackupURL() -> URL {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let timestamp = formatter.string(from: now())
            .replacingOccurrences(of: ":", with: "-")
        return settingsURL
            .deletingLastPathComponent()
            .appendingPathComponent("\(settingsURL.lastPathComponent).\(timestamp).invalid")
    }

    private func migrateLegacyCredentialsIfNeeded() {
        guard FileManager.default.fileExists(atPath: legacySettingsURL.path),
              !FileManager.default.fileExists(atPath: legacyCredentialMigrationMarkerURL.path),
              let data = try? Data(contentsOf: legacySettingsURL),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return
        }

        let settingsObject = object["settings"] as? [String: Any] ?? object
        var apiKeys = legacyAPIKeys(from: settingsObject["post_process_api_keys"])
        apiKeys.merge(legacyAPIKeys(from: settingsObject["postProcessAPIKeys"])) { _, new in new }
        apiKeys.merge(legacyAPIKeys(from: settingsObject["transcription_api_api_keys"])) { _, new in new }
        apiKeys.merge(legacyAPIKeys(from: settingsObject["transcriptionAPIAPIKeys"])) { _, new in new }
        guard !apiKeys.isEmpty else {
            markLegacyCredentialsImported()
            return
        }

        do {
            try credentialStore.importAPIKeys(apiKeys)
            markLegacyCredentialsImported()
        } catch {
            return
        }
    }

    private func legacyAPIKeys(from value: Any?) -> [String: String] {
        guard let rawKeys = value as? [String: Any] else {
            return [:]
        }

        return rawKeys.reduce(into: [String: String]()) { result, element in
            guard let apiKey = element.value as? String else {
                return
            }
            let trimmedProviderID = element.key.trimmingCharacters(in: .whitespacesAndNewlines)
            let trimmedAPIKey = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmedProviderID.isEmpty, !trimmedAPIKey.isEmpty else {
                return
            }
            result[trimmedProviderID] = trimmedAPIKey
        }
    }

    private func markLegacyCredentialsImported() {
        try? "imported\n".write(to: legacyCredentialMigrationMarkerURL, atomically: true, encoding: .utf8)
    }
}
