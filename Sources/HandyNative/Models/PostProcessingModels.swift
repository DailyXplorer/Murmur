import Foundation

struct PostProcessPrompt: Identifiable, Codable, Equatable {
    var id: String
    var name: String
    var prompt: String

    static let defaultImproveTranscriptions = PostProcessPrompt(
        id: "default_improve_transcriptions",
        name: "Improve Transcriptions",
        prompt: """
        Clean this transcript:
        1. Fix spelling, capitalization, and punctuation errors
        2. Convert number words to digits (twenty-five -> 25, ten percent -> 10%, five dollars -> $5)
        3. Replace spoken punctuation with symbols (period -> ., comma -> ,, question mark -> ?)
        4. Remove filler words (um, uh, like as filler)
        5. Keep the language in the original version (if it was french, keep it in french for example)

        Preserve exact meaning and word order. Do not paraphrase or reorder content.

        Return only the cleaned transcript.

        Transcript:
        ${output}
        """
    )

    static func generatedID(date: Date = Date()) -> String {
        "prompt_\(Int64(date.timeIntervalSince1970 * 1000))"
    }
}

struct PostProcessProvider: Identifiable, Codable, Equatable {
    var id: String
    var label: String
    var baseURL: String
    var allowBaseURLEdit: Bool
    var modelsEndpoint: String?
    var supportsStructuredOutput: Bool

    enum CodingKeys: String, CodingKey {
        case id
        case label
        case baseURL = "base_url"
        case allowBaseURLEdit = "allow_base_url_edit"
        case modelsEndpoint = "models_endpoint"
        case supportsStructuredOutput = "supports_structured_output"
    }

    init(
        id: String,
        label: String,
        baseURL: String,
        allowBaseURLEdit: Bool = false,
        modelsEndpoint: String? = nil,
        supportsStructuredOutput: Bool = false
    ) {
        self.id = id
        self.label = label
        self.baseURL = baseURL
        self.allowBaseURLEdit = allowBaseURLEdit
        self.modelsEndpoint = modelsEndpoint
        self.supportsStructuredOutput = supportsStructuredOutput
    }

    static let mistralProviderID = "mistral"
    static let appleIntelligenceProviderID = "apple_intelligence"
    static let appleIntelligenceModelID = "Apple Intelligence"
    static let mistralVoxtralModelID = "voxtral-small-2507"

    static let defaults: [PostProcessProvider] = {
        var providers = [
            PostProcessProvider(
                id: "openai",
                label: "OpenAI",
                baseURL: "https://api.openai.com/v1",
                modelsEndpoint: "/models",
                supportsStructuredOutput: true
            ),
            PostProcessProvider(
                id: mistralProviderID,
                label: "Mistral",
                baseURL: "https://api.mistral.ai/v1",
                modelsEndpoint: "/models"
            ),
            PostProcessProvider(
                id: "zai",
                label: "Z.AI",
                baseURL: "https://api.z.ai/api/paas/v4",
                modelsEndpoint: "/models",
                supportsStructuredOutput: true
            ),
            PostProcessProvider(
                id: "openrouter",
                label: "OpenRouter",
                baseURL: "https://openrouter.ai/api/v1",
                modelsEndpoint: "/models",
                supportsStructuredOutput: true
            ),
            PostProcessProvider(
                id: "anthropic",
                label: "Anthropic",
                baseURL: "https://api.anthropic.com/v1",
                modelsEndpoint: "/models"
            ),
            PostProcessProvider(
                id: "groq",
                label: "Groq",
                baseURL: "https://api.groq.com/openai/v1",
                modelsEndpoint: "/models"
            ),
            PostProcessProvider(
                id: "cerebras",
                label: "Cerebras",
                baseURL: "https://api.cerebras.ai/v1",
                modelsEndpoint: "/models",
                supportsStructuredOutput: true
            ),
        ]

        #if arch(arm64)
        providers.append(
            PostProcessProvider(
                id: appleIntelligenceProviderID,
                label: "Apple Intelligence",
                baseURL: "apple-intelligence://local",
                supportsStructuredOutput: true
            )
        )
        #endif

        providers.append(
            PostProcessProvider(
                id: "bedrock_mantle",
                label: "AWS Bedrock (Mantle)",
                baseURL: "https://bedrock-mantle.us-east-1.api.aws/v1",
                modelsEndpoint: "/models",
                supportsStructuredOutput: true
            )
        )

        providers.append(
            PostProcessProvider(
                id: "custom",
                label: "Custom",
                baseURL: "http://localhost:11434/v1",
                allowBaseURLEdit: true,
                modelsEndpoint: "/models"
            )
        )

        return providers
    }()

