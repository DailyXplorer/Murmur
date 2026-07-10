import Foundation
import SQLite3

enum HistoryStoreError: LocalizedError {
    case openDatabase(String)
    case prepare(String)
    case step(String)
    case missingEntry(Int64)

    var errorDescription: String? {
        switch self {
        case let .openDatabase(message):
            "Unable to open history database: \(message)"
        case let .prepare(message):
            "Unable to prepare history query: \(message)"
        case let .step(message):
            "Unable to update history: \(message)"
        case let .missingEntry(id):
            "History entry \(id) was not found."
        }
    }
}

final class HistoryStore {
    private static let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

    let databaseURL: URL
    let recordingsDirectory: URL

    convenience init(paths: AppPaths) throws {
        try self.init(
            databaseURL: paths.appDataDirectory.appendingPathComponent("history.db"),
            recordingsDirectory: paths.recordingsDirectory
        )
    }

    init(databaseURL: URL, recordingsDirectory: URL) throws {
        self.databaseURL = databaseURL
        self.recordingsDirectory = recordingsDirectory
        try FileManager.default.createDirectory(
            at: recordingsDirectory,
            withIntermediateDirectories: true
        )
        try initializeDatabase()
    }

    func saveEntry(
        fileName: String,
        transcriptionText: String,
        postProcessRequested: Bool = false,
        postProcessedText: String? = nil,
        postProcessPrompt: String? = nil,
        timestamp: Date = Date()
    ) throws -> HistoryEntry {
        let timestampSeconds = Int64(timestamp.timeIntervalSince1970)
        let title = Self.formatTitle(timestamp: timestamp)
        let database = try openDatabase()
        defer { sqlite3_close(database) }

        let sql = """
        INSERT INTO transcription_history (
            file_name,
            timestamp,
            saved,
            title,
            transcription_text,
            post_processed_text,
            post_process_prompt,
            post_process_requested
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        """
        let statement = try prepare(sql, in: database)
        defer { sqlite3_finalize(statement) }

        bind(fileName, at: 1, in: statement)
        sqlite3_bind_int64(statement, 2, timestampSeconds)
        sqlite3_bind_int(statement, 3, 0)
        bind(title, at: 4, in: statement)
        bind(transcriptionText, at: 5, in: statement)
        bind(postProcessedText, at: 6, in: statement)
        bind(postProcessPrompt, at: 7, in: statement)
        sqlite3_bind_int(statement, 8, postProcessRequested ? 1 : 0)

        guard sqlite3_step(statement) == SQLITE_DONE else {
            throw HistoryStoreError.step(lastError(database))
        }

        let entry = HistoryEntry(
            id: sqlite3_last_insert_rowid(database),
            fileName: fileName,
            timestamp: timestampSeconds,
            saved: false,
            title: title,
            transcriptionText: transcriptionText,
            postProcessedText: postProcessedText,
            postProcessPrompt: postProcessPrompt,
            postProcessRequested: postProcessRequested
        )

        return entry
    }

    func entries(cursor: Int64? = nil, limit: Int? = nil) throws -> PaginatedHistory {
        let database = try openDatabase()
        defer { sqlite3_close(database) }

        let boundedLimit = limit.map { max(0, min(100, $0)) }
        let fetchLimit = boundedLimit.map { $0 + 1 }
        let sql: String
        if cursor != nil, fetchLimit != nil {
            sql = """
            SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
            FROM transcription_history
            WHERE id < ?1
            ORDER BY id DESC
            LIMIT ?2
            """
        } else if fetchLimit != nil {
            sql = """
            SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
            FROM transcription_history
            ORDER BY id DESC
            LIMIT ?1
            """
        } else {
            sql = """
            SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
            FROM transcription_history
            ORDER BY id DESC
            """
        }

        let statement = try prepare(sql, in: database)
        defer { sqlite3_finalize(statement) }

        if let cursor, let fetchLimit {
            sqlite3_bind_int64(statement, 1, cursor)
            sqlite3_bind_int64(statement, 2, Int64(fetchLimit))
        } else if let fetchLimit {
            sqlite3_bind_int64(statement, 1, Int64(fetchLimit))
        }

        var rows: [HistoryEntry] = []
        while sqlite3_step(statement) == SQLITE_ROW {
            rows.append(mapEntry(statement))
        }

        let hasMore = boundedLimit.map { rows.count > $0 } ?? false
        if hasMore {
            rows.removeLast()
        }

        return PaginatedHistory(entries: rows, hasMore: hasMore)
    }

