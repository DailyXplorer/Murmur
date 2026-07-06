@testable import MurmurNative
import XCTest

final class AppThemeTests: XCTestCase {
    func testThemeOrderMatchesCurrentMurmurSelector() {
        XCTAssertEqual(AppTheme.allCases, [.pink, .blue, .green, .purple, .orange, .gray])
    }

    func testThemePaletteMatchesCurrentMurmurTokens() {
        XCTAssertEqual(AppTheme.pink.palette.primaryHex, 0xFAA2CA)
        XCTAssertEqual(AppTheme.pink.palette.primaryDarkHex, 0xF28CBB)
        XCTAssertEqual(AppTheme.pink.palette.uiHex, 0xDA5893)
        XCTAssertEqual(AppTheme.pink.palette.strokeHex, 0x382731)
        XCTAssertEqual(AppTheme.pink.palette.strokeDarkHex, 0xFAD1ED)
        XCTAssertEqual(AppTheme.pink.palette.textStrokeHex, 0xF6F6F6)
        XCTAssertEqual(AppTheme.pink.palette.overlayBarHex, 0xFFE5EE)

        XCTAssertEqual(AppTheme.blue.palette.primaryHex, 0x7CB7FF)
        XCTAssertEqual(AppTheme.green.palette.uiHex, 0x239B6D)
        XCTAssertEqual(AppTheme.purple.palette.overlayBarHex, 0xEEE7FF)
        XCTAssertEqual(AppTheme.orange.palette.strokeHex, 0x4A2A16)
        XCTAssertEqual(AppTheme.gray.palette.strokeDarkHex, 0xE0E3E9)
    }

    func testUnknownThemeDecodesAsPink() throws {
        let data = #""unknown-theme""#.data(using: .utf8)!

        let theme = try JSONDecoder().decode(AppTheme.self, from: data)

        XCTAssertEqual(theme, .pink)
    }
}
