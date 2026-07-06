import Foundation

struct GlobalShortcutActionContext: Equatable {
    var pushToTalk: Bool
    var recordingState: RecordingState
    var activeRecordingShortcutID: String?
}

enum GlobalShortcutEvent: Equatable {
    case pressed
    case released
}

enum GlobalShortcutAction: Equatable {
    case none
    case startRecording(postProcessRequested: Bool, shortcutID: String)
    case stopRecording
    case cancelRecording
}

enum GlobalShortcutActionRouter {
    static func action(
        for event: GlobalShortcutEvent,
        bindingID: String,
        context: GlobalShortcutActionContext
    ) -> GlobalShortcutAction {
        switch event {
        case .pressed:
            return pressedAction(bindingID: bindingID, context: context)
        case .released:
            return releasedAction(bindingID: bindingID, context: context)
        }
    }

    private static func pressedAction(
        bindingID: String,
        context: GlobalShortcutActionContext
    ) -> GlobalShortcutAction {
        if bindingID == ShortcutBinding.cancelID {
            return context.recordingState.isActive ? .cancelRecording : .none
        }

        guard bindingID == ShortcutBinding.transcribeID ||
            bindingID == ShortcutBinding.transcribeWithPostProcessID
        else {
            return .none
        }

        let postProcessRequested = bindingID == ShortcutBinding.transcribeWithPostProcessID
        if context.pushToTalk {
            guard context.recordingState == .idle else {
                return .none
            }
            return .startRecording(postProcessRequested: postProcessRequested, shortcutID: bindingID)
        }

        if context.recordingState.isRecording,
           context.activeRecordingShortcutID == bindingID {
            return .stopRecording
        }

        if context.recordingState == .idle {
            return .startRecording(postProcessRequested: postProcessRequested, shortcutID: bindingID)
        }

        return .none
    }

    private static func releasedAction(
        bindingID: String,
        context: GlobalShortcutActionContext
    ) -> GlobalShortcutAction {
        guard context.pushToTalk,
              context.recordingState.isRecording,
              context.activeRecordingShortcutID == bindingID
        else {
            return .none
        }

        return .stopRecording
    }
}
