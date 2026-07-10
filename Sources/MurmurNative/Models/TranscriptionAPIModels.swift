import Foundation

enum TranscriptionAPIKind: String, Codable, Equatable {
    case audioTranscriptions = "audio_transcriptions"
    case chatCompletionsInputAudio = "chat_completions_input_audio"
}

struct TranscriptionAPIProvider: Identifiable, Codable, Equatable {
    var id: String
    var label: String
    var baseURL: String
    var allowBaseURLEdit: Bool
    var apiKind: TranscriptionAPIKind
    var requiresAPIKey: Bool

    enum CodingKeys: String, CodingKey {
        case id
        case label
        case baseURL = "base_url"
        case allowBaseURLEdit = "allow_base_url_edit"
        case apiKind = "api_kind"
        case requiresAPIKey = "requires_api_key"
    }

    init(
        id: String,
        label: String,
        baseURL: String,
        allowBaseURLEdit: Bool = false,
        apiKind: TranscriptionAPIKind = .audioTranscriptions,
        requiresAPIKey: Bool = true
    ) {
        self.id = id
        self.label = label
        self.baseURL = baseURL
        self.allowBaseURLEdit = allowBaseURLEdit
        self.apiKind = apiKind
        self.requiresAPIKey = requiresAPIKey
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        label = try container.decode(String.self, forKey: .label)
        baseURL = try container.decode(String.self, forKey: .baseURL)
        allowBaseURLEdit = try container.decodeIfPresent(Bool.self, forKey: .allowBaseURLEdit) ?? false
        apiKind = try container.decodeIfPresent(TranscriptionAPIKind.self, forKey: .apiKind) ?? .audioTranscriptions
        requiresAPIKey = try container.decodeIfPresent(Bool.self, forKey: .requiresAPIKey) ?? true
    }

    static let appleSpeechModelID = "apple-speech-native"
    static let openAIProviderID = "openai"
    static let mistralProviderID = "mistral"
    static let mistralVoxtralModelID = "voxtral-small-latest"
    static let legacyMistralVoxtralModelIDs = [
        "voxtral-small-2507",
        "voxtral-mini-2507",
    ]
    static let openRouterProviderID = "openrouter"

    static let defaults: [TranscriptionAPIProvider] = [
        TranscriptionAPIProvider(
            id: openAIProviderID,
            label: "OpenAI",
            baseURL: "https://api.openai.com/v1",
            apiKind: .audioTranscriptions
        ),
        TranscriptionAPIProvider(
            id: mistralProviderID,
            label: "Mistral",
            baseURL: "https://api.mistral.ai/v1",
            apiKind: .chatCompletionsInputAudio
        ),
        TranscriptionAPIProvider(
            id: openRouterProviderID,
            label: "OpenRouter",
            baseURL: "https://openrouter.ai/api/v1",
            apiKind: .chatCompletionsInputAudio
        ),
        TranscriptionAPIProvider(
            id: "groq",
            label: "Groq",
            baseURL: "https://api.groq.com/openai/v1",
            apiKind: .audioTranscriptions
        ),
        TranscriptionAPIProvider(
            id: "custom",
            label: "Custom",
            baseURL: "http://localhost:11434/v1",
            allowBaseURLEdit: true,
            apiKind: .audioTranscriptions,
            requiresAPIKey: false
        ),
    ]
}

struct TranscriptionAPIModel: Identifiable, Codable, Equatable {
    var id: String
    var providerID: String
    var modelID: String
    var displayName: String
    var description: String
    var isCustom: Bool

    enum CodingKeys: String, CodingKey {
        case id
        case providerID = "provider_id"
        case modelID = "model_id"
        case displayName = "display_name"
        case description
        case isCustom = "is_custom"
    }

    init(
        id: String,
        providerID: String,
        modelID: String,
        displayName: String,
        description: String,
        isCustom: Bool
    ) {
        self.id = id
        self.providerID = providerID
        self.modelID = modelID
        self.displayName = displayName
        self.description = description
        self.isCustom = isCustom
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        providerID = try container.decode(String.self, forKey: .providerID)
        modelID = try container.decode(String.self, forKey: .modelID)
        displayName = try container.decodeIfPresent(String.self, forKey: .displayName) ?? modelID
        description = try container.decodeIfPresent(String.self, forKey: .description) ?? ""
        isCustom = try container.decodeIfPresent(Bool.self, forKey: .isCustom) ?? false
    }

