import Foundation

enum NativeUpdateInstallScriptSmokeRunner {
    static func runSynchronouslyAndExit(_ request: NativeUpdateInstallScriptSmokeRequest) -> Never {
        let semaphore = DispatchSemaphore(value: 0)
        let exitCode = LockedUpdateInstallScriptExitCode()

        Task.detached {
            do {
                let output = try await run(request)
                try write(output: output, to: request.outputPath)
                if request.outputPath == nil {
                    FileHandle.standardOutput.writeLine(try outputString(for: output))
                }
                exitCode.set(0)
            } catch {
                FileHandle.standardError.writeLine(error.localizedDescription)
                exitCode.set(1)
            }
            semaphore.signal()
        }

        semaphore.wait()
        exit(Int32(exitCode.value))
    }

    private static func run(_ request: NativeUpdateInstallScriptSmokeRequest) async throws -> NativeUpdateInstallScriptSmokeOutput {
        let fileManager = FileManager.default
        let root = fileManager.temporaryDirectory
            .appendingPathComponent("handy-update-install-smoke-\(UUID().uuidString)", isDirectory: true)
        let targetParent = root.appendingPathComponent("target-parent", isDirectory: true)

        do {
            try fileManager.createDirectory(at: root, withIntermediateDirectories: true)
            let sourceDirectory = root.appendingPathComponent("zip-source", isDirectory: true)
            let sourceApp = sourceDirectory.appendingPathComponent("Handy.app", isDirectory: true)
            try createFakeAppBundle(at: sourceApp)
            let zipURL = root.appendingPathComponent("Handy_\(sanitizedFileName(request.version))_aarch64.app.zip")
            try runProcess(
                executable: "/usr/bin/ditto",
                arguments: ["-c", "-k", "--norsrc", "--noextattr", sourceDirectory.path, zipURL.path]
            )

            try fileManager.createDirectory(at: targetParent, withIntermediateDirectories: true)
            let currentApp = targetParent.appendingPathComponent("Current Handy.app", isDirectory: true)
            try createFakeAppBundle(at: currentApp)
            if request.protectedTargetParent {
                try fileManager.setAttributes([.posixPermissions: 0o555], ofItemAtPath: targetParent.path)
            }

            let update = UpdateInfo(
                version: request.version,
                notes: nil,
                releaseURL: URL(string: "https://updates.example.test/\(zipURL.lastPathComponent)")!
            )
            let artifact = try await UpdateInstallationService.prepareForInstallation(
                artifactURL: zipURL,
                update: update,
                workDirectory: root.appendingPathComponent("updates", isDirectory: true),
                currentAppBundleURL: currentApp,
                currentProcessID: 12_345
            )

            let scriptURL = try unwrap(artifact.installerScriptURL, message: "Prepared update did not include an installer script.")
            let script = try String(contentsOf: scriptURL, encoding: .utf8)
            let shellCheck = runProcessForStatus(executable: "/bin/sh", arguments: ["-n", scriptURL.path])
            let targetParentWritable = runProcessForStatus(
                executable: "/bin/sh",
                arguments: ["-c", "test -w \"$1\"", "sh", targetParent.path]
            ) == 0

            try fileManager.setAttributes([.posixPermissions: 0o755], ofItemAtPath: targetParent.path)
            try? fileManager.removeItem(at: root)

            let helperSection = scriptSection(
                in: script,
                after: "cat > \"$INSTALL_HELPER\" <<'HANDY_INSTALL_HELPER'",
                before: "HANDY_INSTALL_HELPER\n\nchmod 700"
            )
            return NativeUpdateInstallScriptSmokeOutput(
                success: artifact.canInstallAndRelaunch &&
                    artifact.preparedAppBundleURL != nil &&
                    shellCheck == 0,
                version: request.version,
                protectedTargetParent: request.protectedTargetParent,
                artifactFileName: zipURL.lastPathComponent,
                preparedAppBundleName: artifact.preparedAppBundleURL?.lastPathComponent,
                installerScriptName: scriptURL.lastPathComponent,
                installerScriptShellCheckStatus: shellCheck,
                targetParentWritable: targetParentWritable,
                scriptContainsAdminBranch: script.contains("with administrator privileges"),
                scriptContainsWritableBranch: script.contains("if [ -w \"$TARGET_PARENT\" ]; then"),
                scriptContainsRollback: script.contains("BACKUP_APP=") &&
                    script.contains("mv \"$BACKUP_APP\" \"$TARGET_APP\""),
                scriptContainsUserRelaunch: script.contains("rm -f \"$INSTALL_HELPER\" \"$0\"\n/usr/bin/open -n \"$TARGET_APP\""),
                helperContainsRelaunch: helperSection?.contains("/usr/bin/open -n") ?? false
            )
        } catch {
            try? fileManager.setAttributes([.posixPermissions: 0o755], ofItemAtPath: targetParent.path)
            try? fileManager.removeItem(at: root)
            throw error
        }
    }

