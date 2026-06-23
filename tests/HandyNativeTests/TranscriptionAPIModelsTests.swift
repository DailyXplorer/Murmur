@testable import HandyNative
import XCTest

final class TranscriptionAPIModelsTests: XCTestCase {
    func testMistralDefaultsUseCurrentVoxtralAliases() throws {
        let settings = AppSettings.defaults
        _ = try XCTUnwrap(
            settings.transcriptionAPIProviders.first { $0.id == TranscriptionAPIProvider.mistralProviderID }
        )
        let defaultModel = try XCTUnwrap(
            settings.transcriptionAPIModels.first { $0.id == TranscriptionAPIProvider.mistralVoxtralModelID }
        )

        XCTAssertEqual(settings.selectedModel, "voxtral-small-latest")
        XCTAssertEqual(defaultModel.modelID, "voxtral-small-latest")
        XCTAssertFalse(settings.transcriptionAPIModels.contains { $0.id == "voxtral-mini-2507" })
    }

    func testTranscriptionAPIProviderIgnoresRemovedSchemaKeys() throws {
        let data = """
        {
          "id": "custom",
          "label": "Custom",
          "base_url": "http://localhost:11434/v1",
          "allow_base_url_edit": true,
          "models_endpoint": "/models",
          "api_kind": "audio_transcriptions",
          "suggested_models": ["legacy-model"],
          "requires_api_key": false
        }
        """.data(using: .utf8)!

        let provider = try JSONDecoder().decode(TranscriptionAPIProvider.self, from: data)

        XCTAssertEqual(provider.id, "custom")
        XCTAssertEqual(provider.label, "Custom")
        XCTAssertEqual(provider.baseURL, "http://localhost:11434/v1")
        XCTAssertTrue(provider.allowBaseURLEdit)
        XCTAssertEqual(provider.apiKind, .audioTranscriptions)
        XCTAssertFalse(provider.requiresAPIKey)
    }

    func testEnsureDefaultsMigratesLegacyVoxtralSelection() throws {
        var settings = AppSettings.defaults
        settings.selectedModel = "voxtral-small-2507"
        settings.transcriptionAPIProviders = [
            TranscriptionAPIProvider(
                id: TranscriptionAPIProvider.mistralProviderID,
                label: "Mistral",
                baseURL: "https://api.mistral.ai/v1",
                apiKind: .chatCompletionsInputAudio
            ),
        ]
        settings.transcriptionAPIModels = [
            TranscriptionAPIModel(
                id: "voxtral-small-2507",
                providerID: TranscriptionAPIProvider.mistralProviderID,
                modelID: "voxtral-small-2507",
                displayName: "Mistral Voxtral Small",
                description: "Cloud transcription via Mistral.",
                isCustom: false
            ),
        ]

        settings.ensureTranscriptionAPIDefaults()

        let mistralProvider = try XCTUnwrap(
            settings.transcriptionAPIProviders.first { $0.id == TranscriptionAPIProvider.mistralProviderID }
        )
        XCTAssertEqual(settings.selectedModel, "voxtral-small-latest")
        XCTAssertEqual(mistralProvider.apiKind, .chatCompletionsInputAudio)
        XCTAssertTrue(settings.transcriptionAPIModels.contains { $0.id == "voxtral-small-latest" })
        XCTAssertFalse(settings.transcriptionAPIModels.contains { $0.id == "voxtral-small-2507" })
    }

