@testable import HandyNative
import XCTest

final class NativeSmokeTranscriptionSettingsResolverTests: XCTestCase {
    func testLocalModelIDBuildsDefaultLocalSettings() async throws {
        let configuration = try await NativeSmokeTranscriptionSettingsResolver.configuration(
            modelID: "base",
            language: "fr",
            useSelectedSettings: false,
            paths: temporaryPaths()
        )

        XCTAssertEqual(configuration.settings.selectedModel, "base")
        XCTAssertEqual(configuration.settings.selectedTranscriptionModelDisplayName, "Whisper Base")
        XCTAssertEqual(configuration.settings.selectedLanguage, "fr")
        XCTAssertNil(configuration.appleSpeechTranscriptionService)
    }

    func testAppleSpeechModelIDBuildsAppleSpeechSettings() async throws {
        let configuration = try await NativeSmokeTranscriptionSettingsResolver.configuration(
            modelID: TranscriptionAPIProvider.appleSpeechModelID,
            language: "en-US",
            useSelectedSettings: false,
            paths: temporaryPaths()
        )

        XCTAssertEqual(configuration.settings.selectedModel, TranscriptionAPIProvider.appleSpeechModelID)
        XCTAssertEqual(configuration.settings.selectedTranscriptionModelDisplayName, "Apple Speech")
        XCTAssertEqual(configuration.settings.selectedLanguage, "en-US")
        XCTAssertNotNil(configuration.appleSpeechTranscriptionService)
    }

    func testUnknownModelIDThrowsUnsupportedModel() async throws {
        do {
            _ = try await NativeSmokeTranscriptionSettingsResolver.configuration(
                modelID: "parakeet-tdt-0.6b-v3",
                language: nil,
                useSelectedSettings: false,
                paths: temporaryPaths()
            )
            XCTFail("Expected unsupported model to throw")
        } catch let error as NativeSmokeTranscriptionSettingsError {
            XCTAssertEqual(error.localizedDescription, "Transcription model 'parakeet-tdt-0.6b-v3' is not available in the native Swift smoke runner.")
        }
    }

    private func temporaryPaths() -> AppPaths {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("handy-smoke-settings-\(UUID().uuidString)", isDirectory: true)
        return AppPaths(
            appDataDirectory: root,
            recordingsDirectory: root.appendingPathComponent("recordings", isDirectory: true),
            modelsDirectory: root.appendingPathComponent("models", isDirectory: true),
            logsDirectory: root.appendingPathComponent("logs", isDirectory: true)
        )
    }
}
