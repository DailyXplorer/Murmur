import Foundation

final class NativeLogStore {
    static let fileName = "handy-native.log"

    private let logsDirectory: URL
    private let fileManager: FileManager
    private let now: () -> Date
    private let timestampFormatter: ISO8601DateFormatter

    init(
        logsDirectory: URL,
        fileManager: FileManager = .default,
        now: @escaping () -> Date = Date.init
    ) {
        self.logsDirectory = logsDirectory
        self.fileManager = fileManager
        self.now = now
        timestampFormatter = ISO8601DateFormatter()
        timestampFormatter.formatOptions = [
            .withInternetDateTime,
            .withFractionalSeconds
        ]
    }

    var logURL: URL {
        logsDirectory.appendingPathComponent(Self.fileName)
    }

    @discardableResult
    func write(
        _ level: NativeLogLevel,
        _ message: String,
        minimumLevel: NativeLogLevel
    ) throws -> Bool {
        guard minimumLevel.allows(level) else {
            return false
        }

        try fileManager.createDirectory(
            at: logsDirectory,
            withIntermediateDirectories: true
        )
        let line = formattedLine(level: level, message: message)
        let data = Data(line.utf8)

        if fileManager.fileExists(atPath: logURL.path) {
            let handle = try FileHandle(forWritingTo: logURL)
            do {
                try handle.seekToEnd()
                try handle.write(contentsOf: data)
                try handle.close()
            } catch {
                try? handle.close()
                throw error
            }
        } else {
            try data.write(to: logURL, options: [.atomic])
        }

        return true
    }

    private func formattedLine(level: NativeLogLevel, message: String) -> String {
        let timestamp = timestampFormatter.string(from: now())
        return "\(timestamp) [\(level.title.uppercased())] \(sanitized(message))\n"
    }

    private func sanitized(_ message: String) -> String {
        let singleLine = message
            .components(separatedBy: .newlines)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { $0.isEmpty == false }
            .joined(separator: " ")
        if singleLine.count <= 2_000 {
            return singleLine
        }

        return "\(singleLine.prefix(2_000))..."
    }
}
