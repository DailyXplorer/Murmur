@testable import HandyNative
import Foundation
import XCTest

final class MenuBarModelOptionsTests: XCTestCase {
    func testMenuIncludesAppleSpeechAPIModelsAndDownloadedLocalModels() {
        var settings = AppSettings.defaults
        settings.selectTranscriptionModel(id: TranscriptionAPIProvider.appleSpeechModelID)

        let options = MenuBarModelOptions.make(
            settings: settings,
            localModelStorageStates: [
                "tiny": LocalModelStorageState(
                    modelID: "tiny",
                    isDownloaded: true,
                    byteCount: 12,
                    directories: []
                )
            ]
        )

        XCTAssertEqual(options.first?.id, TranscriptionAPIProvider.appleSpeechModelID)
        XCTAssertTrue(options.contains { $0.id == "tiny" && $0.title == "Whisper Tiny" && $0.isEnabled })
        XCTAssertTrue(options.contains { $0.id == TranscriptionAPIProvider.mistralVoxtralModelID && $0.title == "Mistral Voxtral Small" })
        XCTAssertTrue(options.first { $0.id == TranscriptionAPIProvider.appleSpeechModelID }?.isSelected == true)
    }

    func testMenuHidesUndownloadedLocalModelsUnlessSelected() {
        var settings = AppSettings.defaults
        settings.selectTranscriptionModel(id: "base")

        let options = MenuBarModelOptions.make(
            settings: settings,
            localModelStorageStates: [:]
        )

        XCTAssertTrue(options.contains { $0.id == "base" && $0.isSelected && !$0.isEnabled })
        XCTAssertFalse(options.contains { $0.id == "tiny" })
    }

    func testMenuPreservesUnknownSelectionAsDisabledActiveItem() {
        var settings = AppSettings.defaults
        settings.selectedModel = "parakeet-tdt-0.6b-v3"

        let options = MenuBarModelOptions.make(
            settings: settings,
            localModelStorageStates: [:]
        )

        XCTAssertEqual(options.first?.id, "parakeet-tdt-0.6b-v3")
        XCTAssertEqual(options.first?.title, "parakeet-tdt-0.6b-v3")
        XCTAssertTrue(options.first?.isSelected == true)
        XCTAssertFalse(options.first?.isEnabled == true)
    }
}
