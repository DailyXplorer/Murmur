import Foundation

enum RecordingOverlayState: Equatable {
    case recording
    case transcribing
    case processing

    var title: String {
        switch self {
        case .recording: "Recording"
        case .transcribing: "Transcribing"
        case .processing: "Processing"
        }
    }
}
