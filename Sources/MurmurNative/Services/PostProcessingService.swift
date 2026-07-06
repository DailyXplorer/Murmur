import Foundation

private let transcriptionField = "transcription"

struct PreparedPostProcessPrompt: Equatable {
    var systemPrompt: String
    var renderedPrompt: String
}

struct ChatMessage: Codable, Equatable {
    var role: String
    var content: String
}

struct ReasoningConfig: Codable, Equatable {
    var effort: String?
    var exclude: Bool?
}

struct ChatCompletionRequest: Encodable, Equatable {
    var model: String
    var messages: [ChatMessage]
    var responseFormat: ResponseFormat?
    var reasoningEffort: String?
    var reasoning: ReasoningConfig?

    enum CodingKeys: String, CodingKey {
        case model
        case messages
        case responseFormat = "response_format"
        case reasoningEffort = "reasoning_effort"
        case reasoning
    }
}

struct ResponseFormat: Encodable, Equatable {
    var type = "json_schema"
    var jsonSchema: JsonSchema

    enum CodingKeys: String, CodingKey {
        case type
        case jsonSchema = "json_schema"
    }
}

struct JsonSchema: Encodable, Equatable {
    var name = "transcription_output"
    var strict = true
    var schema: JSONValue
}

enum JSONValue: Encodable, Equatable {
    case string(String)
    case bool(Bool)
    case object([String: JSONValue])
    case array([JSONValue])

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case let .string(value):
            try container.encode(value)
        case let .bool(value):
            try container.encode(value)
        case let .object(value):
            try container.encode(value)
        case let .array(value):
            try container.encode(value)
        }
    }
}

struct ChatCompletionResponse: Decodable {
    var choices: [ChatChoice]
}

struct ChatChoice: Decodable {
    var message: ChatMessageResponse
}

struct ChatMessageResponse: Decodable {
    var content: String?
}

enum PostProcessingError: LocalizedError {
    case invalidURL(String)
    case httpStatus(Int)
    case missingAPIKey(String)
    case missingResponseContent

    var errorDescription: String? {
        switch self {
        case let .invalidURL(url):
            "Invalid post-processing URL: \(url)"
        case let .httpStatus(status):
            "Post-processing request failed with status \(status)."
        case let .missingAPIKey(provider):
            "API key is required for \(provider)."
        case .missingResponseContent:
            "Post-processing response did not include text."
        }
    }
}

enum PostProcessingService {
    static func prepare(template: String, transcription: String) -> PreparedPostProcessPrompt {
        PreparedPostProcessPrompt(
            systemPrompt: systemPrompt(from: template),
            renderedPrompt: renderedPrompt(from: template, transcription: transcription)
        )
    }

