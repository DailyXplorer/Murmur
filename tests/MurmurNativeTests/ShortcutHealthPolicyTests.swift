import XCTest
@testable import MurmurNative

final class ShortcutHealthPolicyTests: XCTestCase {
    func testHealthyInstalledTrustedIdleRequiresNoAction() {
        XCTAssertEqual(
            ShortcutHealthPolicy.assess(
                accessibilityTrusted: true,
                tapInstalled: true,
                tapHealthy: true,
                secureInputActive: false,
                recordingActive: false
            ),
            .none
        )
    }

    func testDeadTapRequestsReinstall() {
        XCTAssertEqual(
            ShortcutHealthPolicy.assess(
                accessibilityTrusted: true,
                tapInstalled: true,
                tapHealthy: false,
                secureInputActive: false,
                recordingActive: false
            ),
            .reinstall
        )
    }

    func testMissingTapWhileTrustedRequestsInstall() {
        XCTAssertEqual(
            ShortcutHealthPolicy.assess(
                accessibilityTrusted: true,
                tapInstalled: false,
                tapHealthy: false,
                secureInputActive: false,
                recordingActive: false
            ),
            .install
        )
    }

    func testRevokedTrustWithInstalledTapRequestsTeardownAndWarn() {
        XCTAssertEqual(
            ShortcutHealthPolicy.assess(
                accessibilityTrusted: false,
                tapInstalled: true,
                tapHealthy: true,
                secureInputActive: false,
                recordingActive: false
            ),
            .teardownAndWarn
        )
    }

    func testRevokedTrustWithoutTapRequiresNoAction() {
        XCTAssertEqual(
            ShortcutHealthPolicy.assess(
                accessibilityTrusted: false,
                tapInstalled: false,
                tapHealthy: false,
                secureInputActive: false,
                recordingActive: false
            ),
            .none
        )
    }

    func testSecureInputWarnsWhetherOrNotTapIsInstalled() {
        XCTAssertEqual(
            ShortcutHealthPolicy.assess(
                accessibilityTrusted: true,
                tapInstalled: true,
                tapHealthy: true,
                secureInputActive: true,
                recordingActive: false
            ),
            .warnSecureInput
        )
        XCTAssertEqual(
            ShortcutHealthPolicy.assess(
                accessibilityTrusted: true,
                tapInstalled: false,
                tapHealthy: false,
                secureInputActive: true,
                recordingActive: false
            ),
            .warnSecureInput
        )
    }

    func testActiveRecordingAlwaysRequiresNoAction() {
        let combinations: [(trusted: Bool, installed: Bool, healthy: Bool, secureInput: Bool)] = [
            (true, true, true, false),
            (true, true, false, false),
            (true, false, false, false),
            (false, true, true, false),
            (false, false, false, false),
            (true, true, true, true),
            (false, true, false, true),
        ]

        for combination in combinations {
            XCTAssertEqual(
                ShortcutHealthPolicy.assess(
                    accessibilityTrusted: combination.trusted,
                    tapInstalled: combination.installed,
                    tapHealthy: combination.healthy,
                    secureInputActive: combination.secureInput,
                    recordingActive: true
                ),
                .none,
                "Expected .none while recording for combination \(combination)"
            )
        }
    }
}
