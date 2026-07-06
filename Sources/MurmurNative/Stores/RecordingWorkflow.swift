import Foundation

@MainActor
struct RecordingWorkflow {
    var audioCaptureService: any AudioCaptureServicing
    var audioFeedbackService: any AudioFeedbackPlaying
    var systemAudioMuteService: any SystemAudioMuting
    var stopSequencer: RecordingStopSequencer
    var startFeedbackDelaySleep: @MainActor @Sendable (Int) async throws -> Void

    init(
        audioCaptureService: any AudioCaptureServicing,
        audioFeedbackService: any AudioFeedbackPlaying,
        systemAudioMuteService: any SystemAudioMuting,
        stopSequencer: RecordingStopSequencer = RecordingStopSequencer(),
        startFeedbackDelaySleep: @escaping @MainActor @Sendable (Int) async throws -> Void = RecordingWorkflow.defaultSleep
    ) {
        self.audioCaptureService = audioCaptureService
        self.audioFeedbackService = audioFeedbackService
        self.systemAudioMuteService = systemAudioMuteService
        self.stopSequencer = stopSequencer
        self.startFeedbackDelaySleep = startFeedbackDelaySleep
    }

    func start(
        settings: AppSettings,
        paths: AppPaths,
        selectedMicrophoneName: String?,
        onLevel: @escaping @Sendable (Float) -> Void
    ) throws {
        try audioCaptureService.start(
            selectedMicrophoneName: selectedMicrophoneName,
            voiceProcessingConfiguration: settings.audioInputVoiceProcessingConfiguration,
            onLevel: onLevel
        )
        audioFeedbackService.play(.start, settings: settings, paths: paths)
    }

    func applyMuteAfterStartFeedback(
        settings: AppSettings,
        shouldApplyMute: @MainActor () -> Bool
    ) async {
        do {
            try await startFeedbackDelaySleep(250)
        } catch {
            return
        }

        if shouldApplyMute() {
            systemAudioMuteService.applyMuteIfNeeded(settings: settings)
        }
    }

    func stopAfterTrailingBuffer(settings: AppSettings, paths: AppPaths) async throws -> AudioRecording {
        try await stopSequencer.stopAfterTrailingBuffer(
            settings: settings,
            paths: paths,
            audioCaptureService: audioCaptureService,
            systemAudioMuteService: systemAudioMuteService,
            audioFeedbackService: audioFeedbackService
        )
    }

    func cancel(settings: AppSettings) {
        audioCaptureService.cancel(
            keepStreamOpen: settings.alwaysOnMicrophone,
            lazyClose: settings.lazyStreamClose
        )
        systemAudioMuteService.removeMuteIfNeeded()
    }

    private static func defaultSleep(_ milliseconds: Int) async throws {
        try await Task.sleep(for: .milliseconds(milliseconds))
    }
}
