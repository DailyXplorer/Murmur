@testable import MurmurNative
import XCTest

final class LaunchAtLoginServiceTests: XCTestCase {
    func testStoredEnabledSettingOnlyAppliesWhenSystemStateDiffers() {
        XCTAssertFalse(LaunchAtLoginStatus.enabled.shouldApplyStoredSetting(true))
        XCTAssertFalse(LaunchAtLoginStatus.requiresApproval.shouldApplyStoredSetting(true))
        XCTAssertTrue(LaunchAtLoginStatus.disabled.shouldApplyStoredSetting(true))
        XCTAssertTrue(LaunchAtLoginStatus.notFound.shouldApplyStoredSetting(true))
    }

    func testStoredDisabledSettingOnlyAppliesWhenSystemStateIsEnabledOrPending() {
        XCTAssertTrue(LaunchAtLoginStatus.enabled.shouldApplyStoredSetting(false))
        XCTAssertTrue(LaunchAtLoginStatus.requiresApproval.shouldApplyStoredSetting(false))
        XCTAssertFalse(LaunchAtLoginStatus.disabled.shouldApplyStoredSetting(false))
        XCTAssertFalse(LaunchAtLoginStatus.notFound.shouldApplyStoredSetting(false))
        XCTAssertFalse(LaunchAtLoginStatus.unavailable.shouldApplyStoredSetting(false))
    }

    func testStatusLabelsAreCompactForAdvancedSettingsRows() {
        XCTAssertEqual(LaunchAtLoginStatus.enabled.userFacingLabel, "Enabled")
        XCTAssertEqual(LaunchAtLoginStatus.disabled.userFacingLabel, "Off")
        XCTAssertEqual(LaunchAtLoginStatus.requiresApproval.userFacingLabel, "Approval required")
        XCTAssertEqual(LaunchAtLoginStatus.unknown("").userFacingLabel, "Unknown")
    }
}
