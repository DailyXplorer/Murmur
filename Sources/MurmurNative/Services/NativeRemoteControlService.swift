import AppKit
import Foundation

enum RemoteControlCommand: String, Equatable {
    case toggleTranscription = "toggle-transcription"
    case togglePostProcess = "toggle-post-process"
    case cancel
}

final class NativeRemoteControlService: NSObject {
    private static let notificationName = Notification.Name("computer.murmur.native.remote-control")
    private static let commandKey = "command"
    private static let tokenKey = "token"
    private static let tokenFileName = ".remote-control-token"

    private var onCommand: (@Sendable (RemoteControlCommand) -> Void)?
    private var expectedToken: String?

    func start(expectedToken: String, onCommand: @escaping @Sendable (RemoteControlCommand) -> Void) {
        stop()
        self.expectedToken = expectedToken
        self.onCommand = onCommand
        DistributedNotificationCenter.default().addObserver(
            self,
            selector: #selector(handleNotification(_:)),
            name: Self.notificationName,
            object: nil
        )
    }

    func stop() {
        DistributedNotificationCenter.default().removeObserver(self)
        onCommand = nil
        expectedToken = nil
    }

    deinit {
        stop()
    }

    static func localAuthorizationToken(appDataDirectory: URL) -> String {
        let tokenURL = appDataDirectory.appendingPathComponent(tokenFileName)
        if let existing = try? String(contentsOf: tokenURL, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines),
            existing.isEmpty == false {
            return existing
        }
        let token = UUID().uuidString
        try? token.write(to: tokenURL, atomically: true, encoding: .utf8)
        try? FileManager.default.setAttributes(
            [.posixPermissions: 0o600],
            ofItemAtPath: tokenURL.path
        )
        return token
    }

    static func resolveLocalAuthorizationToken() -> String? {
        guard let paths = try? AppPaths.resolve() else {
            return nil
        }
        return localAuthorizationToken(appDataDirectory: paths.appDataDirectory)
    }

    static func authorizedCommand(
        from userInfo: [AnyHashable: Any]?,
        expectedToken: String
    ) -> RemoteControlCommand? {
        guard let token = userInfo?[Self.tokenKey] as? String,
              token == expectedToken,
              let rawCommand = userInfo?[Self.commandKey] as? String,
              let command = RemoteControlCommand(rawValue: rawCommand)
        else {
            return nil
        }
        return command
    }

    static func sendToRunningInstance(
        _ command: RemoteControlCommand,
        token: String,
        bundleIdentifier: String? = Bundle.main.bundleIdentifier,
        currentProcessIdentifier: pid_t = getpid()
    ) -> Bool {
        guard hasRunningPeer(
            bundleIdentifier: bundleIdentifier,
            currentProcessIdentifier: currentProcessIdentifier
        ) else {
            return false
        }

        DistributedNotificationCenter.default().postNotificationName(
            notificationName,
            object: nil,
            userInfo: [commandKey: command.rawValue, tokenKey: token],
            deliverImmediately: true
        )
        return true
    }

    static func hasRunningPeer(
        bundleIdentifier: String? = Bundle.main.bundleIdentifier,
        currentProcessIdentifier: pid_t = getpid()
    ) -> Bool {
        guard let bundleIdentifier, bundleIdentifier.isEmpty == false else {
            return false
        }

        return NSRunningApplication
            .runningApplications(withBundleIdentifier: bundleIdentifier)
            .contains { $0.processIdentifier != currentProcessIdentifier }
    }

    @objc private func handleNotification(_ notification: Notification) {
        guard let expectedToken,
              let command = Self.authorizedCommand(
                  from: notification.userInfo,
                  expectedToken: expectedToken
              )
        else {
            return
        }

        onCommand?(command)
    }
}
