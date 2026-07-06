import AppKit
@testable import MurmurNative
import XCTest

final class AppLaunchConfigurationTests: XCTestCase {
    func testNormalLaunchUsesRegularActivationAndActivatesApp() {
        let configuration = AppLaunchConfiguration(startHidden: false, showMenuBarIcon: true)

        XCTAssertEqual(configuration.activationPolicy, .regular)
        XCTAssertTrue(configuration.shouldActivateOnLaunch)
        XCTAssertFalse(configuration.shouldHideOnLaunch)
    }

    func testHiddenLaunchWithMenuBarIconUsesAccessoryActivation() {
        let configuration = AppLaunchConfiguration(startHidden: true, showMenuBarIcon: true)

        XCTAssertEqual(configuration.activationPolicy, .accessory)
        XCTAssertFalse(configuration.shouldActivateOnLaunch)
        XCTAssertTrue(configuration.shouldHideOnLaunch)
    }

    func testStartHiddenArgumentOverridesStoredLaunchSetting() {
        let configuration = AppLaunchConfiguration(
            settings: .defaults,
            launchArguments: NativeLaunchArguments.parse(["Murmur", "--start-hidden"])
        )

        XCTAssertEqual(configuration.activationPolicy, .accessory)
        XCTAssertFalse(configuration.shouldActivateOnLaunch)
        XCTAssertTrue(configuration.shouldHideOnLaunch)
    }

    func testNoTrayArgumentDisablesAccessoryActivationAtRuntime() {
        var settings = AppSettings.defaults
        settings.startHidden = true
        settings.showMenuBarIcon = true

        let configuration = AppLaunchConfiguration(
            settings: settings,
            launchArguments: NativeLaunchArguments.parse(["Murmur", "--no-tray"])
        )

        XCTAssertEqual(configuration.activationPolicy, .regular)
        XCTAssertFalse(configuration.showMenuBarIcon)
    }

    func testHiddenLaunchWithoutMenuBarIconKeepsDockRecoveryPath() {
        let configuration = AppLaunchConfiguration(startHidden: true, showMenuBarIcon: false)

        XCTAssertEqual(configuration.activationPolicy, .regular)
        XCTAssertFalse(configuration.shouldActivateOnLaunch)
        XCTAssertTrue(configuration.shouldHideOnLaunch)
    }
}
