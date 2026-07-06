import Foundation

enum APITranscriptionServiceError: LocalizedError {
    case missingModel(String)
    case missingProvider(String)
    case missingAPIKey(String)
    case invalidURL(String)
    case httpStatus(Int)
    case missingResponseText(String)

    var errorDescription: String? {
        switch self {
        case let .missingModel(modelID):
            "Selected API transcription model was not found: \(modelID)."
        case let .missingProvider(providerID):
            "Transcription provider was not found: \(providerID)."
        case let .missingAPIKey(provider):
            "API key is required for \(provider) transcription."
        case let .invalidURL(url):
            "Invalid transcription URL: \(url)"
        case let .httpStatus(status):
            "Transcription request failed with status \(status)."
        case let .missingResponseText(provider):
            "\(provider) transcription response did not include text."
        }
    }
}

enum APITranscriptionService {
    static func transcribe(
        fileURL: URL,
        settings: AppSettings,
        credentialStore: any PostProcessCredentialStoring,
        urlSession: URLSession = .shared
    ) async throws -> String {
        guard let model = settings.selectedTranscriptionAPIModel else {
            throw APITranscriptionServiceError.missingModel(settings.selectedModel)
        }
        guard let provider = settings.transcriptionAPIProviders.first(where: { $0.id == model.providerID }) else {
            throw APITranscriptionServiceError.missingProvider(model.providerID)
        }

        let apiKey = (try? credentialStore.readAPIKey(providerID: provider.id))?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if provider.requiresAPIKey && apiKey.isEmpty {
            throw APITranscriptionServiceError.missingAPIKey(provider.label)
        }

        let wavData = try Data(contentsOf: fileURL)
        let transcript: String
        switch provider.apiKind {
        case .audioTranscriptions:
            transcript = try await transcribeWithAudioEndpoint(
                provider: provider,
                apiKey: apiKey,
                modelID: model.modelID,
                language: settings.selectedLanguage,
                translateToEnglish: settings.translateToEnglish,
                wavData: wavData,
                urlSession: urlSession
            )
        case .chatCompletionsInputAudio:
            transcript = try await transcribeWithChatAudio(
                provider: provider,
                apiKey: apiKey,
                modelID: model.modelID,
                language: settings.selectedLanguage,
                translateToEnglish: settings.translateToEnglish,
                wavData: wavData,
                urlSession: urlSession
            )
        }

        return cleanTranscript(transcript)
    }

    static func transcribeWithAudioEndpoint(
        provider: TranscriptionAPIProvider,
        apiKey: String,
        modelID: String,
        language: String,
        translateToEnglish: Bool,
        wavData: Data,
        urlSession: URLSession = .shared
    ) async throws -> String {
        let path = audioEndpointPath(provider: provider, modelID: modelID, translateToEnglish: translateToEnglish)
        var request = try makeURLRequest(provider: provider, apiKey: apiKey, path: path)
        let multipart = buildAudioTranscriptionMultipart(
            modelID: modelID,
            language: language,
            wavData: wavData,
            includeLanguage: path != "/audio/translations"
        )
        request.setValue(multipart.contentType, forHTTPHeaderField: "Content-Type")
        request.httpBody = multipart.body

        let (data, response) = try await urlSession.data(for: request)
        try validate(response: response, data: data)

        let object = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        guard let text = object?["text"] as? String else {
            throw APITranscriptionServiceError.missingResponseText(provider.label)
        }
        return text
    }

    static func audioEndpointPath(
        provider: TranscriptionAPIProvider,
        modelID: String,
        translateToEnglish: Bool
    ) -> String {
        guard translateToEnglish,
              provider.id == TranscriptionAPIProvider.openAIProviderID,
              modelID == "whisper-1"
        else {
            return "/audio/transcriptions"
        }

        return "/audio/translations"
    }

    static func transcribeWithChatAudio(
        provider: TranscriptionAPIProvider,
        apiKey: String,
        modelID: String,
        language: String,
        translateToEnglish: Bool,
        wavData: Data,
        urlSession: URLSession = .shared
    ) async throws -> String {
        var request = try makeURLRequest(provider: provider, apiKey: apiKey, path: "/chat/completions")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(
            TranscriptionChatCompletionRequest(
                model: modelID,
                messages: [
                    TranscriptionChatMessage(
                        role: "user",
                        content: [
                            .inputAudio(wavData.base64EncodedString()),
                            .text(transcriptionPrompt(language: language, translateToEnglish: translateToEnglish)),
                        ]
                    ),
                ],
                temperature: 0
            )
        )

        let (data, response) = try await urlSession.data(for: request)
        try validate(response: response, data: data)

        guard let text = extractChatResponseText(data: data) else {
            throw APITranscriptionServiceError.missingResponseText(provider.label)
        }
        return text
    }

