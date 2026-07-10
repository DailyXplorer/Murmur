import Foundation
@testable import MurmurNative
import XCTest

/// Characterization tests for AppModel's recording flow:
/// shortcut/UI action -> start -> stop -> trailing buffer -> audible gate ->
/// WAV write -> history entry -> transcribe -> paste -> settle to idle.
///
/// These tests pin CURRENT behavior (as of commit 87381b4) so the deferred
/// state-machine/AppModel-split refactors have a safety net. They are not a
/// spec: if one of these assertions blocks an intentional behavior change,
/// update the assertion alongside the change.
///
/// Known limitation: the transcription engines are constructed inside AppModel
/// (transcribeAudioFile calls the static AudioFileTranscriptionPipeline.transcribe,
/// and the API engine's URLSession is not injectable from AppModel's call path),
/// so the transcription-SUCCESS leg cannot be exercised here. The suite covers
/// paths that fail before engine dispatch (missing API key for the default
/// Mistral model, unknown model id) and paths that never reach transcription
/// (silent audio, cancel, start failure). No network request is ever made:
/// both failure shapes throw before any URLSession dispatch.
@MainActor
final class AppModelRecordingFlowTests: XCTestCase {
    private var context: RecordingFlowTestContext?

    override func tearDown() async throws {
        context?.cleanUp()
        context = nil
        try await super.tearDown()
    }

    private func makeContext(
        capture: FlowFakeAudioCaptureService = FlowFakeAudioCaptureService(),
        paste: FlowFakePasteService = FlowFakePasteService()
    ) async throws -> RecordingFlowTestContext {
        let context = try await makeTestAppModel(capture: capture, paste: paste)
        self.context = context
        return context
    }

