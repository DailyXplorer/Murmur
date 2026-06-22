import Foundation
@testable import HandyNative
import XCTest

final class PostProcessingTests: XCTestCase {
    func testPromptMutationsMatchExistingPromptRules() {
        var settings = AppSettings.defaults

        XCTAssertNil(settings.selectedPostProcessPrompt)
        XCTAssertFalse(settings.deletePostProcessPrompt(id: PostProcessPrompt.defaultImproveTranscriptions.id))

        let created = settings.addPostProcessPrompt(
            name: "  Support reply  ",
            prompt: "  Clean ${output}  ",
            id: "prompt_test"
        )

        XCTAssertEqual(created?.id, "prompt_test")
        XCTAssertEqual(settings.postProcessSelectedPromptID, "prompt_test")
        XCTAssertEqual(settings.selectedPostProcessPrompt?.name, "Support reply")
        XCTAssertTrue(settings.updatePostProcessPrompt(id: "prompt_test", name: "Formal", prompt: "Rewrite ${output}"))
        XCTAssertEqual(settings.selectedPostProcessPrompt?.prompt, "Rewrite ${output}")
        XCTAssertTrue(settings.deletePostProcessPrompt(id: "prompt_test"))
        XCTAssertEqual(settings.postProcessSelectedPromptID, PostProcessPrompt.defaultImproveTranscriptions.id)
    }

    func testEnsurePostProcessDefaultsClearsMissingSelectedPrompt() {
        var settings = AppSettings.defaults
        settings.postProcessPrompts = [.defaultImproveTranscriptions]
        settings.postProcessSelectedPromptID = "missing"

        settings.ensurePostProcessDefaults()

        XCTAssertNil(settings.postProcessSelectedPromptID)
    }

    func testPostProcessingPromptPreparation() {
        let template = """
        Clean this:
        ${output}
        """

        let prepared = PostProcessingService.prepare(template: template, transcription: "hello")

        XCTAssertEqual(prepared.systemPrompt, "Clean this:")
        XCTAssertEqual(prepared.renderedPrompt, "Clean this:\nhello")
    }

    func testInvisibleCharactersAreStrippedFromTranscriptionText() {
        let text = "hel\u{200B}lo\u{200C}\u{200D}\u{FEFF}"

        XCTAssertEqual(PostProcessingService.stripInvisibleCharacters(text), "hello")
    }

    func testStructuredPostProcessingUsesProviderKeyAndExtractsTranscriptionField() async throws {
        let provider = PostProcessProvider(
            id: "openai",
            label: "OpenAI",
            baseURL: "https://post-processing.test/v1",
            supportsStructuredOutput: true
        )
        var settings = AppSettings.defaults
        settings.postProcessEnabled = true
        settings.postProcessProviderID = provider.id
        settings.postProcessProviders = [provider]
        settings.postProcessModels = [provider.id: "gpt-test"]
        settings.postProcessPrompts = [
            PostProcessPrompt(id: "clean", name: "Clean", prompt: "Clean this:\n${output}")
        ]
        settings.postProcessSelectedPromptID = "clean"

        let credentialStore = InMemoryPostProcessCredentialStore()
        try credentialStore.saveAPIKey("sk-test", providerID: provider.id)

        URLProtocolStub.handler = { request in
            XCTAssertEqual(request.url?.absoluteString, "https://post-processing.test/v1/chat/completions")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer sk-test")

            let body = try requestBodyData(from: request)
            let json = try XCTUnwrap(JSONSerialization.jsonObject(with: body) as? [String: Any])
            XCTAssertNotNil(json["response_format"])
            let messages = try XCTUnwrap(json["messages"] as? [[String: Any]])
            XCTAssertEqual(messages.first?["role"] as? String, "system")
            XCTAssertEqual(messages.last?["role"] as? String, "user")
            XCTAssertEqual(messages.last?["content"] as? String, "raw text")

            let response = HTTPURLResponse(
                url: try XCTUnwrap(request.url),
                statusCode: 200,
                httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            )!
            let data = """
            { "choices": [ { "message": { "content": "{\\"transcription\\":\\"cleaned\\u200B text\\"}" } } ] }
            """.data(using: .utf8)!
            return (response, data)
        }
        defer { URLProtocolStub.handler = nil }

        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [URLProtocolStub.self]
        let session = URLSession(configuration: configuration)

        let result = await PostProcessingService.postProcessTranscription(
            settings: settings,
            transcription: "raw text",
            credentialStore: credentialStore,
            urlSession: session
        )

        XCTAssertEqual(result, "cleaned text")
    }

    func testAppleIntelligencePostProcessingUsesSystemPromptWithoutAPIKey() async {
        let provider = PostProcessProvider(
            id: PostProcessProvider.appleIntelligenceProviderID,
            label: "Apple Intelligence",
            baseURL: "apple-intelligence://local",
            supportsStructuredOutput: true
        )
        var settings = AppSettings.defaults
        settings.postProcessEnabled = true
        settings.postProcessProviderID = provider.id
        settings.postProcessProviders = [provider]
        settings.postProcessModels = [provider.id: "12"]
        settings.postProcessPrompts = [
            PostProcessPrompt(id: "clean", name: "Clean", prompt: "Clean this:\n${output}")
        ]
        settings.postProcessSelectedPromptID = "clean"

        let processor = StubAppleIntelligenceProcessor(output: "cleaned\u{200B} text")

        let result = await PostProcessingService.postProcessTranscription(
            settings: settings,
            transcription: "raw text",
            credentialStore: InMemoryPostProcessCredentialStore(),
            appleIntelligenceProcessor: processor
        )

        XCTAssertEqual(result, "cleaned text")
        XCTAssertEqual(processor.systemPrompt, "Clean this:")
        XCTAssertEqual(processor.userContent, "raw text")
        XCTAssertEqual(processor.tokenLimit, 12)
    }