    func testEnsureDefaultsRemovesDuplicatedLegacyBuiltInVoxtralModel() throws {
        var settings = AppSettings.defaults
        settings.selectedModel = "voxtral-small-2507"
        settings.transcriptionAPIModels = [
            TranscriptionAPIModel(
                id: "voxtral-small-2507",
                providerID: TranscriptionAPIProvider.mistralProviderID,
                modelID: "voxtral-small-2507",
                displayName: "Mistral Voxtral Small",
                description: "Cloud transcription via Mistral.",
                isCustom: false
            ),
            TranscriptionAPIModel(
                id: "voxtral-small-latest",
                providerID: TranscriptionAPIProvider.mistralProviderID,
                modelID: "voxtral-small-latest",
                displayName: "Mistral Voxtral Small",
                description: "Cloud transcription via Mistral.",
                isCustom: false
            ),
        ]

        settings.ensureTranscriptionAPIDefaults()

        XCTAssertEqual(settings.selectedModel, "voxtral-small-latest")
        XCTAssertEqual(
            settings.transcriptionAPIModels.filter { $0.displayName == "Mistral Voxtral Small" }.map(\.id),
            ["voxtral-small-latest"]
        )
    }

    func testAddingAPIModelUsesCustomDisplayNameWithoutSelectingIt() throws {
        var settings = AppSettings.defaults
        let originalSelectedModel = settings.selectedModel

        settings.selectTranscriptionAPIProvider(id: TranscriptionAPIProvider.openAIProviderID)
        let model = try XCTUnwrap(
            settings.addTranscriptionAPIModelForSelectedProvider(
                modelID: "  gpt-4o-mini-transcribe  ",
                displayName: "  Fast Transcribe  "
            )
        )

        XCTAssertEqual(model.id, "api:openai:gpt-4o-mini-transcribe")
        XCTAssertEqual(model.providerID, TranscriptionAPIProvider.openAIProviderID)
        XCTAssertEqual(model.modelID, "gpt-4o-mini-transcribe")
        XCTAssertEqual(model.displayName, "Fast Transcribe")
        XCTAssertEqual(model.description, "Cloud transcription via OpenAI.")
        XCTAssertTrue(model.isCustom)
        XCTAssertEqual(settings.selectedModel, originalSelectedModel)
    }

    func testAddingAPIModelDefaultsDisplayNameWhenBlank() throws {
        var settings = AppSettings.defaults

        settings.selectTranscriptionAPIProvider(id: "groq")
        let model = try XCTUnwrap(
            settings.addTranscriptionAPIModelForSelectedProvider(
                modelID: "whisper-large-v3",
                displayName: "   "
            )
        )

        XCTAssertEqual(model.displayName, "Groq whisper-large-v3")
    }

    func testAddingAPIModelRejectsDuplicateNormalizedRecordID() throws {
        var settings = AppSettings.defaults

        settings.selectTranscriptionAPIProvider(id: TranscriptionAPIProvider.openAIProviderID)
        let firstModel = try XCTUnwrap(
            settings.addTranscriptionAPIModelForSelectedProvider(
                modelID: "GPT-4O-MINI-TRANSCRIBE",
                displayName: ""
            )
        )
        let duplicateModel = settings.addTranscriptionAPIModelForSelectedProvider(
            modelID: "gpt-4o-mini-transcribe",
            displayName: "Duplicate"
        )

        XCTAssertNil(duplicateModel)
        XCTAssertEqual(
            settings.transcriptionAPIModels.filter { $0.id == firstModel.id }.count,
            1
        )
    }

    func testUpsertSelectsExistingAPIModelByNormalizedRecordID() throws {
        var settings = AppSettings.defaults

        settings.selectTranscriptionAPIProvider(id: TranscriptionAPIProvider.openAIProviderID)
        let model = try XCTUnwrap(
            settings.addTranscriptionAPIModelForSelectedProvider(
                modelID: "GPT-4O-MINI-TRANSCRIBE",
                displayName: "Fast Transcribe"
            )
        )

        let selected = try XCTUnwrap(
            settings.upsertTranscriptionAPIModelForSelectedProvider(modelID: "gpt-4o-mini-transcribe")
        )

        XCTAssertEqual(selected.id, model.id)
        XCTAssertEqual(settings.selectedModel, model.id)
        XCTAssertEqual(
            settings.transcriptionAPIModels.filter { $0.id == model.id }.count,
            1
        )
    }
}
