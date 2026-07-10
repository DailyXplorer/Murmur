import Foundation
@testable import MurmurNative
import XCTest

// Shared fakes and harness for AppModel recording-flow characterization tests.
// Modeled after the private fakes in RecordingWorkflowTests.swift.

struct FlowFakeError: LocalizedError {
    var message: String

    var errorDescription: String? {
        message
    }
}

@MainActor
final class FlowFakeAudioCaptureService: AudioCaptureServicing {
    var isRecording = false
    var voiceProcessingStatus: AudioInputVoiceProcessingStatus = .notConfigured
    var stopResult = AudioRecording.silentFlowFixture()
    var startError: Error?
    var stopError: Error?
    private(set) var startCallCount = 0
    private(set) var stopCallCount = 0
    private(set) var cancelCallCount = 0

    func start(
        selectedMicrophoneName _: String?,
        voiceProcessingConfiguration _: AudioInputVoiceProcessingConfiguration,
        onLevel _: @escaping @Sendable (Float) -> Void
    ) throws {
        startCallCount += 1
        if let startError {
            throw startError
        }
        isRecording = true
    }

    func stop(keepStreamOpen _: Bool, lazyClose _: Bool) throws -> AudioRecording {
        stopCallCount += 1
        isRecording = false
        if let stopError {
            throw stopError
        }
        return stopResult
    }

    func cancel(keepStreamOpen _: Bool, lazyClose _: Bool) {
        cancelCallCount += 1
        isRecording = false
    }

    func openIdleStream(
        selectedMicrophoneName _: String?,
        voiceProcessingConfiguration _: AudioInputVoiceProcessingConfiguration
    ) throws {}

    func closeIdleStream() {}
}

@MainActor
final class FlowFakeAudioFeedbackService: AudioFeedbackPlaying {
    private(set) var playedSounds: [AudioFeedbackSound] = []

    func play(_ sound: AudioFeedbackSound, settings _: AppSettings, paths _: AppPaths) {
        playedSounds.append(sound)
    }
}

@MainActor
final class FlowFakeSystemAudioMuteService: SystemAudioMuting {
    private(set) var applyCallCount = 0
    private(set) var removeCallCount = 0

    func applyMuteIfNeeded(settings _: AppSettings) {
        applyCallCount += 1
    }

    func removeMuteIfNeeded() {
        removeCallCount += 1
    }
}

@MainActor
final class FlowFakePasteService: PasteServicing {
    private(set) var pastedTexts: [String] = []
    var pasteError: Error?

    func paste(_ rawText: String, options _: PasteOutputOptions) async throws {
        if let pasteError {
            throw pasteError
        }
        pastedTexts.append(rawText)
    }
}

final class FlowFakeLaunchAtLoginService: LaunchAtLoginServicing {
    func currentStatus() -> LaunchAtLoginStatus {
        .disabled
    }

    func setEnabled(_: Bool) throws {}
}

extension AudioRecording {
    /// Passes `hasAudibleSignal`; large enough to survive trimming/padding.
    static func audibleFlowFixture() -> AudioRecording {
        AudioRecording(
            samples: Array(repeating: 0.5, count: 32_000),
            sampleRate: 16_000,
            startedAt: Date(),
            endedAt: Date()
        )
    }

    /// Fails `hasAudibleSignal`: triggers the "No speech detected" discard path.
    static func silentFlowFixture() -> AudioRecording {
        AudioRecording(
            samples: Array(repeating: 0, count: 32_000),
            sampleRate: 16_000,
            startedAt: Date(),
            endedAt: Date()
        )
    }
}

/// Everything a recording-flow test needs: the AppModel under test, its fakes,
/// and the isolated data directory (MURMUR_APP_DATA_DIR) for teardown.
@MainActor
final class RecordingFlowTestContext {
    let appModel: AppModel
    let capture: FlowFakeAudioCaptureService
    let feedback: FlowFakeAudioFeedbackService
    let mute: FlowFakeSystemAudioMuteService
    let paste: FlowFakePasteService
    let dataDirectory: URL
    private let previousAppDataDirectory: String?

