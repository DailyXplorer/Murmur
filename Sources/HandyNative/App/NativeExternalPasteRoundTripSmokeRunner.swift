import AppKit
import Foundation

enum NativeExternalPasteRoundTripSmokeRunner {
    @MainActor
    static func runSynchronouslyAndExit(_ request: NativeExternalPasteRoundTripSmokeRequest) -> Never {
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
    static func output(for request: NativeExternalPasteRoundTripSmokeRequest) async throws -> NativeExternalPasteRoundTripSmokeOutput {
        let workingDirectory = FileManager.default
            .temporaryDirectory
            .appendingPathComponent("handy-external-paste-roundtrip-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: workingDirectory, withIntermediateDirectories: true)

        let targetOutputURL = workingDirectory.appendingPathComponent("target.json")
        let targetReadyURL = workingDirectory.appendingPathComponent("ready")
        let targetStandardOutput = Pipe()
        let targetStandardError = Pipe()
        let expectedText = expectedPreparedText(for: request)
        let targetProcess = try startTargetProcess(
            request: request,
            expectedText: expectedText,
            outputURL: targetOutputURL,
            readyURL: targetReadyURL,
            standardOutput: targetStandardOutput,
            standardError: targetStandardError
        )
        defer {
            if targetProcess.isRunning {
                targetProcess.terminate()
            }
        }

        try await waitForReadyFile(targetReadyURL, process: targetProcess)

        let pasteOutput = try await NativePasteSmokeRunner.output(
            for: NativePasteSmokeRequest(
                text: request.text,
                pasteMethod: request.pasteMethod,
                clipboardHandling: request.clipboardHandling,
                pasteDelayMilliseconds: request.pasteDelayMilliseconds,
                startDelayMilliseconds: request.startDelayMilliseconds,
                appendTrailingSpace: request.appendTrailingSpace,
                autoSubmitKey: request.autoSubmitKey,
                targetWindow: false,
                activationProcessIdentifier: targetProcess.processIdentifier,
                outputPath: nil
            )
        )

        let targetTerminationStatus = await waitForExit(
            targetProcess,
            timeoutMilliseconds: request.durationMilliseconds + 2_000
        )
        if targetProcess.isRunning {
            targetProcess.terminate()
        }

        let targetOutput = try readTargetOutput(at: targetOutputURL)
        let targetStdout = readPipe(targetStandardOutput)
        let targetStderr = readPipe(targetStandardError)
        let success = pasteOutput.success &&
            targetOutput.matchedExpectedText == true &&
            targetTerminationStatus == 0

        return NativeExternalPasteRoundTripSmokeOutput(
            success: success,
            requestedText: request.text,
            expectedText: expectedText,
            paste: pasteOutput,
            target: targetOutput,
            targetProcessIdentifier: targetProcess.processIdentifier,
            targetTerminationStatus: targetTerminationStatus,
            targetStandardOutput: targetStdout.nilIfEmpty,
            targetStandardError: targetStderr.nilIfEmpty
        )
    }

    private static func startTargetProcess(
        request: NativeExternalPasteRoundTripSmokeRequest,
        expectedText: String,
        outputURL: URL,
        readyURL: URL,
        standardOutput: Pipe,
        standardError: Pipe
    ) throws -> Process {
        let process = Process()
        process.executableURL = try executableURL()
        process.arguments = [
            "--smoke-external-paste-target",
            outputURL.path,
            "--smoke-external-paste-ready",
            readyURL.path,
            "--smoke-external-paste-expected",
            expectedText,
            "--smoke-external-paste-duration-ms",
            "\(request.durationMilliseconds)",
        ]
        process.standardOutput = standardOutput
        process.standardError = standardError
        try process.run()
        return process
    }

    private static func executableURL() throws -> URL {
        if let executableURL = Bundle.main.executableURL {
            return executableURL
        }

        let argumentZero = CommandLine.arguments[0]
        guard argumentZero.isEmpty == false else {
            throw NativeExternalPasteRoundTripSmokeError.executableUnavailable
        }
        return URL(fileURLWithPath: argumentZero)
    }

    @MainActor
    private static func waitForReadyFile(_ readyURL: URL, process: Process) async throws {
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            if FileManager.default.fileExists(atPath: readyURL.path) {
                return
            }
            if process.isRunning == false {
                throw NativeExternalPasteRoundTripSmokeError.targetExitedBeforeReady(process.terminationStatus)
            }
            try await Task.sleep(for: .milliseconds(100))
        }

        throw NativeExternalPasteRoundTripSmokeError.targetDidNotBecomeReady
    }

    private static func waitForExit(_ process: Process, timeoutMilliseconds: Int) async -> Int32 {
        let deadline = Date().addingTimeInterval(TimeInterval(timeoutMilliseconds) / 1_000)
        while Date() < deadline {
            if process.isRunning == false {
                return process.terminationStatus
            }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return process.isRunning ? -1 : process.terminationStatus
    }

    private static func expectedPreparedText(for request: NativeExternalPasteRoundTripSmokeRequest) -> String {
        var settings = AppSettings.defaults
        settings.pasteMethod = request.pasteMethod
        settings.clipboardHandling = request.clipboardHandling
        settings.pasteDelayMilliseconds = request.pasteDelayMilliseconds
        settings.appendTrailingSpace = request.appendTrailingSpace
        if let autoSubmitKey = request.autoSubmitKey {
            settings.autoSubmitAfterPaste = true
            settings.autoSubmitKey = autoSubmitKey
        }
        return PasteService.preparedText(request.text, options: PasteOutputOptions(settings: settings))
    }

    private static func readTargetOutput(at url: URL) throws -> NativeExternalPasteTargetSmokeOutput {
        let data = try Data(contentsOf: url)
        return try JSONDecoder().decode(NativeExternalPasteTargetSmokeOutput.self, from: data)
    }

    private static func readPipe(_ pipe: Pipe) -> String {
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        return String(data: data, encoding: .utf8) ?? ""
    }

    private static func write(_ output: String, to path: String) throws {
        let outputURL = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try output.write(to: outputURL, atomically: true, encoding: .utf8)
    }

    private static func outputString(for output: NativeExternalPasteRoundTripSmokeOutput) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

struct NativeExternalPasteRoundTripSmokeOutput: Encodable {
    var success: Bool
    var requestedText: String
    var expectedText: String
    var paste: NativePasteSmokeOutput
    var target: NativeExternalPasteTargetSmokeOutput
    var targetProcessIdentifier: Int32
    var targetTerminationStatus: Int32
    var targetStandardOutput: String?
    var targetStandardError: String?
}

private enum NativeExternalPasteRoundTripSmokeError: LocalizedError {
    case executableUnavailable
    case targetDidNotBecomeReady
    case targetExitedBeforeReady(Int32)

    var errorDescription: String? {
        switch self {
        case .executableUnavailable:
            "Unable to locate the Handy executable for external paste roundtrip smoke."
        case .targetDidNotBecomeReady:
            "External paste target did not become ready before timeout."
        case let .targetExitedBeforeReady(status):
            "External paste target exited before becoming ready (status: \(status))."
        }
    }
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
