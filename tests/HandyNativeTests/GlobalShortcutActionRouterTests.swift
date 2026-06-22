import XCTest
@testable import HandyNative

final class GlobalShortcutActionRouterTests: XCTestCase {
    func testPushToTalkPressStartsAndReleaseStopsSameBinding() {
        let idleContext = GlobalShortcutActionContext(
            pushToTalk: true,
            recordingState: .idle,
            activeRecordingShortcutID: nil
        )

        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .pressed,
                bindingID: ShortcutBinding.transcribeID,
                context: idleContext
            ),
            .startRecording(postProcessRequested: false, shortcutID: ShortcutBinding.transcribeID)
        )

        let recordingContext = GlobalShortcutActionContext(
            pushToTalk: true,
            recordingState: .recording(startedAt: Date(timeIntervalSince1970: 1)),
            activeRecordingShortcutID: ShortcutBinding.transcribeID
        )

        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .released,
                bindingID: ShortcutBinding.transcribeID,
                context: recordingContext
            ),
            .stopRecording
        )
    }

    func testPushToTalkIgnoresRepeatPressAndUnrelatedRelease() {
        let context = GlobalShortcutActionContext(
            pushToTalk: true,
            recordingState: .recording(startedAt: Date(timeIntervalSince1970: 1)),
            activeRecordingShortcutID: ShortcutBinding.transcribeID
        )

        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .pressed,
                bindingID: ShortcutBinding.transcribeID,
                context: context
            ),
            .none
        )
        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .released,
                bindingID: ShortcutBinding.transcribeWithPostProcessID,
                context: context
            ),
            .none
        )
    }

    func testToggleModePressStartsThenStopsSameBinding() {
        let idleContext = GlobalShortcutActionContext(
            pushToTalk: false,
            recordingState: .idle,
            activeRecordingShortcutID: nil
        )

        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .pressed,
                bindingID: ShortcutBinding.transcribeID,
                context: idleContext
            ),
            .startRecording(postProcessRequested: false, shortcutID: ShortcutBinding.transcribeID)
        )

        let recordingContext = GlobalShortcutActionContext(
            pushToTalk: false,
            recordingState: .recording(startedAt: Date(timeIntervalSince1970: 1)),
            activeRecordingShortcutID: ShortcutBinding.transcribeID
        )

        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .pressed,
                bindingID: ShortcutBinding.transcribeID,
                context: recordingContext
            ),
            .stopRecording
        )
        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .released,
                bindingID: ShortcutBinding.transcribeID,
                context: recordingContext
            ),
            .none
        )
    }

    func testPostProcessShortcutRequestsPostProcessing() {
        let context = GlobalShortcutActionContext(
            pushToTalk: false,
            recordingState: .idle,
            activeRecordingShortcutID: nil
        )

        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .pressed,
                bindingID: ShortcutBinding.transcribeWithPostProcessID,
                context: context
            ),
            .startRecording(
                postProcessRequested: true,
                shortcutID: ShortcutBinding.transcribeWithPostProcessID
            )
        )
    }

    func testCancelCancelsAnyActiveRecordingOperationState() {
        let idleContext = GlobalShortcutActionContext(
            pushToTalk: false,
            recordingState: .idle,
            activeRecordingShortcutID: nil
        )
        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .pressed,
                bindingID: ShortcutBinding.cancelID,
                context: idleContext
            ),
            .none
        )

        let recordingContext = GlobalShortcutActionContext(
            pushToTalk: false,
            recordingState: .recording(startedAt: Date(timeIntervalSince1970: 1)),
            activeRecordingShortcutID: ShortcutBinding.transcribeID
        )
        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .pressed,
                bindingID: ShortcutBinding.cancelID,
                context: recordingContext
            ),
            .cancelRecording
        )

        let transcribingContext = GlobalShortcutActionContext(
            pushToTalk: false,
            recordingState: .transcribing,
            activeRecordingShortcutID: nil
        )
        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .pressed,
                bindingID: ShortcutBinding.cancelID,
                context: transcribingContext
            ),
            .cancelRecording
        )

        let processingContext = GlobalShortcutActionContext(
            pushToTalk: false,
            recordingState: .processing,
            activeRecordingShortcutID: nil
        )
        XCTAssertEqual(
            GlobalShortcutActionRouter.action(
                for: .pressed,
                bindingID: ShortcutBinding.cancelID,
                context: processingContext
            ),
            .cancelRecording
        )
    }
}
