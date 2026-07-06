import Foundation

struct RecordingCoordinator {
    private(set) var state: RecordingState = .idle

    mutating func start(now: Date = Date()) -> Bool {
        guard state == .idle else {
            return false
        }

        state = .recording(startedAt: now)
        return true
    }

    mutating func stop() -> Bool {
        guard state.isRecording else {
            return false
        }

        state = .transcribing
        return true
    }

    mutating func finishProcessing() {
        state = .idle
    }

    mutating func cancel() {
        state = .idle
    }
}
