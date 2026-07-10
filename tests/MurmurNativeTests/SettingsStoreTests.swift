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

    func testMigrationStripsLegacyKeyFieldsAfterImport() throws {
        let paths = makePaths()
        let legacySettingsURL = paths.appDataDirectory.appendingPathComponent("settings_store.json")
        try """
        {
          "post_process_api_keys": {
            "mistral": "test-key-1"
          },
          "transcription_api_api_keys": {
            "mistral": "test-key-2"
          },
          "selected_model": "tiny"
        }
        """.write(to: legacySettingsURL, atomically: true, encoding: .utf8)
        let credentialStore = FlakyCredentialStore()

        _ = SettingsStore(paths: paths, credentialStore: credentialStore).loadResult()

        XCTAssertEqual(try credentialStore.readAPIKey(providerID: "mistral"), "test-key-2")
        XCTAssertFalse(credentialStore.savedProviderIDs.isEmpty)
        XCTAssertTrue(FileManager.default.fileExists(atPath: legacySettingsURL.path))
        let strippedObject = try XCTUnwrap(
            JSONSerialization.jsonObject(
                with: Data(contentsOf: legacySettingsURL)
            ) as? [String: Any]
        )
        XCTAssertNil(strippedObject["post_process_api_keys"])
        XCTAssertNil(strippedObject["postProcessAPIKeys"])
        XCTAssertNil(strippedObject["transcription_api_api_keys"])
        XCTAssertNil(strippedObject["transcriptionAPIAPIKeys"])
        XCTAssertEqual(strippedObject["selected_model"] as? String, "tiny")
    }

    func testMigrationStripsNestedSettingsKeyFields() throws {
        let paths = makePaths()
        let legacySettingsURL = paths.appDataDirectory.appendingPathComponent("settings_store.json")
        try """
        {
          "settings": {
            "postProcessAPIKeys": {
              "mistral": "test-key-3"
            },
            "transcriptionAPIAPIKeys": {
              "mistral": "test-key-4"
            },
            "selected_model": "tiny"
          }
        }
        """.write(to: legacySettingsURL, atomically: true, encoding: .utf8)
        let credentialStore = FlakyCredentialStore()

        _ = SettingsStore(paths: paths, credentialStore: credentialStore).loadResult()

        XCTAssertEqual(try credentialStore.readAPIKey(providerID: "mistral"), "test-key-4")
        XCTAssertTrue(FileManager.default.fileExists(atPath: legacySettingsURL.path))
        let strippedObject = try XCTUnwrap(
            JSONSerialization.jsonObject(
                with: Data(contentsOf: legacySettingsURL)
            ) as? [String: Any]
        )
        let nested = try XCTUnwrap(strippedObject["settings"] as? [String: Any])
        XCTAssertNil(nested["postProcessAPIKeys"])
        XCTAssertNil(nested["transcriptionAPIAPIKeys"])
        XCTAssertNil(nested["post_process_api_keys"])
        XCTAssertNil(nested["transcription_api_api_keys"])
        XCTAssertEqual(nested["selected_model"] as? String, "tiny")
    }

    func testAlreadyMarkedMigrationStillStripsResidualKeys() throws {
        let paths = makePaths()
        let legacySettingsURL = paths.appDataDirectory.appendingPathComponent("settings_store.json")
        let markerURL = paths.appDataDirectory.appendingPathComponent(".legacy_api_credentials_imported")
        try "imported\n".write(to: markerURL, atomically: true, encoding: .utf8)
        try """
        {
          "post_process_api_keys": {
            "mistral": "test-key-5"
          },
          "selected_model": "tiny"
        }
        """.write(to: legacySettingsURL, atomically: true, encoding: .utf8)
        let credentialStore = FlakyCredentialStore()

        _ = SettingsStore(paths: paths, credentialStore: credentialStore).loadResult()

        XCTAssertTrue(credentialStore.savedProviderIDs.isEmpty)
        XCTAssertTrue(FileManager.default.fileExists(atPath: legacySettingsURL.path))
        let strippedObject = try XCTUnwrap(
            JSONSerialization.jsonObject(
                with: Data(contentsOf: legacySettingsURL)
            ) as? [String: Any]
        )
        XCTAssertNil(strippedObject["post_process_api_keys"])
        XCTAssertEqual(strippedObject["selected_model"] as? String, "tiny")
        XCTAssertTrue(FileManager.default.fileExists(atPath: markerURL.path))
    }

    func testMigrationPreservesNonKeyLegacyContent() throws {
        let paths = makePaths()
        let legacySettingsURL = paths.appDataDirectory.appendingPathComponent("settings_store.json")
        let original: [String: Any] = [
            "selected_model": "tiny",
            "history_limit": 42,
            "push_to_talk": true,
            "settings": [
                "selected_language": "fr-FR",
                "post_process_api_keys": ["mistral": "test-key-6"],
                "custom_words": ["murmur", "voxtral"],
            ] as [String: Any],
            "transcription_api_api_keys": ["mistral": "test-key-7"],
        ]
        let originalData = try JSONSerialization.data(withJSONObject: original)
        try originalData.write(to: legacySettingsURL)
        let credentialStore = FlakyCredentialStore()

        _ = SettingsStore(paths: paths, credentialStore: credentialStore).loadResult()

        let strippedObject = try XCTUnwrap(
            JSONSerialization.jsonObject(
                with: Data(contentsOf: legacySettingsURL)
            ) as? [String: Any]
        )
        var expected = original
        expected.removeValue(forKey: "transcription_api_api_keys")
        var expectedNested = try XCTUnwrap(expected["settings"] as? [String: Any])
        expectedNested.removeValue(forKey: "post_process_api_keys")
        expected["settings"] = expectedNested
        XCTAssertEqual(strippedObject as NSDictionary, expected as NSDictionary)
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

    func testNativeSettingsDecodeClampsHistoryLimitToAtLeastOne() throws {
        let settings = try JSONDecoder().decode(
            AppSettings.self,
            from: #"{"historyLimit":0}"#.data(using: .utf8)!
        )

        XCTAssertEqual(settings.historyLimit, 1)
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
    private(set) var savedProviderIDs: [String] = []
    private var apiKeys: [String: String] = [:]

    func readAPIKey(providerID: String) throws -> String? {
        apiKeys[providerID]
    }

    func saveAPIKey(_ apiKey: String, providerID: String) throws {
        if shouldFailSaves {
            throw NSError(domain: "FlakyCredentialStore", code: 1)
        }
        savedProviderIDs.append(providerID)
        apiKeys[providerID] = apiKey
    }

    func deleteAPIKey(providerID: String) throws {
        apiKeys[providerID] = nil
    }
}