    static func buildAudioTranscriptionMultipart(
        modelID: String,
        language: String,
        wavData: Data,
        includeLanguage: Bool = true
    ) -> (contentType: String, body: Data) {
        let boundary = "murmur-api-transcription-\(UUID().uuidString)"
        var body = Data()
        appendTextPart(name: "model", value: modelID, boundary: boundary, body: &body)
        appendTextPart(name: "response_format", value: "json", boundary: boundary, body: &body)

        if includeLanguage && language != "auto" {
            appendTextPart(name: "language", value: normalizeLanguage(language), boundary: boundary, body: &body)
        }

        body.appendUTF8("--\(boundary)\r\n")
        body.appendUTF8("Content-Disposition: form-data; name=\"file\"; filename=\"recording.wav\"\r\n")
        body.appendUTF8("Content-Type: audio/wav\r\n\r\n")
        body.append(wavData)
        body.appendUTF8("\r\n--\(boundary)--\r\n")

        return ("multipart/form-data; boundary=\(boundary)", body)
    }

    static func transcriptionPrompt(language: String, translateToEnglish: Bool = false) -> String {
        if translateToEnglish {
            if language == "auto" {
                return "Transcribe this audio exactly, then translate it to English. Return only the English text."
            }

            return "Transcribe this audio exactly in language code '\(normalizeLanguage(language))', then translate it to English. Return only the English text."
        }

        if language == "auto" {
            return "Transcribe this audio exactly. Do not translate. Return only the transcript."
        }

        return "Transcribe this audio exactly in language code '\(normalizeLanguage(language))'. Do not translate. Return only the transcript."
    }

    static func normalizeLanguage(_ language: String) -> String {
        switch language {
        case "zh-Hans", "zh-Hant":
            "zh"
        default:
            language
        }
    }

    static func cleanTranscript(_ content: String) -> String {
        content
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: "\"“”"))
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    static func extractChatResponseText(data: Data) -> String? {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let choices = root["choices"] as? [[String: Any]],
              let message = choices.first?["message"] as? [String: Any],
              let content = message["content"]
        else {
            return nil
        }

        if let text = content as? String {
            return text
        }

        if let parts = content as? [[String: Any]] {
            let text = parts.compactMap { $0["text"] as? String }.joined()
            return text.isEmpty ? nil : text
        }

        return nil
    }

    private static func makeURLRequest(provider: TranscriptionAPIProvider, apiKey: String, path: String) throws -> URLRequest {
        let baseURL = provider.baseURL.trimmingCharacters(in: .whitespacesAndNewlines).trimmingTrailingSlashes()
        let urlString = "\(baseURL)\(path)"
        guard let url = URL(string: urlString) else {
            throw APITranscriptionServiceError.invalidURL(urlString)
        }

        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        if !apiKey.isEmpty {
            request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        }
        return request
    }

    private static func appendTextPart(name: String, value: String, boundary: String, body: inout Data) {
        body.appendUTF8("--\(boundary)\r\n")
        body.appendUTF8("Content-Disposition: form-data; name=\"\(name)\"\r\n\r\n")
        body.appendUTF8(value)
        body.appendUTF8("\r\n")
    }

    private static func validate(response: URLResponse, data: Data) throws {
        guard let httpResponse = response as? HTTPURLResponse else {
            return
        }
        guard (200..<300).contains(httpResponse.statusCode) else {
            throw APITranscriptionServiceError.httpStatus(httpResponse.statusCode)
        }
    }
}

private extension Data {
    mutating func appendUTF8(_ value: String) {
        append(Data(value.utf8))
    }
}

private struct TranscriptionChatCompletionRequest: Encodable {
    var model: String
    var messages: [TranscriptionChatMessage]
    var temperature: Double
}

private struct TranscriptionChatMessage: Encodable {
    var role: String
    var content: [TranscriptionChatContent]
}

private enum TranscriptionChatContent: Encodable {
    case inputAudio(String)
    case text(String)

    enum CodingKeys: String, CodingKey {
        case type
        case inputAudio = "input_audio"
        case text
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .inputAudio(value):
            try container.encode("input_audio", forKey: .type)
            try container.encode(value, forKey: .inputAudio)
        case let .text(value):
            try container.encode("text", forKey: .type)
            try container.encode(value, forKey: .text)
        }
    }
}

private extension String {
    func trimmingTrailingSlashes() -> String {
        var value = self
        while value.last == "/" {
            value.removeLast()
        }
        return value
    }
}
