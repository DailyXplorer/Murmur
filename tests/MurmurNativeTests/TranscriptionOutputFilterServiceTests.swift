import XCTest
@testable import MurmurNative

final class TranscriptionOutputFilterServiceTests: XCTestCase {
    func testEnglishDefaultFillersAreRemoved() {
        let result = TranscriptionOutputFilterService.filter(
            "um I think um this is good",
            language: "en",
            customFillerWords: nil
        )

        XCTAssertEqual(result, "I think this is good")
    }

    func testPortuguesePreservesUmAsRealWord() {
        let result = TranscriptionOutputFilterService.filter(
            "um gato bonito",
            language: "pt-BR",
            customFillerWords: nil
        )

        XCTAssertEqual(result, "um gato bonito")
    }

    func testSpanishPreservesHaAsRealWord() {
        let result = TranscriptionOutputFilterService.filter(
            "ha sido un buen dia",
            language: "es",
            customFillerWords: nil
        )

        XCTAssertEqual(result, "ha sido un buen dia")
    }

    func testCustomFillerWordsOverrideLanguageDefaults() {
        let result = TranscriptionOutputFilterService.filter(
            "okay so I think right this works",
            language: "en",
            customFillerWords: ["okay", "right"]
        )

        XCTAssertEqual(result, "so I think this works")
    }

    func testEmptyCustomFillerWordsDisableFillerRemoval() {
        let result = TranscriptionOutputFilterService.filter(
            "So uhm I was thinking uh about this",
            language: "en",
            customFillerWords: []
        )

        XCTAssertEqual(result, "So uhm I was thinking uh about this")
    }

    func testRepeatedAlphabeticStuttersAreCollapsed() {
        let result = TranscriptionOutputFilterService.filter(
            "wh wh wh what I I I mean",
            language: "en",
            customFillerWords: []
        )

        XCTAssertEqual(result, "wh what I mean")
    }

    func testMillimetersAndInterjectionsSurviveEnglishFiltering() {
        let result = TranscriptionOutputFilterService.filter(
            "The gap is 5 mm. Ah well, ha!",
            language: "en",
            customFillerWords: nil
        )

        XCTAssertEqual(result, "The gap is 5 mm. Ah well, ha!")
    }

    func testFillerLanguageFollowsDictationLanguage() {
        let french = TranscriptionOutputFilterService.filter(
            "euh bonjour",
            language: "fr",
            customFillerWords: nil
        )
        let english = TranscriptionOutputFilterService.filter(
            "euh bonjour",
            language: "en",
            customFillerWords: nil
        )

        XCTAssertEqual(french, "bonjour")
        XCTAssertEqual(english, "euh bonjour")
    }

    func testGenuineFillerStillRemovedWithPunctuation() {
        let result = TranscriptionOutputFilterService.filter(
            "Um, hello",
            language: "en",
            customFillerWords: nil
        )

        XCTAssertEqual(result, "hello")
    }

    func testParagraphBreaksSurviveFiltering() {
        let result = TranscriptionOutputFilterService.filter(
            "First paragraph line one.\nStill first.\n\nSecond paragraph.",
            language: "en",
            customFillerWords: nil
        )

        XCTAssertEqual(result, "First paragraph line one.\nStill first.\n\nSecond paragraph.")
    }

    func testStutterCollapseStillWorksWithinALine() {
        let result = TranscriptionOutputFilterService.filter(
            "wh wh wh what I mean\nsecond line stays",
            language: "en",
            customFillerWords: []
        )

        XCTAssertEqual(result, "wh what I mean\nsecond line stays")
    }
}
