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

    private var onCommand: (@Sendable (RemoteControlCommand) -> Void)?

    func start(onCommand: @escaping @Sendable (RemoteControlCommand) -> Void) {
        stop()
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
    }

    deinit {
        stop()
    }

    static func sendToRunningInstance(
        _ command: RemoteControlCommand,
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
            userInfo: [commandKey: command.rawValue],
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
        guard let rawCommand = notification.userInfo?[Self.commandKey] as? String,
              let command = RemoteControlCommand(rawValue: rawCommand)
        else {
            return
        }

        onCommand?(command)
    }
}
