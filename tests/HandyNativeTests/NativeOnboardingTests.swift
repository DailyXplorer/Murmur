@testable import HandyNative
import XCTest

final class NativeOnboardingTests: XCTestCase {
    func testUnknownPermissionsKeepOnboardingChecking() {
        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: .unknown,
                settings: AppSettings.defaults,
                localModelStorageStates: [:],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .checking
        )
    }

    func testMissingPermissionsShowPermissionStepAndKeepReturningUserContext() {
        var settings = AppSettings.defaults
        settings.selectedModel = "turbo"

        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: PermissionSnapshot(
                    accessibilityTrusted: false,
                    microphone: .granted,
                    speechRecognition: .unknown
                ),
                settings: settings,
                localModelStorageStates: [
                    "turbo": LocalModelStorageState(modelID: "turbo", isDownloaded: true, byteCount: 12, directories: [])
                ],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .permissions(returningUser: true)
        )
    }

    func testGrantedPermissionsWithoutUsableModelShowModelStep() {
        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: grantedPermissions,
                settings: AppSettings.defaults,
                localModelStorageStates: [:],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .model
        )
    }

    func testCompletedOnboardingSkipsModelStepWhenSelectedModelIsNotCurrentlyUsable() {
        var settings = AppSettings.defaults
        settings.nativeOnboardingCompleted = true

        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: grantedPermissions,
                settings: settings,
                localModelStorageStates: [:],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .done
        )
    }

    func testDownloadedLocalModelSkipsModelStepEvenWhenSelectedAPIKeyIsMissing() {
        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: grantedPermissions,
                settings: AppSettings.defaults,
                localModelStorageStates: [
                    "tiny": LocalModelStorageState(modelID: "tiny", isDownloaded: true, byteCount: 12, directories: [])
                ],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .done
        )
    }

    func testGrantedPermissionsWithDownloadedSelectedLocalModelCompleteOnboarding() {
        var settings = AppSettings.defaults
        settings.selectedModel = "turbo"

        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: grantedPermissions,
                settings: settings,
                localModelStorageStates: [
                    "turbo": LocalModelStorageState(modelID: "turbo", isDownloaded: true, byteCount: 12, directories: [])
                ],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .done
        )
    }

    func testGrantedPermissionsWithSelectedAPIModelRequireConfiguredKey() {
        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: grantedPermissions,
                settings: AppSettings.defaults,
                localModelStorageStates: [:],
                transcriptionAPIKeyConfigured: true,
                bypass: false
            ),
            .done
        )

        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: grantedPermissions,
                settings: AppSettings.defaults,
                localModelStorageStates: [:],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .model
        )
    }

    func testAppleSpeechRequiresSpeechRecognitionPermissionBeforeOnboardingCompletes() {
        var settings = AppSettings.defaults
        settings.selectedModel = TranscriptionAPIProvider.appleSpeechModelID

        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: PermissionSnapshot(
                    accessibilityTrusted: true,
                    microphone: .granted,
                    speechRecognition: .notDetermined
                ),
                settings: settings,
                localModelStorageStates: [:],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .permissions(returningUser: false)
        )

        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: PermissionSnapshot(
                    accessibilityTrusted: true,
                    microphone: .granted,
                    speechRecognition: .denied
                ),
                settings: settings,
                localModelStorageStates: [:],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .permissions(returningUser: false)
        )

        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: PermissionSnapshot(
                    accessibilityTrusted: true,
                    microphone: .granted,
                    speechRecognition: .granted
                ),
                settings: settings,
                localModelStorageStates: [:],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .done
        )
    }

    func testNonAppleSpeechModelsDoNotRequireSpeechRecognitionPermission() {
        var settings = AppSettings.defaults
        settings.selectedModel = "turbo"

        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: PermissionSnapshot(
                    accessibilityTrusted: true,
                    microphone: .granted,
                    speechRecognition: .denied
                ),
                settings: settings,
                localModelStorageStates: [
                    "turbo": LocalModelStorageState(modelID: "turbo", isDownloaded: true, byteCount: 12, directories: [])
                ],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .done
        )
    }

    func testCompletedOnboardingStillRequiresMissingPermissions() {
        var settings = AppSettings.defaults
        settings.nativeOnboardingCompleted = true

        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: PermissionSnapshot(
                    accessibilityTrusted: false,
                    microphone: .granted,
                    speechRecognition: .unknown
                ),
                settings: settings,
                localModelStorageStates: [:],
                transcriptionAPIKeyConfigured: false,
                bypass: false
            ),
            .permissions(returningUser: false)
        )
    }

    func testBypassCompletesOnboardingForSmokeAndVisualRoutes() {
        XCTAssertEqual(
            NativeOnboardingEvaluator.nextStep(
                permissionSnapshot: .unknown,
                settings: AppSettings.defaults,
                localModelStorageStates: [:],
                transcriptionAPIKeyConfigured: false,
                bypass: true
            ),
            .done
        )
    }

    private var grantedPermissions: PermissionSnapshot {
        PermissionSnapshot(
            accessibilityTrusted: true,
            microphone: .granted,
            speechRecognition: .unknown
        )
    }
}
