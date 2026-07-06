import Foundation

enum NativeRemoteControlSmokeRunner {
    static func runListenerSynchronouslyAndExit(_ request: NativeRemoteControlListenerSmokeRequest) -> Never {
        do {
            let output = try runListener(request)
            try write(output: output, to: request.outputPath)
            if request.outputPath == nil {
                FileHandle.standardOutput.writeLine(try outputString(for: output))
            }
            exit(output.success ? 0 : 1)
        } catch {
            FileHandle.standardError.writeLine(error.localizedDescription)
            exit(1)
        }
    }

    static func runSenderSynchronouslyAndExit(_ request: NativeRemoteControlSendSmokeRequest) -> Never {
        do {
            let output = runSender(request)
            try write(output: output, to: request.outputPath)
            if request.outputPath == nil {
                FileHandle.standardOutput.writeLine(try outputString(for: output))
            }
            exit(output.sent ? 0 : 1)
        } catch {
            FileHandle.standardError.writeLine(error.localizedDescription)
            exit(1)
        }
    }

    private static func runListener(_ request: NativeRemoteControlListenerSmokeRequest) throws -> NativeRemoteControlListenerSmokeOutput {
        let receivedCommands = LockedRemoteControlCommands()
        let service = NativeRemoteControlService()
        service.start { command in
            receivedCommands.append(command)
        }
        defer {
            service.stop()
        }

        let childOutputURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("murmur-remote-control-child-\(UUID().uuidString).json")
        defer {
            try? FileManager.default.removeItem(at: childOutputURL)
        }

        let child = try senderProcess(for: request, outputURL: childOutputURL)
        try child.run()

        let deadline = Date().addingTimeInterval(TimeInterval(request.timeoutMilliseconds) / 1_000)
        while Date() < deadline {
            if receivedCommands.commands.contains(request.command) {
                break
            }
            RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.05))
        }

        waitForProcessExit(child, timeout: senderExitTimeout(for: request.senderLaunchMethod))
        if child.isRunning {
            child.terminate()
        }
        child.waitUntilExit()
        waitForSenderOutput(at: childOutputURL, timeout: 2)

        let senderOutput = try readSenderOutputIfAvailable(from: childOutputURL)
        let commands = receivedCommands.commands
        let receivedExpectedCommand = commands.contains(request.command)
        let success = receivedExpectedCommand &&
            child.terminationStatus == 0 &&
            (senderOutput?.sent ?? false)

        return NativeRemoteControlListenerSmokeOutput(
            success: success,
            expectedCommand: request.command.rawValue,
            observedCommands: commands.map(\.rawValue),
            receivedExpectedCommand: receivedExpectedCommand,
            timeoutMilliseconds: request.timeoutMilliseconds,
            senderLaunchMethod: request.senderLaunchMethod.rawValue,
            senderLaunchPath: senderLaunchPath(for: request.senderLaunchMethod),
            bundleIdentifier: Bundle.main.bundleIdentifier,
            listenerProcessIdentifier: getpid(),
            childTerminationStatus: child.terminationStatus,
            senderOutput: senderOutput
        )
    }

    private static func senderProcess(
        for request: NativeRemoteControlListenerSmokeRequest,
        outputURL: URL
    ) throws -> Process {
        let process = Process()
        switch request.senderLaunchMethod {
        case .executable:
            process.executableURL = try currentExecutableURL()
            process.arguments = remoteControlSenderArguments(for: request.command, outputURL: outputURL)
        case .launchServices:
            process.executableURL = URL(fileURLWithPath: "/usr/bin/open")
            process.arguments = [
                "-n",
                try currentAppBundleURL().path,
                "--args",
            ] + remoteControlSenderArguments(for: request.command, outputURL: outputURL)
        }
        return process
    }

    private static func remoteControlSenderArguments(
        for command: RemoteControlCommand,
        outputURL: URL
    ) -> [String] {
        [
            "--smoke-remote-control-send",
            command.rawValue,
            "--smoke-output-json",
            outputURL.path,
        ]
    }

    private static func runSender(_ request: NativeRemoteControlSendSmokeRequest) -> NativeRemoteControlSendSmokeOutput {
        let bundleIdentifier = Bundle.main.bundleIdentifier
        let processIdentifier = getpid()
        let peerAvailableBeforeSend = NativeRemoteControlService.hasRunningPeer(
            bundleIdentifier: bundleIdentifier,
            currentProcessIdentifier: processIdentifier
        )
        let sent = NativeRemoteControlService.sendToRunningInstance(
            request.command,
            bundleIdentifier: bundleIdentifier,
            currentProcessIdentifier: processIdentifier
        )

        return NativeRemoteControlSendSmokeOutput(
            command: request.command.rawValue,
            sent: sent,
            peerAvailableBeforeSend: peerAvailableBeforeSend,
            bundleIdentifier: bundleIdentifier,
            senderProcessIdentifier: processIdentifier
        )
    }

    private static func currentExecutableURL() throws -> URL {
        if let executableURL = Bundle.main.executableURL {
            return executableURL
        }

        let firstArgument = CommandLine.arguments.first ?? ""
        guard firstArgument.isEmpty == false else {
            throw NativeRemoteControlSmokeError.missingExecutable
        }
        return URL(fileURLWithPath: firstArgument)
    }

    private static func currentAppBundleURL() throws -> URL {
        let bundleURL = Bundle.main.bundleURL
        guard bundleURL.pathExtension == "app" else {
            throw NativeRemoteControlSmokeError.missingAppBundle(bundleURL.path)
        }
        return bundleURL
    }

    private static func readSenderOutputIfAvailable(from url: URL) throws -> NativeRemoteControlSendSmokeOutput? {
        guard FileManager.default.fileExists(atPath: url.path) else {
            return nil
        }
        let data = try Data(contentsOf: url)
        return try JSONDecoder().decode(NativeRemoteControlSendSmokeOutput.self, from: data)
    }

    private static func waitForSenderOutput(at url: URL, timeout: TimeInterval) {
        let deadline = Date().addingTimeInterval(timeout)
        while FileManager.default.fileExists(atPath: url.path) == false && Date() < deadline {
            RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.05))
        }
    }

    private static func waitForProcessExit(_ process: Process, timeout: TimeInterval) {
        let deadline = Date().addingTimeInterval(timeout)
        while process.isRunning && Date() < deadline {
            RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.05))
        }
    }

    private static func senderExitTimeout(for launchMethod: NativeRemoteControlSmokeLaunchMethod) -> TimeInterval {
        switch launchMethod {
        case .executable:
            2
        case .launchServices:
            5
        }
    }

    private static func senderLaunchPath(for launchMethod: NativeRemoteControlSmokeLaunchMethod) -> String? {
        switch launchMethod {
        case .executable:
            try? currentExecutableURL().path
        case .launchServices:
            try? currentAppBundleURL().path
        }
    }

    private static func write<T: Encodable>(output: T, to path: String?) throws {
        guard let path else {
            return
        }

        let outputURL = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try outputString(for: output).write(to: outputURL, atomically: true, encoding: .utf8)
    }

    private static func outputString<T: Encodable>(for output: T) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

