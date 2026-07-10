import Foundation

enum RecordingOverlayState: Equatable {
    case recording
    case transcribing
    case processing
    case failure(message: String)
    case notice(message: String)

    var title: String {
        switch self {
        case .recording: "Recording"
        case .transcribing: "Transcribing"
        case .processing: "Processing"
        case let .failure(message): message
        case let .notice(message): message
        }
    }
}