    private static func createFakeAppBundle(at appURL: URL) throws {
        let executableURL = appURL
            .appendingPathComponent("Contents", isDirectory: true)
            .appendingPathComponent("MacOS", isDirectory: true)
            .appendingPathComponent("Handy", isDirectory: false)
        try FileManager.default.createDirectory(
            at: executableURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try Data("binary".utf8).write(to: executableURL)
    }

    private static func runProcess(executable: String, arguments: [String]) throws {
        let status = runProcessForStatus(executable: executable, arguments: arguments)
        guard status == 0 else {
            throw NativeUpdateInstallScriptSmokeError.processFailed(executable, status)
        }
    }

    private static func runProcessForStatus(executable: String, arguments: [String]) -> Int32 {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        do {
            try process.run()
            process.waitUntilExit()
            return process.terminationStatus
        } catch {
            return 127
        }
    }

    private static func unwrap<T>(_ value: T?, message: String) throws -> T {
        guard let value else {
            throw NativeUpdateInstallScriptSmokeError.invalidPreparedArtifact(message)
        }
        return value
    }

    private static func scriptSection(in value: String, after startMarker: String, before endMarker: String) -> String? {
        guard let startRange = value.range(of: startMarker),
              let endRange = value.range(of: endMarker, range: startRange.upperBound..<value.endIndex)
        else {
            return nil
        }
        return String(value[startRange.upperBound..<endRange.lowerBound])
    }

    private static func sanitizedFileName(_ value: String) -> String {
        let sanitized = value.map { character in
            character.isLetter || character.isNumber || character == "." || character == "-" ? character : "-"
        }
        let result = String(sanitized)
        return result.isEmpty ? "update" : result
    }

    private static func write(output: NativeUpdateInstallScriptSmokeOutput, to path: String?) throws {
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

    private static func outputString(for output: NativeUpdateInstallScriptSmokeOutput) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

struct NativeUpdateInstallScriptSmokeOutput: Encodable {
    var success: Bool
    var version: String
    var protectedTargetParent: Bool
    var artifactFileName: String
    var preparedAppBundleName: String?
    var installerScriptName: String?
    var installerScriptShellCheckStatus: Int32
    var targetParentWritable: Bool
    var scriptContainsAdminBranch: Bool
    var scriptContainsWritableBranch: Bool
    var scriptContainsRollback: Bool
    var scriptContainsUserRelaunch: Bool
    var helperContainsRelaunch: Bool
}

private final class LockedUpdateInstallScriptExitCode: @unchecked Sendable {
    private let lock = NSLock()
    private var storage = 1

    var value: Int {
        lock.withLock { storage }
    }

    func set(_ value: Int) {
        lock.withLock {
            storage = value
        }
    }
}

private enum NativeUpdateInstallScriptSmokeError: LocalizedError {
    case processFailed(String, Int32)
    case invalidPreparedArtifact(String)

    var errorDescription: String? {
        switch self {
        case let .processFailed(executable, status):
            "\(executable) exited with status \(status)."
        case let .invalidPreparedArtifact(message):
            message
        }
    }
}
