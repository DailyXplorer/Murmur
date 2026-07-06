import XCTest
@testable import MurmurNative

final class RecordingCoordinatorTests: XCTestCase {
    func testStartOnlyTransitionsFromIdle() {
        var coordinator = RecordingCoordinator()

        XCTAssertTrue(coordinator.start())
        XCTAssertTrue(coordinator.state.isRecording)
        XCTAssertFalse(coordinator.start())
    }

    func testStopOnlyTransitionsFromRecording() {
        var coordinator = RecordingCoordinator()

        XCTAssertFalse(coordinator.stop())
        XCTAssertTrue(coordinator.start())
        XCTAssertTrue(coordinator.stop())
        XCTAssertEqual(coordinator.state, .transcribing)
    }

    func testCancelReturnsToIdle() {
        var coordinator = RecordingCoordinator()

        XCTAssertTrue(coordinator.start())
        coordinator.cancel()

        XCTAssertEqual(coordinator.state, .idle)
    }

    func testCancelReturnsTranscribingToIdle() {
        var coordinator = RecordingCoordinator()

        XCTAssertTrue(coordinator.start())
        XCTAssertTrue(coordinator.stop())
        coordinator.cancel()

        XCTAssertEqual(coordinator.state, .idle)
    }

    func testRecordingStateActiveIncludesTranscribingAndProcessing() {
        XCTAssertFalse(RecordingState.idle.isActive)
        XCTAssertTrue(RecordingState.recording(startedAt: Date(timeIntervalSince1970: 1)).isActive)
        XCTAssertTrue(RecordingState.transcribing.isActive)
        XCTAssertTrue(RecordingState.processing.isActive)
    }
}
