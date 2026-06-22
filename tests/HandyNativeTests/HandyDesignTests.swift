@testable import HandyNative
import XCTest

final class HandyDesignTests: XCTestCase {
    func testMainWindowMetricsMatchCurrentNativeWindow() {
        XCTAssertEqual(HandyDesign.windowWidth, 860)
        XCTAssertEqual(HandyDesign.windowHeight, 640)
        XCTAssertEqual(HandyDesign.sidebarWidth, 160)
    }
}
