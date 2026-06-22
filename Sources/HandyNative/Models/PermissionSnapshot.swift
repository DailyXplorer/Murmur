import Foundation

struct PermissionSnapshot: Equatable {
    enum Microphone: String {
        case notDetermined
        case granted
        case denied
        case restricted
        case unknown

        var title: String {
            switch self {
            case .notDetermined: "Not requested"
            case .granted: "Granted"
            case .denied: "Denied"
            case .restricted: "Restricted"
            case .unknown: "Unknown"
            }
        }
    }

    enum SpeechRecognition: String {
        case notDetermined
        case granted
        case denied
        case restricted
        case unknown

        var title: String {
            switch self {
            case .notDetermined: "Not requested"
            case .granted: "Granted"
            case .denied: "Denied"
            case .restricted: "Restricted"
            case .unknown: "Unknown"
            }
        }
    }

    var accessibilityTrusted: Bool
    var microphone: Microphone
    var speechRecognition: SpeechRecognition

    static let unknown = PermissionSnapshot(
        accessibilityTrusted: false,
        microphone: .unknown,
        speechRecognition: .unknown
    )
}
