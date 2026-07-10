import Foundation

enum ShortcutHealthAction: Equatable {
    case none
    case install                      // trusted, but tap not installed
    case reinstall                    // tap installed but dead/disabled
    case teardownAndWarn              // trust revoked while tap installed
    case warnSecureInput              // suppressed by another app's secure input
}

enum ShortcutHealthPolicy {
    static func assess(
        accessibilityTrusted: Bool,
        tapInstalled: Bool,
        tapHealthy: Bool,
        secureInputActive: Bool,
        recordingActive: Bool
    ) -> ShortcutHealthAction {
        if recordingActive {
            return .none    // never touch the tap mid-gesture (plan 001's invariant)
        }
        if accessibilityTrusted == false {
            return tapInstalled ? .teardownAndWarn : .none
        }
        if secureInputActive {
            return .warnSecureInput
        }
        if tapInstalled == false {
            return .install
        }
        if tapHealthy == false {
            return .reinstall
        }
        return .none
    }
}