    static let defaults: [TranscriptionAPIModel] = [
        TranscriptionAPIModel(
            id: TranscriptionAPIProvider.mistralVoxtralModelID,
            providerID: TranscriptionAPIProvider.mistralProviderID,
            modelID: TranscriptionAPIProvider.mistralVoxtralModelID,
            displayName: "Mistral Voxtral Small",
            description: "Cloud transcription via Mistral.",
            isCustom: false
        ),
    ]
}

extension AppSettings {
    var selectedTranscriptionAPIProvider: TranscriptionAPIProvider? {
        transcriptionAPIProviders.first { $0.id == transcriptionAPIProviderID }
    }

    var selectedTranscriptionAPIModel: TranscriptionAPIModel? {
        transcriptionAPIModels.first { $0.id == selectedModel }
    }

    var selectedLocalTranscriptionModel: LocalTranscriptionModel? {
        LocalTranscriptionModel.model(for: selectedModel)
    }

    var selectedTranscriptionModelDisplayName: String {
        if let localModel = selectedLocalTranscriptionModel {
            return localModel.name
        }
        if let apiModel = selectedTranscriptionAPIModel {
            return apiModel.displayName
        }
        if selectedModel == TranscriptionAPIProvider.appleSpeechModelID {
            return "Apple Speech"
        }
        return selectedModel
    }

    var selectedTranscriptionAPIModelForProvider: TranscriptionAPIModel? {
        transcriptionAPIModels.first { $0.providerID == transcriptionAPIProviderID }
    }

    var selectedTranscriptionSupportsTranslation: Bool {
        if let localModel = selectedLocalTranscriptionModel {
            return localModel.supportsTranslation
        }
        guard let apiModel = selectedTranscriptionAPIModel,
              let provider = transcriptionAPIProviders.first(where: { $0.id == apiModel.providerID })
        else {
            return false
        }
        return provider.apiKind == .chatCompletionsInputAudio
    }

    var selectedTranscriptionSupportsLanguageSelection: Bool {
        if selectedLocalTranscriptionModel != nil {
            return true
        }
        if selectedTranscriptionAPIModel != nil {
            return true
        }
        return selectedModel == TranscriptionAPIProvider.appleSpeechModelID
    }

    var selectedTranscriptionHasModelSettings: Bool {
        selectedTranscriptionSupportsLanguageSelection || selectedTranscriptionSupportsTranslation
    }

    mutating func ensureTranscriptionAPIDefaults() {
        if transcriptionAPIProviders.isEmpty {
            transcriptionAPIProviders = TranscriptionAPIProvider.defaults
        } else {
            for provider in TranscriptionAPIProvider.defaults {
                if let index = transcriptionAPIProviders.firstIndex(where: { $0.id == provider.id }) {
                    var updatedProvider = provider
                    if provider.allowBaseURLEdit {
                        updatedProvider.baseURL = transcriptionAPIProviders[index].baseURL
                    }
                    transcriptionAPIProviders[index] = updatedProvider
                } else {
                    transcriptionAPIProviders.append(provider)
                }
            }
        }

        let legacyMistralVoxtralModelIDs = Set(TranscriptionAPIProvider.legacyMistralVoxtralModelIDs)
        transcriptionAPIModels.removeAll { model in
            !model.isCustom &&
                model.providerID == TranscriptionAPIProvider.mistralProviderID &&
                (legacyMistralVoxtralModelIDs.contains(model.id) ||
                    legacyMistralVoxtralModelIDs.contains(model.modelID))
        }

        if !transcriptionAPIProviders.contains(where: { $0.id == transcriptionAPIProviderID }) {
            transcriptionAPIProviderID = TranscriptionAPIProvider.mistralProviderID
        }

        if transcriptionAPIModels.isEmpty {
            transcriptionAPIModels = TranscriptionAPIModel.defaults
        } else {
            for model in TranscriptionAPIModel.defaults {
                if let index = transcriptionAPIModels.firstIndex(where: { $0.id == model.id }) {
                    if !transcriptionAPIModels[index].isCustom {
                        transcriptionAPIModels[index] = model
                    }
                } else {
                    transcriptionAPIModels.append(model)
                }
            }
        }

        if TranscriptionAPIProvider.legacyMistralVoxtralModelIDs.contains(selectedModel) ||
            selectedModel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            selectedModel = TranscriptionAPIProvider.mistralVoxtralModelID
        }

