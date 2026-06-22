import Foundation

struct AppPaths {
    static let sharedAppDataIdentifier = "com.pais.handy"
    static let portableMarkerContents = "Handy Portable Mode"

    let appDataDirectory: URL
    let recordingsDirectory: URL
    let modelsDirectory: URL
    let logsDirectory: URL

    static func resolve() throws -> AppPaths {
        let applicationSupportDirectory = try FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        let logsDirectory = try FileManager.default.url(
            for: .libraryDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        .appendingPathComponent("Logs", isDirectory: true)

        if let appDataOverride = ProcessInfo.processInfo.environment["HANDY_APP_DATA_DIR"]?
            .trimmingCharacters(in: .whitespacesAndNewlines),
            appDataOverride.isEmpty == false {
            return try resolve(
                applicationSupportDirectory: URL(fileURLWithPath: appDataOverride, isDirectory: true),
                logsDirectory: URL(fileURLWithPath: appDataOverride, isDirectory: true).appendingPathComponent("logs", isDirectory: true),
                executableDirectory: nil
            )
        }

        return try resolve(
            applicationSupportDirectory: applicationSupportDirectory,
            logsDirectory: logsDirectory,
            executableDirectory: Bundle.main.executableURL?.deletingLastPathComponent()
        )
    }

    static func resolve(
        applicationSupportDirectory: URL,
        logsDirectory: URL,
        executableDirectory: URL?,
        fileManager: FileManager = .default
    ) throws -> AppPaths {
        let portableDataDirectory = try executableDirectory.flatMap {
            try resolvePortableDataDirectory(executableDirectory: $0, fileManager: fileManager)
        }
        let base = portableDataDirectory ?? applicationSupportDirectory
            .appendingPathComponent(sharedAppDataIdentifier, isDirectory: true)
        let resolvedLogsDirectory = portableDataDirectory?
            .appendingPathComponent("logs", isDirectory: true)
            ?? logsDirectory.appendingPathComponent(sharedAppDataIdentifier, isDirectory: true)

        let paths = AppPaths(
            appDataDirectory: base,
            recordingsDirectory: base.appendingPathComponent("recordings", isDirectory: true),
            modelsDirectory: base.appendingPathComponent("models", isDirectory: true),
            logsDirectory: resolvedLogsDirectory
        )

        try [paths.appDataDirectory, paths.recordingsDirectory, paths.modelsDirectory, paths.logsDirectory]
            .forEach { try fileManager.createDirectory(at: $0, withIntermediateDirectories: true) }

        return paths
    }

    static func resolvePortableDataDirectory(executableDirectory: URL, fileManager: FileManager = .default) throws -> URL? {
        let markerURL = executableDirectory.appendingPathComponent("portable")
        let dataDirectory = executableDirectory.appendingPathComponent("Data", isDirectory: true)

        if isValidPortableMarker(at: markerURL) {
            try fileManager.createDirectory(at: dataDirectory, withIntermediateDirectories: true)
            return dataDirectory
        }

        if fileManager.fileExists(atPath: markerURL.path),
           fileManager.fileExists(atPath: dataDirectory.path) {
            try? portableMarkerContents.write(to: markerURL, atomically: true, encoding: .utf8)
            return dataDirectory
        }

        return nil
    }

    static func isValidPortableMarker(at markerURL: URL) -> Bool {
        guard let contents = try? String(contentsOf: markerURL, encoding: .utf8) else {
            return false
        }

        return contents.trimmingCharacters(in: .whitespacesAndNewlines)
            .hasPrefix(portableMarkerContents)
    }
}