    static func systemPrompt(from template: String) -> String {
        template
            .replacingOccurrences(of: "${output}", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    static func renderedPrompt(from template: String, transcription: String) -> String {
        template.replacingOccurrences(of: "${output}", with: transcription)
    }

    static func stripInvisibleCharacters(_ text: String) -> String {
        let invisibleScalars: Set<UnicodeScalar> = [
            "\u{200B}",
            "\u{200C}",
            "\u{200D}",
            "\u{FEFF}",
        ]

        return String(text.unicodeScalars.filter { !invisibleScalars.contains($0) })
    }

    static func postProcessTranscription(
        settings: AppSettings,
        transcription: String,
        credentialStore: any PostProcessCredentialStoring,
        urlSession: URLSession = .shared,
        appleIntelligenceProcessor: any AppleIntelligenceProcessing = AppleIntelligenceService()
    ) async -> String? {
        guard let provider = settings.selectedPostProcessProvider,
              let prompt = settings.selectedPostProcessPrompt?.prompt.trimmingCharacters(in: .whitespacesAndNewlines),
              !prompt.isEmpty
        else {
            return nil
        }

        if provider.id == PostProcessProvider.appleIntelligenceProviderID {
            do {
                let prepared = prepare(template: prompt, transcription: transcription)
                let tokenLimit = AppleIntelligenceService.tokenLimit(from: settings.postProcessModels[provider.id])
                let output = try await appleIntelligenceProcessor.process(
                    systemPrompt: prepared.systemPrompt,
                    userContent: transcription,
                    tokenLimit: tokenLimit
                )
                return stripInvisibleCharacters(output)
            } catch {
                return nil
            }
        }

        let model = settings.postProcessModels[provider.id]?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !model.isEmpty else {
            return nil
        }

        let apiKey = (try? credentialStore.readAPIKey(providerID: provider.id))?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard provider.id == "custom" || !apiKey.isEmpty else {
            return nil
        }

        let reasoning = reasoningConfiguration(providerID: provider.id)

        if provider.supportsStructuredOutput {
            do {
                let prepared = prepare(template: prompt, transcription: transcription)
                let content = try await sendChatCompletion(
                    provider: provider,
                    apiKey: apiKey,
                    model: model,
                    userContent: transcription,
                    systemPrompt: prepared.systemPrompt,
                    schema: transcriptionSchema(),
                    reasoningEffort: reasoning.effort,
                    reasoning: reasoning.config,
                    urlSession: urlSession
                )
                return extractStructuredTranscription(content)
            } catch {
                // Structured-output failures fall back to plain text prompting.
            }
        }

        do {
            let content = try await sendChatCompletion(
                provider: provider,
                apiKey: apiKey,
                model: model,
                userContent: renderedPrompt(from: prompt, transcription: transcription),
                systemPrompt: nil,
                schema: nil,
                reasoningEffort: reasoning.effort,
                reasoning: reasoning.config,
                urlSession: urlSession
            )
            return stripInvisibleCharacters(content)
        } catch {
            return nil
        }
    }

    static func makeChatCompletionRequest(
        provider: PostProcessProvider,
        model: String,
        transcription: String,
        prompt: String
    ) -> ChatCompletionRequest {
        let reasoning = reasoningConfiguration(providerID: provider.id)
        if provider.supportsStructuredOutput {
            let prepared = prepare(template: prompt, transcription: transcription)
            return ChatCompletionRequest(
                model: model,
                messages: [
                    ChatMessage(role: "system", content: prepared.systemPrompt),
                    ChatMessage(role: "user", content: transcription),
                ],
                responseFormat: ResponseFormat(jsonSchema: JsonSchema(schema: transcriptionSchema())),
                reasoningEffort: reasoning.effort,
                reasoning: reasoning.config
            )
        }

        return ChatCompletionRequest(
            model: model,
            messages: [
                ChatMessage(role: "user", content: renderedPrompt(from: prompt, transcription: transcription)),
            ],
            responseFormat: nil,
            reasoningEffort: reasoning.effort,
            reasoning: reasoning.config
        )
    }

    static func extractStructuredTranscription(_ content: String) -> String {
        guard let data = content.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let value = object[transcriptionField] as? String
        else {
            return stripInvisibleCharacters(content)
        }

        return stripInvisibleCharacters(value)
    }

    static func fetchModels(
        provider: PostProcessProvider,
        credentialStore: any PostProcessCredentialStoring,
        urlSession: URLSession = .shared
    ) async throws -> [String] {
        let apiKey = (try? credentialStore.readAPIKey(providerID: provider.id))?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard provider.id == "custom" || !apiKey.isEmpty else {
            throw PostProcessingError.missingAPIKey(provider.label)
        }

        let endpoint = provider.modelsEndpoint ?? "/models"
        let request = try makeURLRequest(
            provider: provider,
            apiKey: apiKey,
            path: endpoint,
            method: "GET"
        )
        let (data, response) = try await urlSession.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            return []
        }
        guard (200..<300).contains(httpResponse.statusCode) else {
            throw PostProcessingError.httpStatus(httpResponse.statusCode)
        }

        return parseModelList(data: data)
    }

    private static func sendChatCompletion(
        provider: PostProcessProvider,
        apiKey: String,
        model: String,
        userContent: String,
        systemPrompt: String?,
        schema: JSONValue?,
        reasoningEffort: String?,
        reasoning: ReasoningConfig?,
        urlSession: URLSession
    ) async throws -> String {
        let requestBody = chatCompletionBody(
            model: model,
            userContent: userContent,
            systemPrompt: systemPrompt,
            schema: schema,
            reasoningEffort: reasoningEffort,
            reasoning: reasoning
        )
        var request = try makeURLRequest(
            provider: provider,
            apiKey: apiKey,
            path: "/chat/completions",
            method: "POST"
        )
        request.httpBody = try JSONEncoder().encode(requestBody)

        let (data, response) = try await urlSession.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw PostProcessingError.missingResponseContent
        }
        guard (200..<300).contains(httpResponse.statusCode) else {
            throw PostProcessingError.httpStatus(httpResponse.statusCode)
        }

        let completion = try JSONDecoder().decode(ChatCompletionResponse.self, from: data)
        guard let content = completion.choices.first?.message.content else {
            throw PostProcessingError.missingResponseContent
        }
        return content
    }

