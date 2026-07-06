import XCTest
@testable import MurmurNative

final class ChineseVariantConversionServiceTests: XCTestCase {
    func testSimplifiedChineseSelectionConvertsTraditionalCharacters() {
        XCTAssertEqual(
            ChineseVariantConversionService.convertedText(
                "繁體中文，後臺發展重複。",
                selectedLanguage: "zh-Hans"
            ),
            "繁体中文，后台发展重复。"
        )
    }

    func testTraditionalChineseSelectionConvertsSimplifiedCharacters() {
        XCTAssertEqual(
            ChineseVariantConversionService.convertedText(
                "汉语后发，鼠标重复。",
                selectedLanguage: "zh-Hant"
            ),
            "漢語後發，鼠標重復。"
        )
    }

    func testNonChineseVariantSelectionDoesNotConvert() {
        XCTAssertNil(
            ChineseVariantConversionService.convertedText(
                "繁體中文",
                selectedLanguage: "zh"
            )
        )
        XCTAssertNil(
            ChineseVariantConversionService.convertedText(
                "繁體中文",
                selectedLanguage: "auto"
            )
        )
    }
}
