@testable import MurmurNative
import XCTest

final class SettingsPermissionBannerModelTests: XCTestCase {
    func testBannerOnlyRequiresAccessibilityForTypingPermission() {
        let model = SettingsPermissionBannerModel.make(
            snapshot: PermissionSnapshot(
                accessibilityTrusted: false,
                microphone: .granted,
                speechRecognition: .granted
            )
        )

        XCTAssertEqual(
            model,
            SettingsPermissionBannerModel(
                message: "Murmur needs accessibility permissions to type transcribed text.",
                buttonTitle: "Open Settings"
            )
        )
    }

    func testBannerDoesNotShowForMicrophoneOrSpeechOnlyPermissionGaps() {
        XCTAssertNil(
            SettingsPermissionBannerModel.make(
                snapshot: PermissionSnapshot(
                    accessibilityTrusted: true,
                    microphone: .denied,
                    speechRecognition: .denied
                )
            )
        )
    }
}
