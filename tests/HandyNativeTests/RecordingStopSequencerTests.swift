import Foundation
import XCTest
@testable import HandyNative

@MainActor
final class RecordingStopSequencerTests: XCTestCase {
    func testStopEffectsRunOnlyAfterCaptureStops() async throws {
        let eventLog = RecordingStopSequencerEventLog()
        let sequencer = RecordingStopSequencer { milliseconds in
            eventLog.append("sleep:\(milliseconds)")
        }
        var settings = AppSettings.defaults
        settings.audioFeedback = true
        settings.extraRecordingBufferMilliseconds = 250
        let captureService = FakeRecordingAudioCaptureService(
            eventLog: eventLog,
            recording: Self.audibleRecording()
        )
        let muteService = FakeRecordingSystemAudioMuteService(eventLog: eventLog)
        let feedbackService = FakeRecordingAudioFeedbackService(eventLog: eventLog)

        _ = try await sequencer.stopAfterTrailingBuffer(
            settings: settings,
            paths: Self.makePaths(),
            audioCaptureService: captureService,
            systemAudioMuteService: muteService,
            audioFeedbackService: feedbackService
        )

        XCTAssertEqual(
            eventLog.events,
            ["sleep:250", "capture.stop", "mute.remove", "feedback.stop"]
        )
    }

    func testStopFailureStillReleasesMuteWithoutPlayingStopFeedback() async throws {
        let eventLog = RecordingStopSequencerEventLog()
        let sequencer = RecordingStopSequencer { _ in }
        var settings = AppSettings.defaults
        settings.audioFeedback = true
        let captureService = FakeRecordingAudioCaptureService(
            eventLog: eventLog,
            recording: Self.audibleRecording()
        )
        captureService.stopError = RecordingStopSequencerTestError.stopFailed
        let muteService = FakeRecordingSystemAudioMuteService(eventLog: eventLog)
        let feedbackService = FakeRecordingAudioFeedbackService(eventLog: eventLog)

        do {
            _ = try await sequencer.stopAfterTrailingBuffer(
                settings: settings,
                paths: Self.makePaths(),
                audioCaptureService: captureService,
                systemAudioMuteService: muteService,
                audioFeedbackService: feedbackService
            )
            XCTFail("Expected capture stop to fail.")
        } catch RecordingStopSequencerTestError.stopFailed {
            XCTAssertEqual(eventLog.events, ["capture.stop", "mute.remove"])
        }
    }

    private static func audibleRecording() -> AudioRecording {
        AudioRecording(
            samples: Array(repeating: 0.02, count: 16_000),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 1)
        )
    }

    private static func makePaths() -> AppPaths {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("RecordingStopSequencerTests", isDirectory: true)
        return AppPaths(
            appDataDirectory: root.appendingPathComponent("app-data", isDirectory: true),
            recordingsDirectory: root.appendingPathComponent("recordings", isDirectory: true),
            modelsDirectory: root.appendingPathComponent("models", isDirectory: true),
            logsDirectory: root.appendingPathComponent("logs", isDirectory: true)
        )
    }
}

@MainActor
private final class RecordingStopSequencerEventLog {
    private(set) var events: [String] = []

    func append(_ event: String) {
        events.append(event)
    }
}

@MainActor
private final class FakeRecordingAudioCaptureService: AudioCaptureServicing {
    var isRecording = true
    var voiceProcessingStatus: AudioInputVoiceProcessingStatus = .notConfigured
    var stopError: Error?
    private let eventLog: RecordingStopSequencerEventLog
    private let recording: AudioRecording

    init(eventLog: RecordingStopSequencerEventLog, recording: AudioRecording) {
        self.eventLog = eventLog
        self.recording = recording
    }

    func start(
        selectedMicrophoneName _: String?,
        voiceProcessingConfiguration _: AudioInputVoiceProcessingConfiguration,
        onLevel _: @escaping @Sendable (Float) -> Void
    ) throws {}

    func stop(keepStreamOpen _: Bool, lazyClose _: Bool) throws -> AudioRecording {
        eventLog.append("capture.stop")
        if let stopError {
            throw stopError
        }
        isRecording = false
        return recording
    }

    func cancel(keepStreamOpen _: Bool, lazyClose _: Bool) {
        isRecording = false
    }

    func openIdleStream(
        selectedMicrophoneName _: String?,
        voiceProcessingConfiguration _: AudioInputVoiceProcessingConfiguration
    ) throws {}

    func closeIdleStream() {}
}

@MainActor
private final class FakeRecordingSystemAudioMuteService: SystemAudioMuting {
    private let eventLog: RecordingStopSequencerEventLog

    init(eventLog: RecordingStopSequencerEventLog) {
        self.eventLog = eventLog
    }

    func applyMuteIfNeeded(settings _: AppSettings) {
        eventLog.append("mute.apply")
    }

    func removeMuteIfNeeded() {
        eventLog.append("mute.remove")
    }
}

@MainActor
private final class FakeRecordingAudioFeedbackService: AudioFeedbackPlaying {
    private let eventLog: RecordingStopSequencerEventLog

    init(eventLog: RecordingStopSequencerEventLog) {
        self.eventLog = eventLog
    }

    func play(_ sound: AudioFeedbackSound, settings _: AppSettings, paths _: AppPaths) {
        eventLog.append("feedback.\(sound.rawValue)")
    }
}

private enum RecordingStopSequencerTestError: Error {
    case stopFailed
}
