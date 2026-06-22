@testable import HandyNative
import XCTest

final class LastErrorBannerModelTests: XCTestCase {
    func testBannerTrimsErrorMessage() {
        XCTAssertEqual(
            LastErrorBannerModel.make(message: "  Microphone access failed.\n"),
            LastErrorBannerModel(message: "Microphone access failed.")
        )
    }

    func testBannerDoesNotShowForMissingOrBlankMessage() {
        XCTAssertNil(LastErrorBannerModel.make(message: nil))
        XCTAssertNil(LastErrorBannerModel.make(message: " \n\t "))
    }
}