    func testAppleIntelligenceTokenLimitParsesOnlyPositiveNumericModelValues() {
        XCTAssertEqual(AppleIntelligenceService.tokenLimit(from: " 42 "), 42)
        XCTAssertNil(AppleIntelligenceService.tokenLimit(from: "Apple Intelligence"))
        XCTAssertNil(AppleIntelligenceService.tokenLimit(from: "0"))
        XCTAssertNil(AppleIntelligenceService.tokenLimit(from: "-1"))
        XCTAssertNil(AppleIntelligenceService.tokenLimit(from: nil))
    }

    func testAppleIntelligenceTruncatesByWordsWhenLimitIsPositive() {
        XCTAssertEqual(
            AppleIntelligenceService.truncatedText("one two three four", limit: 3),
            "one two three"
        )
        XCTAssertEqual(
            AppleIntelligenceService.truncatedText("one two", limit: 3),
            "one two"
        )
        XCTAssertEqual(
            AppleIntelligenceService.truncatedText("one two", limit: nil),
            "one two"
        )
    }

    func testPostProcessingSkipsProtectedProvidersWithoutAPIKey() async {
        let provider = PostProcessProvider(
            id: "openai",
            label: "OpenAI",
            baseURL: "https://post-processing.test/v1"
        )
        var settings = AppSettings.defaults
        settings.postProcessEnabled = true
        settings.postProcessProviderID = provider.id
        settings.postProcessProviders = [provider]
        settings.postProcessModels = [provider.id: "gpt-test"]
        settings.postProcessPrompts = [
            PostProcessPrompt(id: "clean", name: "Clean", prompt: "Clean ${output}")
        ]
        settings.postProcessSelectedPromptID = "clean"

        let result = await PostProcessingService.postProcessTranscription(
            settings: settings,
            transcription: "raw text",
            credentialStore: InMemoryPostProcessCredentialStore()
        )

        XCTAssertNil(result)
    }

    func testFetchModelsParsesOpenAIModelList() async throws {
        let provider = PostProcessProvider(
            id: "openai",
            label: "OpenAI",
            baseURL: "https://post-processing.test/v1",
            modelsEndpoint: "/models"
        )
        let credentialStore = InMemoryPostProcessCredentialStore()
        try credentialStore.saveAPIKey("sk-test", providerID: provider.id)

        URLProtocolStub.handler = { request in
            XCTAssertEqual(request.httpMethod, "GET")
            XCTAssertEqual(request.url?.absoluteString, "https://post-processing.test/v1/models")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer sk-test")

            let response = HTTPURLResponse(
                url: try XCTUnwrap(request.url),
                statusCode: 200,
                httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            )!
            let data = """
            { "data": [ { "id": "gpt-a" }, { "name": "named-model" }, { "id": "" } ] }
            """.data(using: .utf8)!
            return (response, data)
        }
        defer { URLProtocolStub.handler = nil }

        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [URLProtocolStub.self]
        let session = URLSession(configuration: configuration)

        let models = try await PostProcessingService.fetchModels(
            provider: provider,
            credentialStore: credentialStore,
            urlSession: session
        )

        XCTAssertEqual(models, ["gpt-a", "named-model"])
    }

    func testFetchModelsHTTPFailureDoesNotExposeProviderResponseBody() async throws {
        let provider = PostProcessProvider(
            id: "openai",
            label: "OpenAI",
            baseURL: "https://post-processing.test/v1",
            modelsEndpoint: "/models"
        )
        let credentialStore = InMemoryPostProcessCredentialStore()
        try credentialStore.saveAPIKey("sk-test", providerID: provider.id)

        URLProtocolStub.handler = { request in
            let response = HTTPURLResponse(
                url: try XCTUnwrap(request.url),
                statusCode: 500,
                httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            )!
            let data = #"{"error":"provider detail: prompt contains private text"}"#.data(using: .utf8)!
            return (response, data)
        }
        defer { URLProtocolStub.handler = nil }

        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [URLProtocolStub.self]
        let session = URLSession(configuration: configuration)

        do {
            _ = try await PostProcessingService.fetchModels(
                provider: provider,
                credentialStore: credentialStore,
                urlSession: session
            )
            XCTFail("Expected HTTP status failure")
        } catch let error as PostProcessingError {
            XCTAssertEqual(error.errorDescription, "Post-processing request failed with status 500.")
            XCTAssertFalse(error.localizedDescription.contains("provider detail"))
            XCTAssertFalse(error.localizedDescription.contains("private text"))
        }
    }
}

private final class StubAppleIntelligenceProcessor: AppleIntelligenceProcessing, @unchecked Sendable {
    let output: String
    private(set) var systemPrompt: String?
    private(set) var userContent: String?
    private(set) var tokenLimit: Int?

    init(output: String) {
        self.output = output
    }

    func process(
        systemPrompt: String,
        userContent: String,
        tokenLimit: Int?
    ) async throws -> String {
        self.systemPrompt = systemPrompt
        self.userContent = userContent
        self.tokenLimit = tokenLimit
        return output
    }
}

private final class InMemoryPostProcessCredentialStore: PostProcessCredentialStoring, @unchecked Sendable {
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

private final class URLProtocolStub: URLProtocol {
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

private func requestBodyData(from request: URLRequest) throws -> Data {
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
