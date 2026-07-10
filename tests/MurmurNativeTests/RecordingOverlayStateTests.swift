import XCTest
@testable import MurmurNative

final class RecordingOverlayStateTests: XCTestCase {
    func testTitleMapsBaseCases() {
        XCTAssertEqual(RecordingOverlayState.recording.title, "Recording")
        XCTAssertEqual(RecordingOverlayState.transcribing.title, "Transcribing")
        XCTAssertEqual(RecordingOverlayState.processing.title, "Processing")
    }

    func testFailureTitleUsesAssociatedMessage() {
        XCTAssertEqual(
            RecordingOverlayState.failure(message: "Transcription failed").title,
            "Transcription failed"
        )
    }

    func testNoticeTitleUsesAssociatedMessage() {
        XCTAssertEqual(
            RecordingOverlayState.notice(message: "No speech detected").title,
            "No speech detected"
        )
    }

    func testEquatableComparesAssociatedMessages() {
        XCTAssertEqual(
            RecordingOverlayState.failure(message: "A"),
            RecordingOverlayState.failure(message: "A")
        )
        XCTAssertNotEqual(
            RecordingOverlayState.failure(message: "A"),
            RecordingOverlayState.failure(message: "B")
        )
        XCTAssertEqual(
            RecordingOverlayState.notice(message: "A"),
            RecordingOverlayState.notice(message: "A")
        )
        XCTAssertNotEqual(
            RecordingOverlayState.notice(message: "A"),
            RecordingOverlayState.notice(message: "B")
        )
    }

    func testFailureAndNoticeWithSameMessageAreDistinct() {
        XCTAssertNotEqual(
            RecordingOverlayState.failure(message: "Same"),
            RecordingOverlayState.notice(message: "Same")
        )
    }
}