    func latestCompletedEntry() throws -> HistoryEntry? {
        let database = try openDatabase()
        defer { sqlite3_close(database) }

        let statement = try prepare(
            """
            SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
            FROM transcription_history
            WHERE transcription_text != ''
            ORDER BY timestamp DESC
            LIMIT 1
            """,
            in: database
        )
        defer { sqlite3_finalize(statement) }

        guard sqlite3_step(statement) == SQLITE_ROW else {
            return nil
        }

        return mapEntry(statement)
    }

    func entry(id: Int64) throws -> HistoryEntry? {
        let database = try openDatabase()
        defer { sqlite3_close(database) }

        return try entry(id: id, in: database)
    }

    func updateTranscription(
        id: Int64,
        transcriptionText: String,
        postProcessedText: String? = nil,
        postProcessPrompt: String? = nil
    ) throws -> HistoryEntry {
        let database = try openDatabase()
        defer { sqlite3_close(database) }

        let statement = try prepare(
            """
            UPDATE transcription_history
            SET transcription_text = ?1,
                post_processed_text = ?2,
                post_process_prompt = ?3
            WHERE id = ?4
            """,
            in: database
        )
        defer { sqlite3_finalize(statement) }

        bind(transcriptionText, at: 1, in: statement)
        bind(postProcessedText, at: 2, in: statement)
        bind(postProcessPrompt, at: 3, in: statement)
        sqlite3_bind_int64(statement, 4, id)

        guard sqlite3_step(statement) == SQLITE_DONE else {
            throw HistoryStoreError.step(lastError(database))
        }

        guard sqlite3_changes(database) > 0,
              let updated = try entry(id: id, in: database)
        else {
            throw HistoryStoreError.missingEntry(id)
        }

        return updated
    }

    func toggleSavedStatus(id: Int64) throws -> HistoryEntry {
        let database = try openDatabase()
        defer { sqlite3_close(database) }

        guard let current = try entry(id: id, in: database) else {
            throw HistoryStoreError.missingEntry(id)
        }

        let statement = try prepare(
            "UPDATE transcription_history SET saved = ?1 WHERE id = ?2",
            in: database
        )
        defer { sqlite3_finalize(statement) }

        sqlite3_bind_int(statement, 1, current.saved ? 0 : 1)
        sqlite3_bind_int64(statement, 2, id)

        guard sqlite3_step(statement) == SQLITE_DONE else {
            throw HistoryStoreError.step(lastError(database))
        }

        guard let updated = try entry(id: id, in: database) else {
            throw HistoryStoreError.missingEntry(id)
        }
        return updated
    }

    func deleteEntry(id: Int64) throws {
        let database = try openDatabase()
        defer { sqlite3_close(database) }

        let entry = try entry(id: id, in: database)
        let statement = try prepare(
            "DELETE FROM transcription_history WHERE id = ?1",
            in: database
        )
        defer { sqlite3_finalize(statement) }

        sqlite3_bind_int64(statement, 1, id)
        guard sqlite3_step(statement) == SQLITE_DONE else {
            throw HistoryStoreError.step(lastError(database))
        }

        if let entry {
            let audioURL = audioFileURL(fileName: entry.fileName)
            if FileManager.default.fileExists(atPath: audioURL.path) {
                try? FileManager.default.removeItem(at: audioURL)
            }
        }
    }

