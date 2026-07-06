import Foundation
import ServiceManagement

enum LaunchAtLoginStatus: Equatable {
    case enabled
    case disabled
    case requiresApproval
    case notFound
    case unavailable
    case unknown(String)

    var userFacingLabel: String {
        switch self {
        case .enabled:
            "Enabled"
        case .disabled:
            "Off"
        case .requiresApproval:
            "Approval required"
        case .notFound:
            "Not found"
        case .unavailable:
            "Unavailable"
        case let .unknown(value):
            value.isEmpty ? "Unknown" : value
        }
    }

    func shouldApplyStoredSetting(_ enabled: Bool) -> Bool {
        switch (enabled, self) {
        case (true, .enabled), (true, .requiresApproval):
            false
        case (false, .disabled), (false, .notFound), (false, .unavailable):
            false
        default:
            true
        }
    }
}

protocol LaunchAtLoginServicing {
    func currentStatus() -> LaunchAtLoginStatus
    func setEnabled(_ enabled: Bool) throws
}

enum LaunchAtLoginServiceError: LocalizedError {
    case unavailable

    var errorDescription: String? {
        switch self {
        case .unavailable:
            "Launch at login is unavailable on this version of macOS."
        }
    }
}

struct LaunchAtLoginService: LaunchAtLoginServicing {
    func currentStatus() -> LaunchAtLoginStatus {
        if #available(macOS 13.0, *) {
            return LaunchAtLoginStatus(SMAppService.mainApp.status)
        }

        return .unavailable
    }

    func setEnabled(_ enabled: Bool) throws {
        guard #available(macOS 13.0, *) else {
            throw LaunchAtLoginServiceError.unavailable
        }

        if enabled {
            try SMAppService.mainApp.register()
        } else {
            try SMAppService.mainApp.unregister()
        }
    }
}

@available(macOS 13.0, *)
private extension LaunchAtLoginStatus {
    init(_ status: SMAppService.Status) {
        switch status {
        case .enabled:
            self = .enabled
        case .notRegistered:
            self = .disabled
        case .requiresApproval:
            self = .requiresApproval
        case .notFound:
            self = .notFound
        @unknown default:
            self = .unknown(String(describing: status))
        }
    }
}