        let selectionIsValid = selectedModel == TranscriptionAPIProvider.appleSpeechModelID ||
            LocalTranscriptionModel.model(for: selectedModel) != nil ||
            transcriptionAPIModels.contains { $0.id == selectedModel }
        if !selectionIsValid {
            selectedModel = TranscriptionAPIProvider.mistralVoxtralModelID
        }
    }

    mutating func selectTranscriptionAPIProvider(id: String) {
        guard transcriptionAPIProviders.contains(where: { $0.id == id }) else {
            return
        }
        transcriptionAPIProviderID = id
    }

    mutating func updateSelectedTranscriptionAPIBaseURL(_ baseURL: String) {
        guard let index = transcriptionAPIProviders.firstIndex(where: { $0.id == transcriptionAPIProviderID }),
              transcriptionAPIProviders[index].allowBaseURLEdit
        else {
            return
        }
        transcriptionAPIProviders[index].baseURL = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    mutating func selectTranscriptionModel(id: String) {
        guard id == TranscriptionAPIProvider.appleSpeechModelID ||
              LocalTranscriptionModel.model(for: id) != nil ||
              transcriptionAPIModels.contains(where: { $0.id == id })
        else {
            return
        }
        selectedModel = id
    }

    func transcriptionAPIModelExistsForSelectedProvider(modelID: String) -> Bool {
        let trimmedModelID = modelID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedModelID.isEmpty,
              let provider = selectedTranscriptionAPIProvider
        else {
            return false
        }

        let recordID = Self.transcriptionAPIModelRecordID(providerID: provider.id, modelID: trimmedModelID)
        return transcriptionAPIModels.contains {
            $0.id == recordID || ($0.providerID == provider.id && $0.modelID == trimmedModelID)
        }
    }

    mutating func addTranscriptionAPIModelForSelectedProvider(modelID: String, displayName: String) -> TranscriptionAPIModel? {
        let trimmedModelID = modelID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedModelID.isEmpty,
              let provider = selectedTranscriptionAPIProvider,
              !transcriptionAPIModelExistsForSelectedProvider(modelID: trimmedModelID)
        else {
            return nil
        }

        let trimmedDisplayName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let model = TranscriptionAPIModel(
            id: Self.transcriptionAPIModelRecordID(providerID: provider.id, modelID: trimmedModelID),
            providerID: provider.id,
            modelID: trimmedModelID,
            displayName: trimmedDisplayName.isEmpty ? "\(provider.label) \(trimmedModelID)" : trimmedDisplayName,
            description: "Cloud transcription via \(provider.label).",
            isCustom: true
        )
        transcriptionAPIModels.append(model)
        return model
    }

    mutating func upsertTranscriptionAPIModelForSelectedProvider(modelID: String) -> TranscriptionAPIModel? {
        let trimmedModelID = modelID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedModelID.isEmpty,
              let provider = selectedTranscriptionAPIProvider
        else {
            return nil
        }

        let id = Self.transcriptionAPIModelRecordID(providerID: provider.id, modelID: trimmedModelID)
        if let existing = transcriptionAPIModels.first(where: { $0.id == id || ($0.providerID == provider.id && $0.modelID == trimmedModelID) }) {
            selectedModel = existing.id
            return existing
        }

        let model = TranscriptionAPIModel(
            id: id,
            providerID: provider.id,
            modelID: trimmedModelID,
            displayName: "\(provider.label) \(trimmedModelID)",
            description: "Cloud transcription via \(provider.label).",
            isCustom: true
        )
        transcriptionAPIModels.append(model)
        selectedModel = model.id
        return model
    }

    @discardableResult
    mutating func removeTranscriptionAPIModel(id: String) -> Bool {
        guard let index = transcriptionAPIModels.firstIndex(where: { $0.id == id }),
              transcriptionAPIModels[index].isCustom
        else {
            return false
        }

        transcriptionAPIModels.remove(at: index)
        if selectedModel == id {
            selectedModel = TranscriptionAPIProvider.mistralVoxtralModelID
        }
        return true
    }

    static func transcriptionAPIModelRecordID(providerID: String, modelID: String) -> String {
        "api:\(sanitizedModelIDFragment(providerID)):\(sanitizedModelIDFragment(modelID))"
    }

    private static func sanitizedModelIDFragment(_ value: String) -> String {
        let sanitized = value
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .map { character -> Character in
                if character.isASCII, character.isLetter || character.isNumber || character == "-" || character == "_" {
                    return Character(character.lowercased())
                }
                return "-"
            }

        return String(sanitized)
            .split(separator: "-")
            .filter { !$0.isEmpty }
            .joined(separator: "-")
    }
}
