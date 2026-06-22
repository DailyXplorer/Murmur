import Foundation

enum UpdateInstallationServiceError: LocalizedError, Equatable {
    case invalidArtifactURL
    case httpStatus(Int)
    case emptyArtifact
    case cannotCreateTemporaryFile
    case unsupportedArtifact(String)
    case extractionFailed(String)
    case missingAppBundle
    case cannotLocateCurrentAppBundle

    var errorDescription: String? {
        switch self {
        case .invalidArtifactURL:
            "Invalid update artifact URL."
        case let .httpStatus(status):
            "Update download failed with status \(status)."
        case .emptyArtifact:
            "The update artifact was empty."
        case .cannotCreateTemporaryFile:
            "Unable to create a temporary update download file."
        case let .unsupportedArtifact(fileName):
            "Unsupported update artifact: \(fileName)."
        case let .extractionFailed(message):
            "Unable to extract the update artifact: \(message)."
        case .missingAppBundle:
            "The update artifact did not contain a macOS app bundle."
        case .cannotLocateCurrentAppBundle:
            "Unable to locate the current app bundle."
        }
    }
}

enum UpdateInstallationService {
    private struct MountedDiskImage {
        var mountPoint: URL
        var deviceIdentifier: String?
    }

    static func download(
        update: UpdateInfo,
        destinationDirectory: URL,
        urlSession: URLSession = .shared,
        fileManager: FileManager = .default,
        progress: (@Sendable (UpdateInstallationProgress) async -> Void)? = nil
    ) async throws -> URL {
        guard update.releaseURL.scheme?.hasPrefix("http") == true else {
            throw UpdateInstallationServiceError.invalidArtifactURL
        }

        var request = URLRequest(url: update.releaseURL)
        request.httpMethod = "GET"
        request.cachePolicy = .reloadIgnoringLocalCacheData

        let (bytes, response) = try await urlSession.bytes(for: request)
        if let httpResponse = response as? HTTPURLResponse,
           !(200..<300).contains(httpResponse.statusCode) {
            throw UpdateInstallationServiceError.httpStatus(httpResponse.statusCode)
        }

        let expectedLength = response.expectedContentLength > 0 ? response.expectedContentLength : nil
        let fileName = artifactFileName(for: update)
        try fileManager.createDirectory(at: destinationDirectory, withIntermediateDirectories: true)

        let destinationURL = destinationDirectory.appendingPathComponent(fileName, isDirectory: false)
        let temporaryURL = destinationDirectory.appendingPathComponent(".\(fileName).download", isDirectory: false)
        try? fileManager.removeItem(at: temporaryURL)

        guard fileManager.createFile(atPath: temporaryURL.path, contents: nil),
              let fileHandle = FileHandle(forWritingAtPath: temporaryURL.path)
        else {
            throw UpdateInstallationServiceError.cannotCreateTemporaryFile
        }

        var downloadedBytes: Int64 = 0
        var pendingBytes: [UInt8] = []
        pendingBytes.reserveCapacity(64 * 1_024)

        do {
            await progress?(.init(downloadedBytes: 0, totalBytes: expectedLength))
            for try await byte in bytes {
                pendingBytes.append(byte)
                downloadedBytes += 1

                if pendingBytes.count >= 64 * 1_024 {
                    fileHandle.write(Data(pendingBytes))
                    pendingBytes.removeAll(keepingCapacity: true)
                    await progress?(.init(downloadedBytes: downloadedBytes, totalBytes: expectedLength))
                }
            }

            if pendingBytes.isEmpty == false {
                fileHandle.write(Data(pendingBytes))
            }
            try fileHandle.close()
        } catch {
            try? fileHandle.close()
            try? fileManager.removeItem(at: temporaryURL)
            throw error
        }

        guard downloadedBytes > 0 else {
            try? fileManager.removeItem(at: temporaryURL)
            throw UpdateInstallationServiceError.emptyArtifact
        }

        await progress?(.init(downloadedBytes: downloadedBytes, totalBytes: expectedLength))
        try? fileManager.removeItem(at: destinationURL)
        try fileManager.moveItem(at: temporaryURL, to: destinationURL)
        return destinationURL
    }

