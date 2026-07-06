import Foundation
@testable import MurmurNative
import XCTest

final class APITranscriptionServiceTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("MurmurNativeAPITranscription-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: temporaryDirectory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        APITranscriptionURLProtocolStub.handler = nil
        if let temporaryDirectory {
            try? FileManager.default.removeItem(at: temporaryDirectory)
        }
    }

    func testAudioTranscriptionEndpointSendsMultipartRequest() async throws {
        let audioURL = try writeAudioFixture()
        let provider = TranscriptionAPIProvider(
            id: "openai",
            label: "OpenAI",
            baseURL: "https://transcription.test/v1",
            apiKind: .audioTranscriptions
        )
        let model = TranscriptionAPIModel(
            id: "openai-whisper",
            providerID: provider.id,
            modelID: "whisper-1",
            displayName: "OpenAI Whisper",
            description: "Cloud transcription via OpenAI.",
            isCustom: false
        )
        var settings = AppSettings.defaults
        settings.selectedLanguage = "fr-FR"
        settings.selectedModel = model.id
        settings.transcriptionAPIProviderID = provider.id
        settings.transcriptionAPIProviders = [provider]
        settings.transcriptionAPIModels = [model]

        let credentialStore = InMemoryAPITranscriptionCredentialStore()
        try credentialStore.saveAPIKey("sk-transcribe", providerID: provider.id)

        APITranscriptionURLProtocolStub.handler = { request in
            XCTAssertEqual(request.httpMethod, "POST")
            XCTAssertEqual(request.url?.absoluteString, "https://transcription.test/v1/audio/transcriptions")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer sk-transcribe")
            XCTAssertTrue(request.value(forHTTPHeaderField: "Content-Type")?.hasPrefix("multipart/form-data; boundary=murmur-api-transcription-") == true)

            let body = try apiTranscriptionRequestBodyData(from: request)
            let bodyText = String(data: body, encoding: .utf8) ?? ""
            XCTAssertTrue(bodyText.contains("name=\"model\"\r\n\r\nwhisper-1"))
            XCTAssertTrue(bodyText.contains("name=\"response_format\"\r\n\r\njson"))
            XCTAssertTrue(bodyText.contains("name=\"language\"\r\n\r\nfr-FR"))
            XCTAssertTrue(bodyText.contains("filename=\"recording.wav\""))

            let response = HTTPURLResponse(
                url: try XCTUnwrap(request.url),
                statusCode: 200,
                httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            )!
            let data = #"{ "text": " “bonjour” " }"#.data(using: .utf8)!
            return (response, data)
        }

        let result = try await APITranscriptionService.transcribe(
            fileURL: audioURL,
            settings: settings,
            credentialStore: credentialStore,
            urlSession: stubbedSession()
        )

        XCTAssertEqual(result, "bonjour")
    }

    func testOpenAIWhisperTranslationUsesAudioTranslationsEndpoint() async throws {
        let audioURL = try writeAudioFixture()
        let provider = TranscriptionAPIProvider(
            id: TranscriptionAPIProvider.openAIProviderID,
            label: "OpenAI",
            baseURL: "https://transcription.test/v1",
            apiKind: .audioTranscriptions
        )
        let model = TranscriptionAPIModel(
            id: "openai-whisper",
            providerID: provider.id,
            modelID: "whisper-1",
            displayName: "OpenAI Whisper",
            description: "Cloud transcription via OpenAI.",
            isCustom: false
        )
        var settings = AppSettings.defaults
        settings.selectedLanguage = "fr-FR"
        settings.translateToEnglish = true
        settings.selectedModel = model.id
        settings.transcriptionAPIProviderID = provider.id
        settings.transcriptionAPIProviders = [provider]
        settings.transcriptionAPIModels = [model]

        let credentialStore = InMemoryAPITranscriptionCredentialStore()
        try credentialStore.saveAPIKey("sk-translate", providerID: provider.id)

        APITranscriptionURLProtocolStub.handler = { request in
            XCTAssertEqual(request.httpMethod, "POST")
            XCTAssertEqual(request.url?.absoluteString, "https://transcription.test/v1/audio/translations")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer sk-translate")

            let body = try apiTranscriptionRequestBodyData(from: request)
            let bodyText = String(data: body, encoding: .utf8) ?? ""
            XCTAssertTrue(bodyText.contains("name=\"model\"\r\n\r\nwhisper-1"))
            XCTAssertTrue(bodyText.contains("name=\"response_format\"\r\n\r\njson"))
            XCTAssertFalse(bodyText.contains("name=\"language\""))

            let response = HTTPURLResponse(
                url: try XCTUnwrap(request.url),
                statusCode: 200,
                httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            )!
            let data = #"{ "text": " hello " }"#.data(using: .utf8)!
            return (response, data)
        }

        let result = try await APITranscriptionService.transcribe(
            fileURL: audioURL,
            settings: settings,
            credentialStore: credentialStore,
            urlSession: stubbedSession()
        )

        XCTAssertEqual(result, "hello")
    }

    func testAudioTranslationKeepsUnsupportedOpenAIModelsOnTranscriptionEndpoint() async throws {
        let audioURL = try writeAudioFixture()
        let provider = TranscriptionAPIProvider(
            id: TranscriptionAPIProvider.openAIProviderID,
            label: "OpenAI",
            baseURL: "https://transcription.test/v1",
            apiKind: .audioTranscriptions
        )
        let model = TranscriptionAPIModel(
            id: "openai-gpt-transcribe",
            providerID: provider.id,
            modelID: "gpt-4o-mini-transcribe",
            displayName: "OpenAI GPT-4o mini transcribe",
            description: "Cloud transcription via OpenAI.",
            isCustom: false
        )
        var settings = AppSettings.defaults
        settings.selectedLanguage = "fr-FR"
        settings.translateToEnglish = true
        settings.selectedModel = model.id
        settings.transcriptionAPIProviderID = provider.id
        settings.transcriptionAPIProviders = [provider]
        settings.transcriptionAPIModels = [model]

        let credentialStore = InMemoryAPITranscriptionCredentialStore()
        try credentialStore.saveAPIKey("sk-transcribe", providerID: provider.id)

        APITranscriptionURLProtocolStub.handler = { request in
            XCTAssertEqual(request.url?.absoluteString, "https://transcription.test/v1/audio/transcriptions")

            let body = try apiTranscriptionRequestBodyData(from: request)
            let bodyText = String(data: body, encoding: .utf8) ?? ""
            XCTAssertTrue(bodyText.contains("name=\"model\"\r\n\r\ngpt-4o-mini-transcribe"))
            XCTAssertTrue(bodyText.contains("name=\"language\"\r\n\r\nfr-FR"))

            let response = HTTPURLResponse(
                url: try XCTUnwrap(request.url),
                statusCode: 200,
                httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            )!
            let data = #"{ "text": "bonjour" }"#.data(using: .utf8)!
            return (response, data)
        }

        let result = try await APITranscriptionService.transcribe(
            fileURL: audioURL,
            settings: settings,
            credentialStore: credentialStore,
            urlSession: stubbedSession()
        )

        XCTAssertEqual(result, "bonjour")
    }

    func testChatAudioEndpointSendsBase64AudioAndLanguagePrompt() async throws {
        let audioURL = try writeAudioFixture()
        let provider = TranscriptionAPIProvider(
            id: "mistral",
            label: "Mistral",
            baseURL: "https://transcription.test/v1/",
            apiKind: .chatCompletionsInputAudio
        )
        let model = TranscriptionAPIModel(
            id: "voxtral-small-2507",
            providerID: provider.id,
            modelID: "voxtral-small-2507",
            displayName: "Mistral Voxtral Small",
            description: "Cloud transcription via Mistral.",
            isCustom: false
        )
        var settings = AppSettings.defaults
        settings.selectedLanguage = "zh-Hant"
        settings.translateToEnglish = true
        settings.selectedModel = model.id
        settings.transcriptionAPIProviderID = provider.id
        settings.transcriptionAPIProviders = [provider]
        settings.transcriptionAPIModels = [model]

        let credentialStore = InMemoryAPITranscriptionCredentialStore()
        try credentialStore.saveAPIKey("mistral-key", providerID: provider.id)

        APITranscriptionURLProtocolStub.handler = { request in
            XCTAssertEqual(request.url?.absoluteString, "https://transcription.test/v1/chat/completions")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer mistral-key")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Content-Type"), "application/json")

            let body = try apiTranscriptionRequestBodyData(from: request)
            let json = try XCTUnwrap(JSONSerialization.jsonObject(with: body) as? [String: Any])
            XCTAssertEqual(json["model"] as? String, "voxtral-small-2507")
            XCTAssertEqual(json["temperature"] as? Double, 0)
            let messages = try XCTUnwrap(json["messages"] as? [[String: Any]])
            let content = try XCTUnwrap(messages.first?["content"] as? [[String: Any]])
            XCTAssertEqual(content.first?["type"] as? String, "input_audio")
            XCTAssertEqual(content.first?["input_audio"] as? String, try Data(contentsOf: audioURL).base64EncodedString())
            XCTAssertEqual(content.last?["type"] as? String, "text")
            XCTAssertEqual(
                content.last?["text"] as? String,
                "Transcribe this audio exactly in language code 'zh', then translate it to English. Return only the English text."
            )

            let response = HTTPURLResponse(
                url: try XCTUnwrap(request.url),
                statusCode: 200,
                httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            )!
            let data = """
            { "choices": [ { "message": { "content": [ { "text": "hello" }, { "text": " world" } ] } } ] }
            """.data(using: .utf8)!
            return (response, data)
        }

        let result = try await APITranscriptionService.transcribe(
            fileURL: audioURL,
            settings: settings,
            credentialStore: credentialStore,
            urlSession: stubbedSession()
        )

        XCTAssertEqual(result, "hello world")
    }

    func testProtectedProviderWithoutKeyThrowsMissingAPIKey() async throws {
        let audioURL = try writeAudioFixture()
        var settings = AppSettings.defaults

        do {
            _ = try await APITranscriptionService.transcribe(
                fileURL: audioURL,
                settings: settings,
                credentialStore: InMemoryAPITranscriptionCredentialStore(),
                urlSession: stubbedSession()
            )
            XCTFail("Expected missing API key")
        } catch let error as APITranscriptionServiceError {
            XCTAssertEqual(error.errorDescription, "API key is required for Mistral transcription.")
        }

        settings.transcriptionAPIProviders = [
            TranscriptionAPIProvider(
                id: "custom",
                label: "Custom",
                baseURL: "https://transcription.test/v1",
                apiKind: .audioTranscriptions,
                requiresAPIKey: false
            ),
        ]
        settings.transcriptionAPIModels = [
            TranscriptionAPIModel(
                id: "custom-model",
                providerID: "custom",
                modelID: "local-whisper",
                displayName: "Custom local-whisper",
                description: "Cloud transcription via Custom.",
                isCustom: true
            ),
        ]
        settings.selectedModel = "custom-model"

        APITranscriptionURLProtocolStub.handler = { request in
            XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
            let response = HTTPURLResponse(url: try XCTUnwrap(request.url), statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (response, #"{ "text": "custom" }"#.data(using: .utf8)!)
        }

        let result = try await APITranscriptionService.transcribe(
            fileURL: audioURL,
            settings: settings,
            credentialStore: InMemoryAPITranscriptionCredentialStore(),
            urlSession: stubbedSession()
        )

        XCTAssertEqual(result, "custom")
    }

    func testHTTPFailureDoesNotExposeProviderResponseBody() async throws {
        let audioURL = try writeAudioFixture()
        let provider = TranscriptionAPIProvider(
            id: "mistral",
            label: "Mistral",
            baseURL: "https://transcription.test/v1",
            apiKind: .audioTranscriptions
        )
        let model = TranscriptionAPIModel(
            id: "voxtral-small",
            providerID: provider.id,
            modelID: "voxtral-small",
            displayName: "Mistral Voxtral Small",
            description: "Cloud transcription via Mistral.",
            isCustom: false
        )
        var settings = AppSettings.defaults
        settings.selectedModel = model.id
        settings.transcriptionAPIProviderID = provider.id
        settings.transcriptionAPIProviders = [provider]
        settings.transcriptionAPIModels = [model]

        let credentialStore = InMemoryAPITranscriptionCredentialStore()
        try credentialStore.saveAPIKey("mistral-key", providerID: provider.id)

        APITranscriptionURLProtocolStub.handler = { request in
            let response = HTTPURLResponse(
                url: try XCTUnwrap(request.url),
                statusCode: 429,
                httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            )!
            let data = #"{"error":"provider detail: transcript contains private text"}"#.data(using: .utf8)!
            return (response, data)
        }

        do {
            _ = try await APITranscriptionService.transcribe(
                fileURL: audioURL,
                settings: settings,
                credentialStore: credentialStore,
                urlSession: stubbedSession()
            )
            XCTFail("Expected HTTP status failure")
        } catch let error as APITranscriptionServiceError {
            XCTAssertEqual(error.errorDescription, "Transcription request failed with status 429.")
            XCTAssertFalse(error.localizedDescription.contains("provider detail"))
            XCTAssertFalse(error.localizedDescription.contains("private text"))
        }
    }

    func testTranscriptionPromptKeepsNonTranslationContractByDefault() {
        XCTAssertEqual(
            APITranscriptionService.transcriptionPrompt(language: "auto"),
            "Transcribe this audio exactly. Do not translate. Return only the transcript."
        )
        XCTAssertEqual(
            APITranscriptionService.transcriptionPrompt(language: "fr", translateToEnglish: true),
            "Transcribe this audio exactly in language code 'fr', then translate it to English. Return only the English text."
        )
    }

    private func writeAudioFixture() throws -> URL {
        let url = temporaryDirectory.appendingPathComponent("recording.wav")
        try Data("RIFFmurmurWAVE".utf8).write(to: url)
        return url
    }

    private func stubbedSession() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [APITranscriptionURLProtocolStub.self]
        return URLSession(configuration: configuration)
    }
}