    private static func makeURLRequest(
        provider: PostProcessProvider,
        apiKey: String,
        path: String,
        method: String
    ) throws -> URLRequest {
        let baseURL = provider.baseURL.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let normalizedPath = path.hasPrefix("/") ? path : "/\(path)"
        let urlString = "\(baseURL)\(normalizedPath)"
        guard let url = URL(string: urlString) else {
            throw PostProcessingError.invalidURL(urlString)
        }

        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("https://github.com/", forHTTPHeaderField: "Referer")
        request.setValue("Murmur/1.0", forHTTPHeaderField: "User-Agent")
        request.setValue("Murmur", forHTTPHeaderField: "X-Title")

        if !apiKey.isEmpty {
            if provider.id == "anthropic" {
                request.setValue(apiKey, forHTTPHeaderField: "x-api-key")
                request.setValue("2023-06-01", forHTTPHeaderField: "anthropic-version")
            } else {
                request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
            }
        }

        return request
    }

    private static func parseModelList(data: Data) -> [String] {
        guard let parsed = try? JSONSerialization.jsonObject(with: data) else {
            return []
        }

        let models: [String]
        if let object = parsed as? [String: Any],
           let data = object["data"] as? [[String: Any]] {
            models = data.compactMap { entry in
                if let id = entry["id"] as? String {
                    return id
                }
                if let name = entry["name"] as? String {
                    return name
                }
                return nil
            }
        } else if let array = parsed as? [String] {
            models = array
        } else {
            models = []
        }

        return models
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    private static func chatCompletionBody(
        model: String,
        userContent: String,
        systemPrompt: String?,
        schema: JSONValue?,
        reasoningEffort: String?,
        reasoning: ReasoningConfig?
    ) -> ChatCompletionRequest {
        var messages: [ChatMessage] = []
        if let systemPrompt, !systemPrompt.isEmpty {
            messages.append(ChatMessage(role: "system", content: systemPrompt))
        }
        messages.append(ChatMessage(role: "user", content: userContent))

        return ChatCompletionRequest(
            model: model,
            messages: messages,
            responseFormat: schema.map { ResponseFormat(jsonSchema: JsonSchema(schema: $0)) },
            reasoningEffort: reasoningEffort,
            reasoning: reasoning
        )
    }

    private static func reasoningConfiguration(providerID: String) -> (effort: String?, config: ReasoningConfig?) {
        switch providerID {
        case "custom":
            (effort: "none", config: nil)
        case "openrouter":
            (effort: nil, config: ReasoningConfig(effort: "none", exclude: true))
        default:
            (effort: nil, config: nil)
        }
    }

    private static func transcriptionSchema() -> JSONValue {
        .object([
            "type": .string("object"),
            "properties": .object([
                transcriptionField: .object([
                    "type": .string("string"),
                    "description": .string("The cleaned and processed transcription text"),
                ]),
            ]),
            "required": .array([.string(transcriptionField)]),
            "additionalProperties": .bool(false),
        ])
    }
}
