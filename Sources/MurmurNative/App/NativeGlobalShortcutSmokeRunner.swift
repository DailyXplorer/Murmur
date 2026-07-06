import AppKit
import Foundation

enum NativeGlobalShortcutSmokeRunner {
    @MainActor
    static func runSynchronouslyAndExit(_ request: NativeGlobalShortcutSmokeRequest) -> Never {
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)
        application.finishLaunching()

        Task { @MainActor in
            do {
                let output = try await output(for: request)
                let outputString = try outputString(for: output)
                if let outputPath = request.outputPath {
                    try write(outputString, to: outputPath)
                }
                FileHandle.standardOutput.writeLine(outputString)
                exit(output.success ? 0 : 1)
            } catch {
                FileHandle.standardError.writeLine(error.localizedDescription)
                exit(1)
            }
        }

        application.run()
        exit(1)
    }

    @MainActor
    static func output(for request: NativeGlobalShortcutSmokeRequest) async throws -> NativeGlobalShortcutSmokeOutput {
        guard let descriptor = GlobalShortcutDescriptor.parse(request.binding) else {
            throw NativeGlobalShortcutSmokeError.invalidBinding(request.binding)
        }

        let permissionSnapshot = PermissionService().snapshot()
        guard permissionSnapshot.accessibilityTrusted else {
            return NativeGlobalShortcutSmokeOutput(
                success: false,
                requestedBindingID: request.bindingID,
                requestedBinding: request.binding,
                keyCode: Int(descriptor.keyCode),
                requiredFlagsRawValue: Int(descriptor.requiredFlags.rawValue),
                accessibilityTrusted: false,
                eventTapRunning: false,
                eventPostSucceeded: false,
                pressedBindingIDs: [],
                releasedBindingIDs: [],
                observedPressed: false,
                observedReleased: false
            )
        }

        let observedEvents = LockedGlobalShortcutSmokeEvents()
        let shortcutService = GlobalShortcutService()
        try shortcutService.start(
            registrations: [
                GlobalShortcutRegistration(
                    bindingID: request.bindingID,
                    descriptor: descriptor
                )
            ],
            onPressed: { bindingID in
                observedEvents.recordPressed(bindingID)
            },
            onReleased: { bindingID in
                observedEvents.recordReleased(bindingID)
            }
        )
        defer {
            shortcutService.stop()
        }

        try await Task.sleep(for: .milliseconds(150))
        try CGKeyboardEventPoster().postShortcut(
            virtualKey: descriptor.keyCode,
            flags: descriptor.requiredFlags
        )

        let deadline = Date().addingTimeInterval(3)
        while Date() < deadline {
            let snapshot = observedEvents.snapshot()
            if snapshot.pressed.contains(request.bindingID),
               snapshot.released.contains(request.bindingID) {
                break
            }
            try await Task.sleep(for: .milliseconds(50))
        }

        let snapshot = observedEvents.snapshot()
        let observedPressed = snapshot.pressed.contains(request.bindingID)
        let observedReleased = snapshot.released.contains(request.bindingID)
        return NativeGlobalShortcutSmokeOutput(
            success: shortcutService.isRunning && observedPressed && observedReleased,
            requestedBindingID: request.bindingID,
            requestedBinding: request.binding,
            keyCode: Int(descriptor.keyCode),
            requiredFlagsRawValue: Int(descriptor.requiredFlags.rawValue),
            accessibilityTrusted: true,
            eventTapRunning: shortcutService.isRunning,
            eventPostSucceeded: true,
            pressedBindingIDs: snapshot.pressed,
            releasedBindingIDs: snapshot.released,
            observedPressed: observedPressed,
            observedReleased: observedReleased
        )
    }

    private static func write(_ output: String, to path: String) throws {
        let outputURL = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try output.write(to: outputURL, atomically: true, encoding: .utf8)
    }

    private static func outputString(for output: NativeGlobalShortcutSmokeOutput) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

struct NativeGlobalShortcutSmokeOutput: Encodable {
    var success: Bool
    var requestedBindingID: String
    var requestedBinding: String
    var keyCode: Int
    var requiredFlagsRawValue: Int
    var accessibilityTrusted: Bool
    var eventTapRunning: Bool
    var eventPostSucceeded: Bool
    var pressedBindingIDs: [String]
    var releasedBindingIDs: [String]
    var observedPressed: Bool
    var observedReleased: Bool
}

private enum NativeGlobalShortcutSmokeError: LocalizedError {
    case invalidBinding(String)

    var errorDescription: String? {
        switch self {
        case let .invalidBinding(binding):
            "Invalid global shortcut smoke binding: \(binding)."
        }
    }
}

private final class LockedGlobalShortcutSmokeEvents: @unchecked Sendable {
    private let lock = NSLock()
    private var pressedBindingIDs: [String] = []
    private var releasedBindingIDs: [String] = []

    func recordPressed(_ bindingID: String) {
        lock.lock()
        pressedBindingIDs.append(bindingID)
        lock.unlock()
    }

    func recordReleased(_ bindingID: String) {
        lock.lock()
        releasedBindingIDs.append(bindingID)
        lock.unlock()
    }

    func snapshot() -> (pressed: [String], released: [String]) {
        lock.lock()
        let snapshot = (pressed: pressedBindingIDs, released: releasedBindingIDs)
        lock.unlock()
        return snapshot
    }
}
