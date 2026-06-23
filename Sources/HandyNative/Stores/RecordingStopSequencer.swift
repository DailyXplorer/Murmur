import Foundation

@MainActor
struct RecordingStopSequencer {
    var sleep: @MainActor @Sendable (Int) async throws -> Void

    init(sleep: @escaping @MainActor @Sendable (Int) async throws -> Void = RecordingStopSequencer.defaultSleep) {
        self.sleep = sleep
    }

    func stopAfterTrailingBuffer(
        settings: AppSettings,
        paths: AppPaths,
        audioCaptureService: any AudioCaptureServicing,
        systemAudioMuteService: any SystemAudioMuting,
        audioFeedbackService: any AudioFeedbackPlaying
    ) async throws -> AudioRecording {
        let trailingBufferMilliseconds = settings.extraRecordingBufferMilliseconds
        if trailingBufferMilliseconds > 0 {
            try await sleep(trailingBufferMilliseconds)
        }
        try Task.checkCancellation()

        do {
            let recording = try audioCaptureService.stop(
                keepStreamOpen: settings.alwaysOnMicrophone,
                lazyClose: settings.lazyStreamClose
            )
            systemAudioMuteService.removeMuteIfNeeded()
            audioFeedbackService.play(.stop, settings: settings, paths: paths)
            return recording
        } catch {
            systemAudioMuteService.removeMuteIfNeeded()
            throw error
        }
    }

    private static func defaultSleep(_ milliseconds: Int) async throws {
        try await Task.sleep(for: .milliseconds(milliseconds))
    }
}
