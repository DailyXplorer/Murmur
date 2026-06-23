import Foundation

@MainActor
protocol AudioCaptureServicing: AnyObject {
    var isRecording: Bool { get }
    var voiceProcessingStatus: AudioInputVoiceProcessingStatus { get }

    func start(
        selectedMicrophoneName: String?,
        voiceProcessingConfiguration: AudioInputVoiceProcessingConfiguration,
        onLevel: @escaping @Sendable (Float) -> Void
    ) throws
    func stop(keepStreamOpen: Bool, lazyClose: Bool) throws -> AudioRecording
    func cancel(keepStreamOpen: Bool, lazyClose: Bool)
    func openIdleStream(
        selectedMicrophoneName: String?,
        voiceProcessingConfiguration: AudioInputVoiceProcessingConfiguration
    ) throws
    func closeIdleStream()
}

extension AudioCaptureService: AudioCaptureServicing {}

@MainActor
protocol AudioFeedbackPlaying: AnyObject {
    func play(_ sound: AudioFeedbackSound, settings: AppSettings, paths: AppPaths)
}

extension AudioFeedbackService: AudioFeedbackPlaying {}

@MainActor
protocol SystemAudioMuting: AnyObject {
    func applyMuteIfNeeded(settings: AppSettings)
    func removeMuteIfNeeded()
}

extension SystemAudioMuteService: SystemAudioMuting {}

@MainActor
protocol PasteServicing: AnyObject {
    func paste(_ rawText: String, options: PasteOutputOptions) async throws
}

extension PasteService: PasteServicing {}

@MainActor
struct AppModelDependencies {
    var audioCaptureService: any AudioCaptureServicing
    var audioFeedbackService: any AudioFeedbackPlaying
    var systemAudioMuteService: any SystemAudioMuting
    var pasteService: any PasteServicing
    var recordingWorkflow: RecordingWorkflow

    init(
        audioCaptureService: any AudioCaptureServicing,
        audioFeedbackService: any AudioFeedbackPlaying,
        systemAudioMuteService: any SystemAudioMuting,
        pasteService: any PasteServicing,
        recordingStopSequencer: RecordingStopSequencer = RecordingStopSequencer()
    ) {
        self.audioCaptureService = audioCaptureService
        self.audioFeedbackService = audioFeedbackService
        self.systemAudioMuteService = systemAudioMuteService
        self.pasteService = pasteService
        recordingWorkflow = RecordingWorkflow(
            audioCaptureService: audioCaptureService,
            audioFeedbackService: audioFeedbackService,
            systemAudioMuteService: systemAudioMuteService,
            stopSequencer: recordingStopSequencer
        )
    }

    static func live() -> AppModelDependencies {
        AppModelDependencies(
            audioCaptureService: AudioCaptureService(),
            audioFeedbackService: AudioFeedbackService(),
            systemAudioMuteService: SystemAudioMuteService(),
            pasteService: PasteService()
        )
    }
}
