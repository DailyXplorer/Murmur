@testable import MurmurNative
import XCTest

final class NativePermissionStatusSmokeRunnerTests: XCTestCase {
    func testPermissionStatusOutputReportsPassiveReadinessBooleans() {
        let output = NativePermissionStatusSmokeRunner.output(
            for: PermissionSnapshot(
                accessibilityTrusted: true,
                microphone: .granted,
                speechRecognition: .denied
            )
        )

        XCTAssertEqual(
            output,
            NativePermissionStatusSmokeOutput(
                success: true,
                prompted: false,
                accessibilityTrusted: true,
                microphone: "granted",
                speechRecognition: "denied",
                shortcutReady: true,
                pasteKeyboardReady: true,
                microphoneReady: true,
                appleSpeechReady: false,
                liveDictationPrerequisitesReady: true
            )
        )
    }

    func testPermissionStatusOutputRequiresAccessibilityAndMicrophoneForLiveDictation() {
        let output = NativePermissionStatusSmokeRunner.output(
            for: PermissionSnapshot(
                accessibilityTrusted: false,
                microphone: .granted,
                speechRecognition: .granted
            )
        )

        XCTAssertFalse(output.shortcutReady)
        XCTAssertFalse(output.pasteKeyboardReady)
        XCTAssertTrue(output.microphoneReady)
        XCTAssertTrue(output.appleSpeechReady)
        XCTAssertFalse(output.liveDictationPrerequisitesReady)
    }

    func testPermissionStatusOutputEncodesJSONContract() throws {
        let output = NativePermissionStatusSmokeOutput(
            success: true,
            prompted: false,
            accessibilityTrusted: true,
            microphone: "granted",
            speechRecognition: "notDetermined",
            shortcutReady: true,
            pasteKeyboardReady: true,
            microphoneReady: true,
            appleSpeechReady: false,
            liveDictationPrerequisitesReady: true
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["success"] as? Bool, true)
        XCTAssertEqual(object["prompted"] as? Bool, false)
        XCTAssertEqual(object["accessibilityTrusted"] as? Bool, true)
        XCTAssertEqual(object["microphone"] as? String, "granted")
        XCTAssertEqual(object["speechRecognition"] as? String, "notDetermined")
        XCTAssertEqual(object["shortcutReady"] as? Bool, true)
        XCTAssertEqual(object["pasteKeyboardReady"] as? Bool, true)
        XCTAssertEqual(object["microphoneReady"] as? Bool, true)
        XCTAssertEqual(object["appleSpeechReady"] as? Bool, false)
        XCTAssertEqual(object["liveDictationPrerequisitesReady"] as? Bool, true)
    }
}