    @discardableResult
    func deleteEntryIfPending(id: Int64) throws -> Bool {
        let database = try openDatabase()
        defer { sqlite3_close(database) }

        guard let entry = try entry(id: id, in: database),
              entry.saved == false,
              entry.hasTranscription == false,
              Self.isBlank(entry.postProcessedText)
        else {
            return false
        }

        try deleteRows([(id: entry.id, fileName: entry.fileName)], in: database)
        return true
    }

    func cleanup(
        retentionPeriod: RecordingRetentionPeriod,
        historyLimit: Int,
        excludingID: Int64? = nil,
        now: Date = Date()
    ) throws {
        switch retentionPeriod {
        case .never:
            return
        case .preserveLimit:
            try cleanupByCount(limit: historyLimit, excludingID: excludingID)
        case .days3, .weeks2, .months3:
            guard let interval = retentionPeriod.retentionInterval else {
                return
            }
            let cutoffTimestamp = Int64(now.timeIntervalSince1970 - interval)
            try cleanupOlderThan(timestamp: cutoffTimestamp)
        }
    }

    func cleanupByCount(limit: Int, excludingID: Int64? = nil) throws {
        let database = try openDatabase()
        defer { sqlite3_close(database) }

        let statement = try prepare(
            "SELECT id, file_name FROM transcription_history WHERE saved = 0 ORDER BY timestamp DESC, id DESC",
            in: database
        )
        defer { sqlite3_finalize(statement) }

        var rows: [(id: Int64, fileName: String)] = []
        while sqlite3_step(statement) == SQLITE_ROW {
            let id = sqlite3_column_int64(statement, 0)
            if let excludingID, id == excludingID {
                continue
            }
            rows.append((id, textColumn(statement, 1) ?? ""))
        }

        let boundedLimit = max(0, limit)
        guard rows.count > boundedLimit else {
            return
        }

        try deleteRows(Array(rows.dropFirst(boundedLimit)), in: database)
    }

    func cleanupOlderThan(timestamp cutoffTimestamp: Int64) throws {
        let database = try openDatabase()
        defer { sqlite3_close(database) }

        let statement = try prepare(
            "SELECT id, file_name FROM transcription_history WHERE saved = 0 AND timestamp < ?1",
            in: database
        )
        defer { sqlite3_finalize(statement) }

        sqlite3_bind_int64(statement, 1, cutoffTimestamp)

        var rows: [(id: Int64, fileName: String)] = []
        while sqlite3_step(statement) == SQLITE_ROW {
            rows.append((sqlite3_column_int64(statement, 0), textColumn(statement, 1) ?? ""))
        }

        try deleteRows(rows, in: database)
    }

    func audioFileURL(fileName: String) -> URL {
        recordingsDirectory.appendingPathComponent(fileName)
    }

    private func deleteRows(_ rows: [(id: Int64, fileName: String)], in database: OpaquePointer) throws {
        for row in rows {
            let deleteStatement = try prepare(
                "DELETE FROM transcription_history WHERE id = ?1",
                in: database
            )

            sqlite3_bind_int64(deleteStatement, 1, row.id)
            let result = sqlite3_step(deleteStatement)
            sqlite3_finalize(deleteStatement)

            guard result == SQLITE_DONE else {
                throw HistoryStoreError.step(lastError(database))
            }

            let audioURL = audioFileURL(fileName: row.fileName)
            if FileManager.default.fileExists(atPath: audioURL.path) {
                try? FileManager.default.removeItem(at: audioURL)
            }
        }
    }

