import XCTest
@testable import MurmurNative

final class TranscriptionOutputFilterServiceTests: XCTestCase {
    func testEnglishDefaultFillersAreRemoved() {
        let result = TranscriptionOutputFilterService.filter(
            "um I think um this is good",
            appLanguage: "en",
            customFillerWords: nil
        )

        XCTAssertEqual(result, "I think this is good")
    }

    func testPortuguesePreservesUmAsRealWord() {
        let result = TranscriptionOutputFilterService.filter(
            "um gato bonito",
            appLanguage: "pt-BR",
            customFillerWords: nil
        )

        XCTAssertEqual(result, "um gato bonito")
    }

    func testSpanishPreservesHaAsRealWord() {
        let result = TranscriptionOutputFilterService.filter(
            "ha sido un buen dia",
            appLanguage: "es",
            customFillerWords: nil
        )

        XCTAssertEqual(result, "ha sido un buen dia")
    }

    func testCustomFillerWordsOverrideLanguageDefaults() {
        let result = TranscriptionOutputFilterService.filter(
            "okay so I think right this works",
            appLanguage: "en",
            customFillerWords: ["okay", "right"]
        )

        XCTAssertEqual(result, "so I think this works")
    }

    func testEmptyCustomFillerWordsDisableFillerRemoval() {
        let result = TranscriptionOutputFilterService.filter(
            "So uhm I was thinking uh about this",
            appLanguage: "en",
            customFillerWords: []
        )

        XCTAssertEqual(result, "So uhm I was thinking uh about this")
    }

    func testRepeatedAlphabeticStuttersAreCollapsed() {
        let result = TranscriptionOutputFilterService.filter(
            "wh wh wh what I I I mean",
            appLanguage: "en",
            customFillerWords: []
        )

        XCTAssertEqual(result, "wh what I mean")
    }
}
