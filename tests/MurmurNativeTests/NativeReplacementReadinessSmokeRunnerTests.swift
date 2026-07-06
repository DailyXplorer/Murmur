@testable import MurmurNative
import XCTest

final class NativeReplacementReadinessSmokeRunnerTests: XCTestCase {
    func testReplacementReadinessAllowsRunnableCurrentModel() {
        var settings = AppSettings.defaults
        settings.selectedModel = "tiny"
        let output = NativeReplacementReadinessSmokeRunner.output(
            settings: settings,
            permissionSnapshot: PermissionSnapshot(
                accessibilityTrusted: true,
                microphone: .granted,
                speechRecognition: .granted
            ),
            hasAPIKey: { _ in false }
        )

        XCTAssertTrue(output.success)
        XCTAssertTrue(output.replacementReady)
        XCTAssertEqual(output.selectedModelKind, "native_local_whisper")
        XCTAssertTrue(output.selectedModelRunnable)
        XCTAssertNil(output.selectedModelBlockingIssue)
        XCTAssertTrue(output.liveDictationPrerequisitesReady)
        XCTAssertTrue(output.appleSpeechReady)
        XCTAssertEqual(output.nativeLocalModelIDs, ["base", "large", "medium", "small", "tiny", "turbo"])
        XCTAssertTrue(output.blockingIssues.isEmpty)
        XCTAssertTrue(output.warnings.isEmpty)
    }

    func testReplacementReadinessBlocksUnknownSelectedModel() {
        var settings = AppSettings.defaults
        settings.selectedModel = "moonshine-base"
        let output = NativeReplacementReadinessSmokeRunner.output(
            settings: settings,
            permissionSnapshot: PermissionSnapshot(
                accessibilityTrusted: true,
                microphone: .granted,
                speechRecognition: .notDetermined
            ),
            hasAPIKey: { _ in false }
        )

        XCTAssertFalse(output.success)
        XCTAssertEqual(output.selectedModelDisplayName, "moonshine-base")
        XCTAssertEqual(output.selectedModelKind, "unknown")
        XCTAssertFalse(output.selectedModelRunnable)
        XCTAssertEqual(
            output.selectedModelBlockingIssue,
            "Selected model is unknown to the native Swift app."
        )
        XCTAssertTrue(output.blockingIssues.contains(output.selectedModelBlockingIssue ?? ""))
        XCTAssertTrue(
            output.warnings.contains("Apple Speech is available only after Speech Recognition permission is granted.")
        )
    }

    func testReplacementReadinessRequiresAPIKeyForSelectedAPIModel() {
        var settings = AppSettings.defaults
        settings.transcriptionAPIProviders = [
            TranscriptionAPIProvider(
                id: "provider",
                label: "Provider",
                baseURL: "https://example.com/v1",
                requiresAPIKey: true
            ),
        ]
        settings.transcriptionAPIModels = [
            TranscriptionAPIModel(
                id: "provider-model",
                providerID: "provider",
                modelID: "provider-model",
                displayName: "Provider Model",
                description: "",
                isCustom: false
            ),
        ]
        settings.selectedModel = "provider-model"

        let outputWithoutKey = NativeReplacementReadinessSmokeRunner.output(
            settings: settings,
            permissionSnapshot: PermissionSnapshot(
                accessibilityTrusted: true,
                microphone: .granted,
                speechRecognition: .granted
            ),
            hasAPIKey: { _ in false }
        )
        let outputWithKey = NativeReplacementReadinessSmokeRunner.output(
            settings: settings,
            permissionSnapshot: PermissionSnapshot(
                accessibilityTrusted: true,
                microphone: .granted,
                speechRecognition: .granted
            ),
            hasAPIKey: { $0 == "provider" }
        )

        XCTAssertEqual(outputWithoutKey.selectedModelKind, "native_api")
        XCTAssertFalse(outputWithoutKey.selectedModelRunnable)
        XCTAssertEqual(outputWithoutKey.selectedModelBlockingIssue, "Selected API transcription model requires an API key.")
        XCTAssertTrue(outputWithKey.selectedModelRunnable)
        XCTAssertNil(outputWithKey.selectedModelBlockingIssue)
        XCTAssertTrue(outputWithKey.replacementReady)
    }

    func testReplacementReadinessOutputEncodesJSONContract() throws {
        let output = NativeReplacementReadinessSmokeOutput(
            success: false,
            replacementReady: false,
            prompted: false,
            selectedModelID: "tiny",
            selectedModelDisplayName: "Whisper Tiny",
            selectedModelKind: "native_local_whisper",
            selectedModelRunnable: true,
            selectedModelBlockingIssue: nil,
            accessibilityTrusted: true,
            microphone: "granted",
            speechRecognition: "notDetermined",
            liveDictationPrerequisitesReady: true,
            appleSpeechReady: false,
            nativeLocalModelIDs: ["tiny"],
            nativeTranscriptionPaths: ["whisperkit_coreml"],
            blockingIssues: [],
            warnings: [
                "Apple Speech is available only after Speech Recognition permission is granted.",
            ]
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]

        let data = try encoder.encode(output)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["success"] as? Bool, false)
        XCTAssertEqual(object["replacementReady"] as? Bool, false)
        XCTAssertEqual(object["selectedModelID"] as? String, "tiny")
        XCTAssertEqual(object["selectedModelKind"] as? String, "native_local_whisper")
        XCTAssertEqual(object["selectedModelRunnable"] as? Bool, true)
        XCTAssertEqual(object["liveDictationPrerequisitesReady"] as? Bool, true)
        XCTAssertNil(object["selectedModelBlockingIssue"])
    }
}
