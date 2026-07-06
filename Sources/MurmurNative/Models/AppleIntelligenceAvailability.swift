import Foundation

enum AppleIntelligenceAvailability: Equatable {
    case unchecked
    case checking
    case available
    case unavailable(String)

    var title: String {
        switch self {
        case .unchecked:
            "Not checked"
        case .checking:
            "Checking..."
        case .available:
            "Available"
        case .unavailable:
            "Unavailable"
        }
    }

    var isEmphasized: Bool {
        if case .available = self {
            return true
        }
        return false
    }

    var detail: String? {
        if case let .unavailable(reason) = self {
            return reason
        }
        return nil
    }
}
