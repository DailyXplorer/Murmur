@testable import HandyNative
import XCTest

final class SystemAudioMuteServiceTests: XCTestCase {
    func testAppleScriptMatchesMacOSMuteCommand() {
        XCTAssertEqual(
            SystemAudioMuteService.appleScript(muted: true),
            "set volume output muted true"
        )
        XCTAssertEqual(
            SystemAudioMuteService.appleScript(muted: false),
            "set volume output muted false"
        )
    }
}