    init(
        appModel: AppModel,
        capture: FlowFakeAudioCaptureService,
        feedback: FlowFakeAudioFeedbackService,
        mute: FlowFakeSystemAudioMuteService,
        paste: FlowFakePasteService,
        dataDirectory: URL,
        previousAppDataDirectory: String?
    ) {
        self.appModel = appModel
        self.capture = capture
        self.feedback = feedback
        self.mute = mute
        self.paste = paste
        self.dataDirectory = dataDirectory
        self.previousAppDataDirectory = previousAppDataDirectory
    }

    /// All .wav files written anywhere under the isolated data directory
    /// (recordings land in `<dataDirectory>/com.pais.murmur/recordings`).
    func recordedWAVFiles() -> [URL] {
        guard let enumerator = FileManager.default.enumerator(
            at: dataDirectory,
            includingPropertiesForKeys: nil
        ) else {
            return []
        }
        return enumerator.compactMap { $0 as? URL }.filter { $0.pathExtension.lowercased() == "wav" }
    }

    func cleanUp() {
        if let previousAppDataDirectory {
            setenv("MURMUR_APP_DATA_DIR", previousAppDataDirectory, 1)
        } else {
            unsetenv("MURMUR_APP_DATA_DIR")
        }
        try? FileManager.default.removeItem(at: dataDirectory)
    }
}

/// Builds an AppModel wired to fakes and isolated under a unique temp data dir.
///
/// - Sets MURMUR_APP_DATA_DIR before construction so settings, history DB,
///   recordings, and logs stay out of the real app data.
/// - Injects a no-op trailing-buffer sleep so stop paths settle immediately.
/// - Disables the overlay (`overlayPosition = .none`) so no NSPanel is shown
///   and clears shortcut bindings so no event tap can be (re)installed.
/// - Consumes the launch permission check, then pins a granted microphone
///   snapshot so the recording flow is deterministic regardless of the test
///   host's real TCC state.
@MainActor
func makeTestAppModel(
    capture: FlowFakeAudioCaptureService = FlowFakeAudioCaptureService(),
    paste: FlowFakePasteService = FlowFakePasteService()
) async throws -> RecordingFlowTestContext {
    let dataDirectory = FileManager.default.temporaryDirectory
        .appendingPathComponent("AppModelRecordingFlowTests-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: dataDirectory, withIntermediateDirectories: true)

    let previousAppDataDirectory = ProcessInfo.processInfo.environment["MURMUR_APP_DATA_DIR"]
    setenv("MURMUR_APP_DATA_DIR", dataDirectory.path, 1)

    let feedback = FlowFakeAudioFeedbackService()
    let mute = FlowFakeSystemAudioMuteService()
    let dependencies = AppModelDependencies(
        audioCaptureService: capture,
        audioFeedbackService: feedback,
        systemAudioMuteService: mute,
        pasteService: paste,
        recordingStopSequencer: RecordingStopSequencer(sleep: { _ in })
    )
    let appModel = AppModel(
        launchAtLoginService: FlowFakeLaunchAtLoginService(),
        dependencies: dependencies
    )

    // Run the launch permission check now so it cannot race the test body
    // (AppModel.init spawns it as a task; the guard makes this call idempotent).
    await appModel.refreshPermissionsAtLaunch()
    appModel.updateSettings { settings in
        settings.overlayPosition = OverlayPosition.none
        settings.shortcutBindings = [:]
    }
    appModel.permissionSnapshot = PermissionSnapshot(
        accessibilityTrusted: false,
        microphone: .granted,
        speechRecognition: .granted
    )
    appModel.clearLastErrorMessage()

    return RecordingFlowTestContext(
        appModel: appModel,
        capture: capture,
        feedback: feedback,
        mute: mute,
        paste: paste,
        dataDirectory: dataDirectory,
        previousAppDataDirectory: previousAppDataDirectory
    )
}