private final class InMemoryAPITranscriptionCredentialStore: PostProcessCredentialStoring, @unchecked Sendable {
    private var apiKeys: [String: String] = [:]

    func readAPIKey(providerID: String) throws -> String? {
        apiKeys[providerID]
    }

    func saveAPIKey(_ apiKey: String, providerID: String) throws {
        apiKeys[providerID] = apiKey
    }

    func deleteAPIKey(providerID: String) throws {
        apiKeys.removeValue(forKey: providerID)
    }
}

private final class APITranscriptionURLProtocolStub: URLProtocol {
    nonisolated(unsafe) static var handler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        guard let handler = Self.handler else {
            client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
            return
        }

        do {
            let (response, data) = try handler(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

private func apiTranscriptionRequestBodyData(from request: URLRequest) throws -> Data {
    if let body = request.httpBody {
        return body
    }

    guard let stream = request.httpBodyStream else {
        return Data()
    }

    stream.open()
    defer { stream.close() }

    var data = Data()
    let bufferSize = 1024
    var buffer = [UInt8](repeating: 0, count: bufferSize)

    while stream.hasBytesAvailable {
        let count = stream.read(&buffer, maxLength: bufferSize)
        if count < 0 {
            throw stream.streamError ?? URLError(.cannotDecodeContentData)
        }
        if count == 0 {
            break
        }
        data.append(buffer, count: count)
    }

    return data
}