    static func prepareForInstallation(
        artifactURL: URL,
        update: UpdateInfo,
        workDirectory: URL,
        currentAppBundleURL: URL? = Bundle.main.bundleURL,
        currentProcessID: Int32 = ProcessInfo.processInfo.processIdentifier,
        fileManager: FileManager = .default
    ) async throws -> DownloadedUpdateArtifact {
        let preparedDirectory = workDirectory
            .appendingPathComponent("prepared-\(sanitizedVersion(update.version))", isDirectory: true)
        try? fileManager.removeItem(at: preparedDirectory)
        try fileManager.createDirectory(at: preparedDirectory, withIntermediateDirectories: true)

        let preparedAppBundleURL: URL?
        if artifactURL.pathExtension == "app",
           isAppBundle(artifactURL, fileManager: fileManager) {
            preparedAppBundleURL = artifactURL
        } else if artifactURL.pathExtension == "zip" {
            try extractZip(artifactURL, to: preparedDirectory)
            preparedAppBundleURL = findAppBundle(in: preparedDirectory, fileManager: fileManager)
        } else if artifactURL.lastPathComponent.hasSuffix(".tar.gz") || artifactURL.lastPathComponent.hasSuffix(".tgz") {
            try extractTarGzip(artifactURL, to: preparedDirectory)
            preparedAppBundleURL = findAppBundle(in: preparedDirectory, fileManager: fileManager)
        } else if artifactURL.pathExtension == "dmg" {
            preparedAppBundleURL = try extractDiskImage(
                artifactURL,
                to: preparedDirectory,
                fileManager: fileManager
            )
        } else {
            let manualArtifact = DownloadedUpdateArtifact(
                artifactURL: artifactURL,
                preparedAppBundleURL: nil,
                installerScriptURL: nil
            )
            return manualArtifact
        }

        guard let preparedAppBundleURL else {
            throw UpdateInstallationServiceError.missingAppBundle
        }

        guard let currentAppBundleURL else {
            throw UpdateInstallationServiceError.cannotLocateCurrentAppBundle
        }

        let scriptURL = try createReplacementScript(
            stagedAppBundleURL: preparedAppBundleURL,
            currentAppBundleURL: currentAppBundleURL,
            scriptDirectory: workDirectory,
            currentProcessID: currentProcessID
        )

        return DownloadedUpdateArtifact(
            artifactURL: artifactURL,
            preparedAppBundleURL: preparedAppBundleURL,
            installerScriptURL: scriptURL
        )
    }

