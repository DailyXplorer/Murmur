import Foundation

enum RecordingState: Equatable {
    case idle
    case recording(startedAt: Date)
    case transcribing
    case processing

    var isRecording: Bool {
        if case .recording = self {
            true
        } else {
            false
        }
    }

    var isActive: Bool {
        self != .idle
    }

    var title: String {
        switch self {
        case .idle: "Ready"
        case .recording: "Recording"
        case .transcribing: "Transcribing"
        case .processing: "Processing"
        }
    }
}