    private func initializeDatabase() throws {
        let database = try openDatabase()
        defer { sqlite3_close(database) }

        try execute(
            """
            CREATE TABLE IF NOT EXISTS transcription_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_name TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                saved BOOLEAN NOT NULL DEFAULT 0,
                title TEXT NOT NULL,
                transcription_text TEXT NOT NULL
            );
            """,
            in: database
        )

        try addColumnIfMissing("post_processed_text", definition: "post_processed_text TEXT", in: database)
        try addColumnIfMissing("post_process_prompt", definition: "post_process_prompt TEXT", in: database)
        try addColumnIfMissing(
            "post_process_requested",
            definition: "post_process_requested BOOLEAN NOT NULL DEFAULT 0",
            in: database
        )
        try execute("PRAGMA user_version = 4", in: database)
    }

    private func addColumnIfMissing(_ column: String, definition: String, in database: OpaquePointer) throws {
        guard try tableColumns(in: database).contains(column) == false else {
            return
        }

        try execute("ALTER TABLE transcription_history ADD COLUMN \(definition)", in: database)
    }

    private func tableColumns(in database: OpaquePointer) throws -> Set<String> {
        let statement = try prepare("PRAGMA table_info(transcription_history)", in: database)
        defer { sqlite3_finalize(statement) }

        var columns = Set<String>()
        while sqlite3_step(statement) == SQLITE_ROW {
            if let column = textColumn(statement, 1) {
                columns.insert(column)
            }
        }
        return columns
    }

    private func entry(id: Int64, in database: OpaquePointer) throws -> HistoryEntry? {
        let statement = try prepare(
            """
            SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
            FROM transcription_history
            WHERE id = ?1
            """,
            in: database
        )
        defer { sqlite3_finalize(statement) }

        sqlite3_bind_int64(statement, 1, id)
        guard sqlite3_step(statement) == SQLITE_ROW else {
            return nil
        }

        return mapEntry(statement)
    }

    private func openDatabase() throws -> OpaquePointer {
        var database: OpaquePointer?
        guard sqlite3_open(databaseURL.path, &database) == SQLITE_OK, let database else {
            let message = database.map(lastError) ?? "unknown error"
            if let database {
                sqlite3_close(database)
            }
            throw HistoryStoreError.openDatabase(message)
        }
        return database
    }

    private func execute(_ sql: String, in database: OpaquePointer) throws {
        guard sqlite3_exec(database, sql, nil, nil, nil) == SQLITE_OK else {
            throw HistoryStoreError.step(lastError(database))
        }
    }

    private func prepare(_ sql: String, in database: OpaquePointer) throws -> OpaquePointer {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK, let statement else {
            throw HistoryStoreError.prepare(lastError(database))
        }
        return statement
    }

    private func bind(_ value: String?, at index: Int32, in statement: OpaquePointer) {
        if let value {
            sqlite3_bind_text(statement, index, value, -1, Self.transient)
        } else {
            sqlite3_bind_null(statement, index)
        }
    }

    private func mapEntry(_ statement: OpaquePointer) -> HistoryEntry {
        HistoryEntry(
            id: sqlite3_column_int64(statement, 0),
            fileName: textColumn(statement, 1) ?? "",
            timestamp: sqlite3_column_int64(statement, 2),
            saved: sqlite3_column_int(statement, 3) != 0,
            title: textColumn(statement, 4) ?? "",
            transcriptionText: textColumn(statement, 5) ?? "",
            postProcessedText: textColumn(statement, 6),
            postProcessPrompt: textColumn(statement, 7),
            postProcessRequested: sqlite3_column_int(statement, 8) != 0
        )
    }

    private func textColumn(_ statement: OpaquePointer, _ index: Int32) -> String? {
        guard sqlite3_column_type(statement, index) != SQLITE_NULL,
              let text = sqlite3_column_text(statement, index)
        else {
            return nil
        }
        return String(cString: text)
    }

    private func lastError(_ database: OpaquePointer) -> String {
        if let message = sqlite3_errmsg(database) {
            return String(cString: message)
        }
        return "unknown SQLite error"
    }

    private static func isBlank(_ value: String?) -> Bool {
        value?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true
    }

    private static func formatTitle(timestamp: Date) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "MMMM d, yyyy - h:mma"
        return formatter.string(from: timestamp)
    }
}