    static func artifactFileName(for update: UpdateInfo) -> String {
        let lastPathComponent = update.releaseURL.lastPathComponent
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if lastPathComponent.isEmpty == false,
           lastPathComponent != "/",
           URL(fileURLWithPath: lastPathComponent).pathExtension.isEmpty == false {
            return lastPathComponent
        }

        let version = update.version
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "/", with: "-")
        return "Handy-\(version.isEmpty ? "update" : version).dmg"
    }

    static func createReplacementScript(
        stagedAppBundleURL: URL,
        currentAppBundleURL: URL,
        scriptDirectory: URL,
        currentProcessID: Int32,
        fileManager: FileManager = .default
    ) throws -> URL {
        try fileManager.createDirectory(at: scriptDirectory, withIntermediateDirectories: true)
        let scriptURL = scriptDirectory.appendingPathComponent("install-handy-update.sh", isDirectory: false)
        let helperScriptURL = scriptDirectory.appendingPathComponent("install-handy-update-helper.sh", isDirectory: false)
        let adminInstallCommand = "/bin/sh \(shellQuoted(helperScriptURL.path))"
        let script = """
        #!/bin/sh
        set -eu

        APP_PID=\(currentProcessID)
        STAGED_APP=\(shellQuoted(stagedAppBundleURL.path))
        TARGET_APP=\(shellQuoted(currentAppBundleURL.path))
        INSTALL_HELPER=\(shellQuoted(helperScriptURL.path))
        TARGET_PARENT="$(dirname "$TARGET_APP")"

        while kill -0 "$APP_PID" 2>/dev/null; do
          sleep 0.2
        done

        cat > "$INSTALL_HELPER" <<'HANDY_INSTALL_HELPER'
        #!/bin/sh
        set -eu

        STAGED_APP=\(shellQuoted(stagedAppBundleURL.path))
        TARGET_APP=\(shellQuoted(currentAppBundleURL.path))
        TARGET_PARENT="$(dirname "$TARGET_APP")"
        TMP_APP="${TARGET_APP}.updating.$$"
        BACKUP_APP="${TARGET_APP}.previous.$$"

        mkdir -p "$TARGET_PARENT"
        rm -rf "$TMP_APP" "$BACKUP_APP"
        /usr/bin/ditto --norsrc --noextattr --noqtn "$STAGED_APP" "$TMP_APP"
        if [ -d "$TARGET_APP" ]; then
          mv "$TARGET_APP" "$BACKUP_APP"
        fi
        if ! mv "$TMP_APP" "$TARGET_APP"; then
          if [ -d "$BACKUP_APP" ] && [ ! -d "$TARGET_APP" ]; then
            mv "$BACKUP_APP" "$TARGET_APP"
          fi
          exit 1
        fi
        /usr/bin/xattr -dr com.apple.quarantine "$TARGET_APP" 2>/dev/null || true
        rm -rf "$BACKUP_APP" "$STAGED_APP"
        HANDY_INSTALL_HELPER

        chmod 700 "$INSTALL_HELPER"

        if [ -w "$TARGET_PARENT" ]; then
          if ! /bin/sh "$INSTALL_HELPER"; then
            [ -d "$TARGET_APP" ] && /usr/bin/open -n "$TARGET_APP"
            exit 1
          fi
        else
          if ! /usr/bin/osascript <<'HANDY_ADMIN_OSASCRIPT'
        do shell script \(appleScriptStringLiteral(adminInstallCommand)) with administrator privileges
        HANDY_ADMIN_OSASCRIPT
          then
            [ -d "$TARGET_APP" ] && /usr/bin/open -n "$TARGET_APP"
            exit 1
          fi
        fi

        rm -f "$INSTALL_HELPER" "$0"
        /usr/bin/open -n "$TARGET_APP"
        """

        try script.write(to: scriptURL, atomically: true, encoding: .utf8)
        try fileManager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: scriptURL.path)
        return scriptURL
    }

    static func launchReplacementScript(_ scriptURL: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = [scriptURL.path]
        try process.run()
    }

    private static func extractZip(_ artifactURL: URL, to destinationURL: URL) throws {
        try run(
            executable: "/usr/bin/ditto",
            arguments: ["-x", "-k", "--norsrc", "--noextattr", "--noqtn", artifactURL.path, destinationURL.path]
        )
    }

    private static func extractTarGzip(_ artifactURL: URL, to destinationURL: URL) throws {
        try run(
            executable: "/usr/bin/tar",
            arguments: ["-xzf", artifactURL.path, "-C", destinationURL.path]
        )
    }

    private static func extractDiskImage(
        _ artifactURL: URL,
        to destinationURL: URL,
        fileManager: FileManager
    ) throws -> URL {
        let mountPoint = destinationURL.appendingPathComponent("mount", isDirectory: true)
        try fileManager.createDirectory(at: mountPoint, withIntermediateDirectories: true)
        let mountedImage = try attachDiskImage(artifactURL, mountPoint: mountPoint)

        do {
            guard let mountedAppBundleURL = findAppBundle(in: mountedImage.mountPoint, fileManager: fileManager) else {
                throw UpdateInstallationServiceError.missingAppBundle
            }

            let stagedAppBundleURL = destinationURL.appendingPathComponent(
                mountedAppBundleURL.lastPathComponent,
                isDirectory: true
            )
            try? fileManager.removeItem(at: stagedAppBundleURL)
            try run(
                executable: "/usr/bin/ditto",
                arguments: [
                    "--norsrc",
                    "--noextattr",
                    "--noqtn",
                    mountedAppBundleURL.path,
                    stagedAppBundleURL.path
                ]
            )
            try detachDiskImage(mountedImage)
            return stagedAppBundleURL
        } catch {
            try? detachDiskImage(mountedImage)
            throw error
        }
    }

    private static func attachDiskImage(_ artifactURL: URL, mountPoint: URL) throws -> MountedDiskImage {
        let plistOutput = try runCapturingOutput(
            executable: "/usr/bin/hdiutil",
            arguments: [
                "attach",
                artifactURL.path,
                "-readonly",
                "-nobrowse",
                "-mountpoint",
                mountPoint.path,
                "-plist"
            ]
        )

        guard let plistData = plistOutput.data(using: .utf8),
              let plist = try? PropertyListSerialization.propertyList(from: plistData, options: [], format: nil),
              let plistDictionary = plist as? [String: Any],
              let entities = plistDictionary["system-entities"] as? [[String: Any]]
        else {
            return MountedDiskImage(mountPoint: mountPoint, deviceIdentifier: nil)
        }

        let mountedEntity = entities.first { $0["mount-point"] is String }
        let mountedPath = mountedEntity?["mount-point"] as? String
        let mountedDevice = mountedEntity?["dev-entry"] as? String
        return MountedDiskImage(
            mountPoint: mountedPath.map { URL(fileURLWithPath: $0, isDirectory: true) } ?? mountPoint,
            deviceIdentifier: mountedDevice
        )
    }

    private static func detachDiskImage(_ mountedImage: MountedDiskImage) throws {
        let candidates = [mountedImage.deviceIdentifier, mountedImage.mountPoint.path]
            .compactMap { $0 }
            .filter { $0.isEmpty == false }
        var lastError: Error?

        for attempt in 0..<3 {
            for candidate in candidates {
                do {
                    try run(executable: "/usr/bin/hdiutil", arguments: ["detach", candidate, "-quiet"])
                    return
                } catch {
                    lastError = error
                }
            }

            if attempt < 2 {
                Thread.sleep(forTimeInterval: 0.2)
            }
        }

        for candidate in candidates {
            do {
                try run(executable: "/usr/bin/hdiutil", arguments: ["detach", "-force", candidate, "-quiet"])
                return
            } catch {
                lastError = error
            }
        }

        if let lastError {
            throw lastError
        }
    }

    private static func runCapturingOutput(executable: String, arguments: [String]) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        let outputPipe = Pipe()
        let errorPipe = Pipe()
        process.standardOutput = outputPipe
        process.standardError = errorPipe

        try process.run()
        process.waitUntilExit()

        let outputData = outputPipe.fileHandleForReading.readDataToEndOfFile()
        guard process.terminationStatus == 0 else {
            let errorData = errorPipe.fileHandleForReading.readDataToEndOfFile()
            let message = String(data: errorData, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            throw UpdateInstallationServiceError.extractionFailed(message?.isEmpty == false ? message! : "exit \(process.terminationStatus)")
        }

        return String(data: outputData, encoding: .utf8) ?? ""
    }

    private static func findAppBundle(in directoryURL: URL, fileManager: FileManager) -> URL? {
        if isAppBundle(directoryURL, fileManager: fileManager) {
            return directoryURL
        }

        guard let enumerator = fileManager.enumerator(
            at: directoryURL,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles, .skipsPackageDescendants]
        ) else {
            return nil
        }

        for case let url as URL in enumerator where isAppBundle(url, fileManager: fileManager) {
            return url
        }
        return nil
    }

    private static func isAppBundle(_ url: URL, fileManager: FileManager) -> Bool {
        var isDirectory: ObjCBool = false
        return url.pathExtension == "app" &&
            fileManager.fileExists(atPath: url.path, isDirectory: &isDirectory) &&
            isDirectory.boolValue
    }

    private static func run(executable: String, arguments: [String]) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        let errorPipe = Pipe()
        process.standardError = errorPipe

        try process.run()
        process.waitUntilExit()

        guard process.terminationStatus == 0 else {
            let errorData = errorPipe.fileHandleForReading.readDataToEndOfFile()
            let message = String(data: errorData, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            throw UpdateInstallationServiceError.extractionFailed(message?.isEmpty == false ? message! : "exit \(process.terminationStatus)")
        }
    }

    private static func shellQuoted(_ value: String) -> String {
        "'\(value.replacingOccurrences(of: "'", with: "'\\''"))'"
    }

    private static func appleScriptStringLiteral(_ value: String) -> String {
        let escaped = value
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        return "\"\(escaped)\""
    }

    private static func sanitizedVersion(_ version: String) -> String {
        let sanitized = version
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .map { character in
                character.isLetter || character.isNumber || character == "." || character == "-" ? character : "-"
            }
        let value = String(sanitized)
        return value.isEmpty ? "update" : value
    }
}
