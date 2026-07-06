@testable import MurmurNative
import XCTest

final class MurmurDesignTests: XCTestCase {
    func testMainWindowMetricsMatchCurrentNativeWindow() {
        XCTAssertEqual(MurmurDesign.windowWidth, 860)
        XCTAssertEqual(MurmurDesign.windowHeight, 640)
        XCTAssertEqual(MurmurDesign.sidebarWidth, 160)
    }
}