struct NativeRemoteControlListenerSmokeOutput: Encodable {
    var success: Bool
    var expectedCommand: String
    var observedCommands: [String]
    var receivedExpectedCommand: Bool
    var timeoutMilliseconds: Int
    var senderLaunchMethod: String
    var senderLaunchPath: String?
    var bundleIdentifier: String?
    var listenerProcessIdentifier: Int32
    var childTerminationStatus: Int32
    var senderOutput: NativeRemoteControlSendSmokeOutput?
}

struct NativeRemoteControlSendSmokeOutput: Codable {
    var command: String
    var sent: Bool
    var peerAvailableBeforeSend: Bool
    var bundleIdentifier: String?
    var senderProcessIdentifier: Int32
}

private final class LockedRemoteControlCommands: @unchecked Sendable {
    private let lock = NSLock()
    private var storage: [RemoteControlCommand] = []

    var commands: [RemoteControlCommand] {
        lock.withLock { storage }
    }

    func append(_ command: RemoteControlCommand) {
        lock.withLock {
            storage.append(command)
        }
    }
}

private enum NativeRemoteControlSmokeError: LocalizedError {
    case missingExecutable
    case missingAppBundle(String)

    var errorDescription: String? {
        switch self {
        case .missingExecutable:
            "Unable to locate the current Murmur executable for remote-control smoke validation."
        case let .missingAppBundle(path):
            "LaunchServices remote-control smoke requires a .app bundle; got \(path)."
        }
    }
}