    static let defaultModels: [String: String] = {
        Dictionary(uniqueKeysWithValues: defaults.map { provider in
            switch provider.id {
            case appleIntelligenceProviderID:
                (provider.id, appleIntelligenceModelID)
            case mistralProviderID:
                (provider.id, mistralVoxtralModelID)
            default:
                (provider.id, "")
            }
        })
    }()
}

extension AppSettings {
    var selectedPostProcessProvider: PostProcessProvider? {
        postProcessProviders.first { $0.id == postProcessProviderID }
    }

    var selectedPostProcessPrompt: PostProcessPrompt? {
        guard let postProcessSelectedPromptID else {
            return nil
        }

        return postProcessPrompts.first { $0.id == postProcessSelectedPromptID }
    }

    var selectedPostProcessModelDisplay: String {
        let model = postProcessModels[postProcessProviderID]?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return model.isEmpty ? "Not configured" : model
    }

    mutating func ensurePostProcessDefaults() {
        if postProcessProviders.isEmpty {
            postProcessProviders = PostProcessProvider.defaults
        } else {
            for provider in PostProcessProvider.defaults where !postProcessProviders.contains(where: { $0.id == provider.id }) {
                postProcessProviders.append(provider)
            }
        }

        if !postProcessProviders.contains(where: { $0.id == postProcessProviderID }) {
            postProcessProviderID = PostProcessProvider.mistralProviderID
        }

        for (providerID, model) in PostProcessProvider.defaultModels where postProcessModels[providerID] == nil {
            postProcessModels[providerID] = model
        }

        if postProcessPrompts.isEmpty {
            postProcessPrompts = [.defaultImproveTranscriptions]
        }

        if let selectedID = postProcessSelectedPromptID,
           !postProcessPrompts.contains(where: { $0.id == selectedID }) {
            postProcessSelectedPromptID = nil
        }
    }

    mutating func selectPostProcessProvider(id: String) {
        guard postProcessProviders.contains(where: { $0.id == id }) else {
            return
        }
        postProcessProviderID = id
        if postProcessModels[id] == nil {
            postProcessModels[id] = PostProcessProvider.defaultModels[id] ?? ""
        }
    }

    mutating func updateSelectedPostProcessModel(_ model: String) {
        postProcessModels[postProcessProviderID] = model.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    mutating func updateSelectedPostProcessBaseURL(_ baseURL: String) {
        guard let index = postProcessProviders.firstIndex(where: { $0.id == postProcessProviderID }),
              postProcessProviders[index].allowBaseURLEdit
        else {
            return
        }
        postProcessProviders[index].baseURL = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    mutating func selectPostProcessPrompt(id: String) {
        guard postProcessPrompts.contains(where: { $0.id == id }) else {
            return
        }
        postProcessSelectedPromptID = id
    }

    mutating func addPostProcessPrompt(name: String, prompt: String, id: String = PostProcessPrompt.generatedID()) -> PostProcessPrompt? {
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedPrompt = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedName.isEmpty, !trimmedPrompt.isEmpty else {
            return nil
        }

        let newPrompt = PostProcessPrompt(id: id, name: trimmedName, prompt: trimmedPrompt)
        postProcessPrompts.append(newPrompt)
        postProcessSelectedPromptID = newPrompt.id
        return newPrompt
    }

    mutating func updatePostProcessPrompt(id: String, name: String, prompt: String) -> Bool {
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedPrompt = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedName.isEmpty, !trimmedPrompt.isEmpty,
              let index = postProcessPrompts.firstIndex(where: { $0.id == id })
        else {
            return false
        }

        postProcessPrompts[index].name = trimmedName
        postProcessPrompts[index].prompt = trimmedPrompt
        return true
    }

    mutating func deletePostProcessPrompt(id: String) -> Bool {
        guard postProcessPrompts.count > 1,
              let index = postProcessPrompts.firstIndex(where: { $0.id == id })
        else {
            return false
        }

        postProcessPrompts.remove(at: index)
        if postProcessSelectedPromptID == id {
            postProcessSelectedPromptID = postProcessPrompts.first?.id
        }
        return true
    }
}