    private func waitForIdle(
        _ appModel: AppModel,
        timeout: TimeInterval = 2,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async {
        let deadline = Date().addingTimeInterval(timeout)
        while appModel.recordingState != .idle {
            if Date() >= deadline {
                XCTFail(
                    "recording flow did not settle within \(timeout)s (state: \(appModel.recordingState))",
                    file: file,
                    line: line
                )
                return
            }
            try? await Task.sleep(for: .milliseconds(10))
        }
    }

    // MARK: - Tests

    func testStartThenStopWithSilentAudioEndsIdleWithoutPaste() async throws {
        let context = try await makeContext()
        context.capture.stopResult = .silentFlowFixture()

        context.appModel.startRecording()
        XCTAssertTrue(context.appModel.recordingState.isRecording)

        context.appModel.stopRecording()
        await waitForIdle(context.appModel)

        XCTAssertTrue(context.paste.pastedTexts.isEmpty)
        // characterization: current behavior as of 87381b4 — a stop that captured
        // no audible signal surfaces "No speech detected" instead of failing silently.
        XCTAssertEqual(context.appModel.lastErrorMessage?.contains("No speech detected"), true)
        // characterization: the silent discard happens before the history entry
        // and WAV write, so no artifacts are produced.
        XCTAssertTrue(context.appModel.historyEntries.isEmpty)
        XCTAssertTrue(context.recordedWAVFiles().isEmpty)
        XCTAssertEqual(context.capture.stopCallCount, 1)
    }

    func testStartThenStopWithAudibleAudioWritesRecordingAndHistoryEntry() async throws {
        let context = try await makeContext()
        context.capture.stopResult = .audibleFlowFixture()

        context.appModel.startRecording()
        XCTAssertTrue(context.appModel.recordingState.isRecording)

        context.appModel.stopRecording()
        await waitForIdle(context.appModel)

        XCTAssertEqual(context.recordedWAVFiles().count, 1)
        XCTAssertNotNil(context.appModel.lastRecordingURL)
        XCTAssertEqual(context.appModel.historyEntries.count, 1)
        // characterization: current behavior as of 87381b4 — the default model is
        // the Mistral API model and no API key exists in a fresh data dir, so the
        // failure leg of transcribeAndOutput runs (missingAPIKey is thrown before
        // any network dispatch). The history entry stays pending, nothing pastes.
        XCTAssertEqual(context.appModel.lastErrorMessage?.contains("API key"), true)
        XCTAssertEqual(context.appModel.historyEntries.first?.hasTranscription, false)
        XCTAssertTrue(context.paste.pastedTexts.isEmpty)
    }

    func testUnknownSelectedModelSurfacesError() async throws {
        let context = try await makeContext()
        context.appModel.updateSettings { $0.selectedModel = "does-not-exist" }
        // characterization: ensureTranscriptionAPIDefaults() repairs unknown
        // selections only at settings LOAD time; a runtime updateSettings write
        // keeps the unknown id in memory.
        XCTAssertEqual(context.appModel.settings.selectedModel, "does-not-exist")

        context.capture.stopResult = .audibleFlowFixture()
        context.appModel.startRecording()
        XCTAssertTrue(context.appModel.recordingState.isRecording)
        context.appModel.stopRecording()
        await waitForIdle(context.appModel)

        // characterization: current behavior as of 87381b4 — an unreachable engine
        // surfaces TranscriptionEngineSelectionError.unsupportedModel, naming the model.
        XCTAssertNotNil(context.appModel.lastErrorMessage)
        XCTAssertEqual(context.appModel.lastErrorMessage?.contains("does-not-exist"), true)
        XCTAssertTrue(context.paste.pastedTexts.isEmpty)
        // The recording itself still succeeded: WAV and pending history entry exist.
        XCTAssertEqual(context.recordedWAVFiles().count, 1)
        XCTAssertEqual(context.appModel.historyEntries.count, 1)
    }

    func testCancelDuringRecordingDiscardsEverything() async throws {
        let context = try await makeContext()
        context.appModel.startRecording()
        XCTAssertTrue(context.appModel.recordingState.isRecording)

        context.appModel.cancelRecording()

        XCTAssertEqual(context.appModel.recordingState, .idle)
        XCTAssertEqual(context.capture.cancelCallCount, 1)
        XCTAssertEqual(context.capture.stopCallCount, 0)
        XCTAssertTrue(context.appModel.historyEntries.isEmpty)
        XCTAssertTrue(context.recordedWAVFiles().isEmpty)
        XCTAssertTrue(context.paste.pastedTexts.isEmpty)
        // characterization: current behavior as of 87381b4 — cancel releases the
        // system-audio mute unconditionally.
        XCTAssertGreaterThanOrEqual(context.mute.removeCallCount, 1)
    }

    func testStartFailureRestoresIdle() async throws {
        let context = try await makeContext()
        context.capture.startError = FlowFakeError(message: "audio engine unavailable")

        context.appModel.startRecording()

        XCTAssertEqual(context.appModel.recordingState, .idle)
        XCTAssertEqual(context.appModel.lastErrorMessage, "audio engine unavailable")
        XCTAssertEqual(context.capture.startCallCount, 1)
        XCTAssertTrue(context.paste.pastedTexts.isEmpty)
        XCTAssertTrue(context.appModel.historyEntries.isEmpty)
    }

    func testRapidStopStartDoesNotCrossOperations() async throws {
        let context = try await makeContext()
        context.capture.stopResult = .silentFlowFixture()

        context.appModel.startRecording()
        XCTAssertTrue(context.appModel.recordingState.isRecording)
        context.appModel.stopRecording()
        XCTAssertEqual(context.appModel.recordingState, .transcribing)

        // Request a new recording while the first stop is still in flight.
        context.appModel.startRecording()
        // characterization: current behavior as of 87381b4 — the coordinator
        // rejects a start while a stop is in flight (state is .transcribing), so
        // the restart request is IGNORED rather than starting a second operation.
        // It does not clobber the in-flight operation and never reaches the
        // capture service.
        XCTAssertEqual(context.appModel.recordingState, .transcribing)
        XCTAssertEqual(context.capture.startCallCount, 1)

        await waitForIdle(context.appModel)
        XCTAssertEqual(context.appModel.lastErrorMessage?.contains("No speech detected"), true)

        // Once the first operation settles, a fresh start works and completes
        // without inheriting anything from the discarded silent operation.
        context.capture.stopResult = .audibleFlowFixture()
        context.appModel.startRecording()
        XCTAssertTrue(context.appModel.recordingState.isRecording)
        context.appModel.stopRecording()
        await waitForIdle(context.appModel)

        // Exactly one completed operation's artifacts exist: the silent first
        // operation was discarded, the audible second one produced one WAV and
        // one (pending) history entry.
        XCTAssertEqual(context.recordedWAVFiles().count, 1)
        XCTAssertEqual(context.appModel.historyEntries.count, 1)
        XCTAssertEqual(context.capture.startCallCount, 2)
        XCTAssertEqual(context.capture.stopCallCount, 2)
        XCTAssertTrue(context.paste.pastedTexts.isEmpty)
    }

    func testStopWithoutStartIsIgnored() async throws {
        let context = try await makeContext()

        context.appModel.stopRecording()

        XCTAssertEqual(context.appModel.recordingState, .idle)
        XCTAssertNil(context.appModel.lastErrorMessage)
        XCTAssertEqual(context.capture.stopCallCount, 0)
        XCTAssertTrue(context.recordedWAVFiles().isEmpty)
    }
}
