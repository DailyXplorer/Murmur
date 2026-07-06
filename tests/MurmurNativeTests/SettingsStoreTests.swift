import Foundation
@testable import MurmurNative
import XCTest

final class SettingsStoreTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("MurmurNativeSettings-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(
            at: temporaryDirectory,
            withIntermediateDirectories: true
        )
    }

    override func tearDownWithError() throws {
        if let temporaryDirectory {
            try? FileManager.default.removeItem(at: temporaryDirectory)
        }
    }

    func testLoadReturnsDefaultsWhenNativeSettingsAreMissing() {
        let settings = SettingsStore(paths: makePaths()).load()

        XCTAssertEqual(settings, .defaults)
        XCTAssertEqual(settings.selectedModel, TranscriptionAPIProvider.mistralVoxtralModelID)
        XCTAssertTrue(settings.appleVoiceProcessingEnabled)
        XCTAssertFalse(settings.nativeOnboardingCompleted)
    }

    func testSaveAndLoadNativeSettings() {
        let paths = makePaths()
        let store = SettingsStore(paths: paths)
        var settings = AppSettings.defaults
        settings.pushToTalk = false
        settings.selectedLanguage = "fr-FR"
        settings.selectTranscriptionModel(id: "small")

        store.save(settings)
        let loaded = store.load()

        XCTAssertEqual(loaded.pushToTalk, false)
        XCTAssertEqual(loaded.selectedLanguage, "fr-FR")
        XCTAssertEqual(loaded.selectedModel, "small")
    }

    func testExistingNativeSettingsWithoutOnboardingFlagAreMigratedAsCompleted() throws {
        let paths = makePaths()
        let settingsURL = paths.appDataDirectory.appendingPathComponent("native_settings.json")
        try #"{"pushToTalk":false,"selectedLanguage":"fr-FR"}"#.write(to: settingsURL, atomically: true, encoding: .utf8)

        let settings = SettingsStore(paths: paths).load()

        XCTAssertTrue(settings.nativeOnboardingCompleted)
    }

    func testLegacySettingsMarkMissingNativeSettingsAsCompletedOnboarding() throws {
        let paths = makePaths()
        let legacySettingsURL = paths.appDataDirectory.appendingPathComponent("settings_store.json")
        try #"{"selected_model":"tiny"}"#.write(to: legacySettingsURL, atomically: true, encoding: .utf8)

        let settings = SettingsStore(paths: paths).load()

        XCTAssertTrue(settings.nativeOnboardingCompleted)
    }

    func testLoadResultBacksUpMalformedNativeSettingsAndReturnsWarning() throws {
        let paths = makePaths()
        let settingsURL = paths.appDataDirectory.appendingPathComponent("native_settings.json")
        try #"{"pushToTalk":false"#.write(to: settingsURL, atomically: true, encoding: .utf8)
        let store = SettingsStore(
            paths: paths,
            now: { Date(timeIntervalSince1970: 1_234) }
        )

        let result = store.loadResult()

        XCTAssertEqual(result.settings, .defaults)
        XCTAssertFalse(FileManager.default.fileExists(atPath: settingsURL.path))
        let backupURLs = try FileManager.default.contentsOfDirectory(
            at: paths.appDataDirectory,
            includingPropertiesForKeys: nil
        ).filter {
            $0.lastPathComponent.hasPrefix("native_settings.json.") &&
                $0.lastPathComponent.hasSuffix(".invalid")
        }
        XCTAssertEqual(backupURLs.count, 1)
        XCTAssertEqual(try String(contentsOf: backupURLs[0], encoding: .utf8), #"{"pushToTalk":false"#)
        XCTAssertTrue(result.warningMessage?.contains("could not read its settings") == true)
        XCTAssertTrue(result.warningMessage?.contains(backupURLs[0].lastPathComponent) == true)
    }

    func testLegacySettingsAPIKeysAreImportedToNativeCredentialStore() throws {
        let paths = makePaths()
        let legacySettingsURL = paths.appDataDirectory.appendingPathComponent("settings_store.json")
        try """
        {
          "settings": {
            "post_process_api_keys": {
              "mistral": "legacy-post-key"
            },
            "transcription_api_api_keys": {
              "mistral": "legacy-transcription-key"
            }
          }
        }
        """.write(to: legacySettingsURL, atomically: true, encoding: .utf8)
        let credentialStore = LocalPostProcessCredentialStore(paths: paths, storageMode: .file)

        _ = SettingsStore(paths: paths, credentialStore: credentialStore).load()

        XCTAssertEqual(
            try credentialStore.readAPIKey(providerID: TranscriptionAPIProvider.mistralProviderID),
            "legacy-transcription-key"
        )
    }

    func testLegacySettingsCredentialMigrationRetriesWhenCredentialStoreFails() throws {
        let paths = makePaths()
        let legacySettingsURL = paths.appDataDirectory.appendingPathComponent("settings_store.json")
        let markerURL = paths.appDataDirectory.appendingPathComponent(".legacy_api_credentials_imported")
        try """
        {
          "settings": {
            "post_process_api_keys": {
              "mistral": "legacy-post-key"
            }
          }
        }
        """.write(to: legacySettingsURL, atomically: true, encoding: .utf8)
        let credentialStore = FlakyCredentialStore()
        credentialStore.shouldFailSaves = true

        _ = SettingsStore(paths: paths, credentialStore: credentialStore).load()

        XCTAssertFalse(FileManager.default.fileExists(atPath: markerURL.path))
        XCTAssertNil(try credentialStore.readAPIKey(providerID: "mistral"))

        credentialStore.shouldFailSaves = false
        _ = SettingsStore(paths: paths, credentialStore: credentialStore).load()

        XCTAssertTrue(FileManager.default.fileExists(atPath: markerURL.path))
        XCTAssertEqual(try credentialStore.readAPIKey(providerID: "mistral"), "legacy-post-key")
    }

    func testNativeSettingsDecodeMissingValuesFromDefaults() throws {
        let data = #"{"pushToTalk":false,"selectedLanguage":"de-DE"}"#.data(using: .utf8)!

        let settings = try JSONDecoder().decode(AppSettings.self, from: data)

        XCTAssertFalse(settings.pushToTalk)
        XCTAssertEqual(settings.selectedLanguage, "de-DE")
        XCTAssertFalse(settings.autostartEnabled)
        XCTAssertTrue(settings.appleVoiceProcessingEnabled)
        XCTAssertEqual(settings.transcribeShortcutBinding.currentBinding, "option+space")
        XCTAssertEqual(settings.transcribeWithPostProcessShortcutBinding.currentBinding, "option+shift+space")
        XCTAssertEqual(settings.cancelShortcutBinding.currentBinding, "escape")
        XCTAssertTrue(settings.customWords.isEmpty)
        XCTAssertFalse(settings.translateToEnglish)
        XCTAssertEqual(settings.wordCorrectionThreshold, 0.18)
        XCTAssertEqual(settings.recordingRetentionPeriod, .preserveLimit)
    }

    func testNativeSettingsIgnoreRemovedUpdateCheckKey() throws {
        let data = #"{"updateChecksEnabled":true,"pushToTalk":false}"#.data(using: .utf8)!

        let settings = try JSONDecoder().decode(AppSettings.self, from: data)

        XCTAssertFalse(settings.pushToTalk)
    }

    func testNativeSettingsDecodeCanDisableAppleVoiceProcessing() throws {
        let data = #"{"appleVoiceProcessingEnabled":false}"#.data(using: .utf8)!

        let settings = try JSONDecoder().decode(AppSettings.self, from: data)

        XCTAssertFalse(settings.appleVoiceProcessingEnabled)
        XCTAssertEqual(
            settings.audioInputVoiceProcessingConfiguration,
            AudioInputVoiceProcessingConfiguration(isEnabled: false, fallbackToUnprocessedAudio: true)
        )
    }

    func testLoadEnsuresNativeTranscriptionAPIDefaults() throws {
        let paths = makePaths()
        let settingsURL = paths.appDataDirectory.appendingPathComponent("native_settings.json")
        try """
        {
          "selectedModel": "",
          "transcriptionAPIProviderID": "",
          "transcriptionAPIProviders": [],
          "transcriptionAPIModels": []
        }
        """.write(to: settingsURL, atomically: true, encoding: .utf8)

        let settings = SettingsStore(paths: paths).load()

        XCTAssertEqual(settings.selectedModel, TranscriptionAPIProvider.mistralVoxtralModelID)
        XCTAssertEqual(settings.transcriptionAPIProviderID, TranscriptionAPIProvider.mistralProviderID)
        XCTAssertFalse(settings.transcriptionAPIProviders.isEmpty)
        XCTAssertFalse(settings.transcriptionAPIModels.isEmpty)
    }

    func testLoadPersistsNormalizedLegacyVoxtralModelSettings() throws {
        let paths = makePaths()
        let settingsURL = paths.appDataDirectory.appendingPathComponent("native_settings.json")
        try """
        {
          "selectedModel": "voxtral-small-2507",
          "transcriptionAPIProviderID": "mistral",
          "transcriptionAPIProviders": [],
          "transcriptionAPIModels": [
            {
              "description": "Cloud transcription via Mistral.",
              "display_name": "Mistral Voxtral Small",
              "id": "voxtral-small-2507",
              "is_custom": false,
              "model_id": "voxtral-small-2507",
              "provider_id": "mistral"
            },
            {
              "description": "Cloud transcription via Mistral.",
              "display_name": "Mistral Voxtral Small",
              "id": "voxtral-small-latest",
              "is_custom": false,
              "model_id": "voxtral-small-latest",
              "provider_id": "mistral"
            }
          ]
        }
        """.write(to: settingsURL, atomically: true, encoding: .utf8)

        let settings = SettingsStore(paths: paths).load()
        let persisted = try JSONDecoder().decode(AppSettings.self, from: Data(contentsOf: settingsURL))

        XCTAssertEqual(settings.selectedModel, TranscriptionAPIProvider.mistralVoxtralModelID)
        XCTAssertEqual(persisted.selectedModel, TranscriptionAPIProvider.mistralVoxtralModelID)
        XCTAssertFalse(persisted.transcriptionAPIModels.contains { $0.id == "voxtral-small-2507" })
        XCTAssertEqual(
            persisted.transcriptionAPIModels.filter { $0.displayName == "Mistral Voxtral Small" }.map(\.id),
            [TranscriptionAPIProvider.mistralVoxtralModelID]
        )
    }

    func testNativeKeyboardImplementationDecodesUnknownValuesAsNativeEventTap() throws {
        let settings = try JSONDecoder().decode(
            AppSettings.self,
            from: #"{"keyboardImplementation":"missing"}"#.data(using: .utf8)!
        )

        XCTAssertEqual(settings.keyboardImplementation, .nativeEventTap)
    }

    private func makePaths() -> AppPaths {
        let appDataDirectory = temporaryDirectory.appendingPathComponent("app-data", isDirectory: true)
        try? FileManager.default.createDirectory(at: appDataDirectory, withIntermediateDirectories: true)
        return AppPaths(
            appDataDirectory: appDataDirectory,
            recordingsDirectory: appDataDirectory.appendingPathComponent("recordings", isDirectory: true),
            modelsDirectory: appDataDirectory.appendingPathComponent("models", isDirectory: true),
            logsDirectory: temporaryDirectory.appendingPathComponent("logs", isDirectory: true)
        )
    }
}

private final class FlakyCredentialStore: PostProcessCredentialStoring, @unchecked Sendable {
    var shouldFailSaves = false
    private var apiKeys: [String: String] = [:]

    func readAPIKey(providerID: String) throws -> String? {
        apiKeys[providerID]
    }

    func saveAPIKey(_ apiKey: String, providerID: String) throws {
        if shouldFailSaves {
            throw NSError(domain: "FlakyCredentialStore", code: 1)
        }
        apiKeys[providerID] = apiKey
    }

    func deleteAPIKey(providerID: String) throws {
        apiKeys[providerID] = nil
    }
}
