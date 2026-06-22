import XCTest
@testable import HandyNative

final class AudioFileTranscriptionPipelineTests: XCTestCase {
    func testChineseVariantConversionIsAppliedToOutputWithoutReplacingRawTranscription() async {
        var settings = AppSettings.defaults
        settings.selectedLanguage = "zh-Hans"

        let result = await AudioFileTranscriptionPipeline.processRecognizedText(
            "繁體中文",
            settings: settings,
            credentialStore: EmptyPipelineCredentialStore(),
            postProcessRequested: false
        )

        XCTAssertEqual(result.transcriptionText, "繁體中文")
        XCTAssertEqual(result.outputText, "繁体中文")
        XCTAssertEqual(result.postProcessedText, "繁体中文")
        XCTAssertNil(result.postProcessPrompt)
    }

    func testChineseVariantOutputSurvivesWhenRequestedPostProcessingCannotRun() async {
        var settings = AppSettings.defaults
        settings.selectedLanguage = "zh-Hant"
        settings.postProcessEnabled = true
        settings.postProcessProviderID = "missing-key-provider"
        settings.postProcessProviders = [
            PostProcessProvider(
                id: "missing-key-provider",
                label: "Missing Key",
                baseURL: "https://post-processing.test/v1"
            ),
        ]
        settings.postProcessModels = ["missing-key-provider": "gpt-test"]
        settings.postProcessPrompts = [
            PostProcessPrompt(id: "clean", name: "Clean", prompt: "Clean:\n${output}")
        ]
        settings.postProcessSelectedPromptID = "clean"

        let result = await AudioFileTranscriptionPipeline.processRecognizedText(
            "汉语后发",
            settings: settings,
            credentialStore: EmptyPipelineCredentialStore(),
            postProcessRequested: true
        )

        XCTAssertEqual(result.transcriptionText, "汉语后发")
        XCTAssertEqual(result.outputText, "漢語後發")
        XCTAssertNil(result.postProcessedText)
        XCTAssertNil(result.postProcessPrompt)
    }

    func testNonChineseVariantOutputKeepsCleanedTranscription() async {
        var settings = AppSettings.defaults
        settings.selectedLanguage = "fr-FR"

        let result = await AudioFileTranscriptionPipeline.processRecognizedText(
            " bonjour\u{200B} ",
            settings: settings,
            credentialStore: EmptyPipelineCredentialStore(),
            postProcessRequested: false
        )

        XCTAssertEqual(result.transcriptionText, " bonjour ")
        XCTAssertEqual(result.outputText, "bonjour")
        XCTAssertEqual(result.postProcessedText, "bonjour")
    }

    func testOutputFilterRemovesFillersWithoutReplacingRawTranscription() async {
        var settings = AppSettings.defaults
        settings.appLanguage = "en"

        let result = await AudioFileTranscriptionPipeline.processRecognizedText(
            "um I think um this is good",
            settings: settings,
            credentialStore: EmptyPipelineCredentialStore(),
            postProcessRequested: false
        )

        XCTAssertEqual(result.transcriptionText, "um I think um this is good")
        XCTAssertEqual(result.outputText, "I think this is good")
        XCTAssertEqual(result.postProcessedText, "I think this is good")
    }

    func testCustomFillerWordsOverrideDefaultOutputFilter() async {
        var settings = AppSettings.defaults
        settings.appLanguage = "en"
        settings.customFillerWords = ["okay"]

        let result = await AudioFileTranscriptionPipeline.processRecognizedText(
            "okay um this stays",
            settings: settings,
            credentialStore: EmptyPipelineCredentialStore(),
            postProcessRequested: false
        )

        XCTAssertEqual(result.transcriptionText, "okay um this stays")
        XCTAssertEqual(result.outputText, "um this stays")
        XCTAssertEqual(result.postProcessedText, "um this stays")
    }

    func testLocalWhisperTranscriptionRequiresDownloadedModel() async throws {
        let paths = try makeTemporaryPipelinePaths()
        let audioURL = paths.recordingsDirectory.appendingPathComponent("recording.wav")
        try FileManager.default.createDirectory(at: paths.recordingsDirectory, withIntermediateDirectories: true)
        try Data("RIFFhandyWAVE".utf8).write(to: audioURL)
        var settings = AppSettings.defaults
        settings.selectTranscriptionModel(id: "base")

        do {
            _ = try await AudioFileTranscriptionPipeline.transcribe(
                fileURL: audioURL,
                settings: settings,
                paths: paths,
                credentialStore: EmptyPipelineCredentialStore(),
                postProcessRequested: false
            )
            XCTFail("Expected missing local model to fail before WhisperKit starts.")
        } catch let error as WhisperKitTranscriptionServiceError {
            XCTAssertEqual(
                error.localizedDescription,
                "Whisper Base is not downloaded yet. Download it from Models before transcribing."
            )
        }
    }

    private func makeTemporaryPipelinePaths() throws -> AppPaths {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("HandyNativePipeline-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        return AppPaths(
            appDataDirectory: root,
            recordingsDirectory: root.appendingPathComponent("recordings", isDirectory: true),
            modelsDirectory: root.appendingPathComponent("models", isDirectory: true),
            logsDirectory: root.appendingPathComponent("logs", isDirectory: true)
        )
    }
}

private struct EmptyPipelineCredentialStore: PostProcessCredentialStoring {
    func readAPIKey(providerID _: String) throws -> String? {
        nil
    }

    func saveAPIKey(_: String, providerID _: String) throws {}

    func deleteAPIKey(providerID _: String) throws {}
}
