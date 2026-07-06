@testable import MurmurNative
import AppKit
import XCTest

final class MurmurDesignTests: XCTestCase {
    func testMainWindowMetricsMatchCurrentNativeWindow() {
        XCTAssertEqual(MurmurDesign.windowWidth, 860)
        XCTAssertEqual(MurmurDesign.windowHeight, 640)
        XCTAssertEqual(MurmurDesign.sidebarWidth, 160)
    }

    func testSidebarLogoMetricsUseAvailableSidebarWidth() {
        XCTAssertEqual(MurmurDesign.sidebarLogoWidth, MurmurDesign.sidebarWidth - 16)
        XCTAssertEqual(MurmurDesign.sidebarLogoHeight, 44)
    }

    func testMurmurTextLogoAssetIsTightlyCropped() throws {
        let fileURL = URL(fileURLWithPath: #filePath)
        let packageRoot = fileURL
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let logoURL = packageRoot.appendingPathComponent("Resources/MurmurTextLogo.png")
        let image = try XCTUnwrap(NSImage(contentsOf: logoURL))

        XCTAssertLessThanOrEqual(image.size.width, 1_220)
        XCTAssertLessThanOrEqual(image.size.height, 360)
        XCTAssertGreaterThan(image.size.width / image.size.height, 3.3)
    }
}
