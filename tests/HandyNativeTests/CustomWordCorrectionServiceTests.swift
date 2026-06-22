@testable import HandyNative
import XCTest

final class CustomWordCorrectionServiceTests: XCTestCase {
    func testExactMatchesPreserveConfiguredCapitalization() {
        let result = CustomWordCorrectionService.applyCustomWords(
            to: "hello world",
            customWords: ["Hello", "World"],
            threshold: 0.5
        )

        XCTAssertEqual(result, "Hello World")
    }

    func testFuzzyMatchesUseThreshold() {
        let result = CustomWordCorrectionService.applyCustomWords(
            to: "helo wrold",
            customWords: ["hello", "world"],
            threshold: 0.5
        )

        XCTAssertEqual(result, "hello world")
    }

    func testNGramCorrectionsPreserveTrailingPunctuation() {
        let result = CustomWordCorrectionService.applyCustomWords(
            to: "il cui nome e Charge B, che permette",
            customWords: ["ChargeBee"],
            threshold: 0.5
        )

        XCTAssertTrue(result.contains("ChargeBee,"))
        XCTAssertFalse(result.contains("Charge B"))
    }

    func testThreeWordNGramCorrection() {
        let result = CustomWordCorrectionService.applyCustomWords(
            to: "use Chat G P T for this",
            customWords: ["ChatGPT"],
            threshold: 0.5
        )

        XCTAssertTrue(result.contains("ChatGPT"))
    }

    func testImportedCustomWordsCanContainSpaces() {
        let words = AppSettings.normalizedCustomWordsForImport([" MacBook Pro ", "MacBook Pro"])
        let result = CustomWordCorrectionService.applyCustomWords(
            to: "using Mac Book Pro",
            customWords: words,
            threshold: 0.5
        )

        XCTAssertEqual(words, ["MacBook Pro"])
        XCTAssertTrue(result.contains("MacBook Pro"))
    }

    func testUIWordSanitizerValidation() {
        XCTAssertEqual(AppSettings.sanitizeCustomWord("  ChargeBee  "), "ChargeBee")
        XCTAssertEqual(AppSettings.sanitizeCustomWord("<ChargeBee>"), "ChargeBee")
        XCTAssertNil(AppSettings.sanitizeCustomWord("MacBook Pro"))
        XCTAssertNil(AppSettings.sanitizeCustomWord(String(repeating: "a", count: 51)))
    }
}
