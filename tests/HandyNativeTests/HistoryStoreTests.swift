import Foundation
@testable import HandyNative
import XCTest

final class HistoryStoreTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("HandyNativeTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(
            at: temporaryDirectory,
            withIntermediateDirectories: true
        )
    }

    override func tearDownWithError() throws {
        if let temporaryDirectory {
            try? FileManager.default.removeItem(at: temporaryDirectory)
        }
    }

    func testEntriesAreReturnedNewestFirstWithCursorPagination() throws {
        let store = try makeStore()
        let first = try store.saveEntry(
            fileName: "first.wav",
            transcriptionText: "first",
            timestamp: Date(timeIntervalSince1970: 100)
        )
        let second = try store.saveEntry(
            fileName: "second.wav",
            transcriptionText: "second",
            timestamp: Date(timeIntervalSince1970: 200)
        )

        let firstPage = try store.entries(limit: 1)
        XCTAssertTrue(firstPage.hasMore)
        XCTAssertEqual(firstPage.entries.map(\.id), [second.id])

        let secondPage = try store.entries(cursor: second.id, limit: 1)
        XCTAssertFalse(secondPage.hasMore)
        XCTAssertEqual(secondPage.entries.map(\.id), [first.id])
    }

    func testLatestCompletedEntrySkipsEmptyTranscriptions() throws {
        let store = try makeStore()
        _ = try store.saveEntry(
            fileName: "completed.wav",
            transcriptionText: "completed",
            timestamp: Date(timeIntervalSince1970: 100)
        )
        _ = try store.saveEntry(
            fileName: "empty.wav",
            transcriptionText: "",
            timestamp: Date(timeIntervalSince1970: 200)
        )

        let latest = try store.latestCompletedEntry()
        XCTAssertEqual(latest?.transcriptionText, "completed")
    }

    func testUpdateTranscriptionWritesRecognizedText() throws {
        let store = try makeStore()
        let entry = try store.saveEntry(
            fileName: "pending.wav",
            transcriptionText: ""
        )

        let updated = try store.updateTranscription(
            id: entry.id,
            transcriptionText: "recognized text"
        )

        XCTAssertEqual(updated.transcriptionText, "recognized text")
        XCTAssertEqual(try store.latestCompletedEntry()?.id, entry.id)
    }

    func testEntryLookupAndRetryUpdateReplaceExistingTranscriptionFields() throws {
        let store = try makeStore()
        let entry = try store.saveEntry(
            fileName: "retry.wav",
            transcriptionText: "old",
            postProcessRequested: true,
            postProcessedText: "old processed",
            postProcessPrompt: "old prompt"
        )

        XCTAssertEqual(try store.entry(id: entry.id)?.fileName, "retry.wav")

        let updated = try store.updateTranscription(
            id: entry.id,
            transcriptionText: "new",
            postProcessedText: "new processed",
            postProcessPrompt: "new prompt"
        )

        XCTAssertEqual(updated.id, entry.id)
        XCTAssertEqual(updated.transcriptionText, "new")
        XCTAssertEqual(updated.postProcessedText, "new processed")
        XCTAssertEqual(updated.postProcessPrompt, "new prompt")
        XCTAssertTrue(updated.postProcessRequested)
        XCTAssertNil(try store.entry(id: 99_999))
    }

    func testToggleSavedStatusAndDeleteEntry() throws {
        let store = try makeStore()
        let entry = try store.saveEntry(
            fileName: "sample.wav",
            transcriptionText: "hello"
        )
        FileManager.default.createFile(
            atPath: store.audioFileURL(fileName: entry.fileName).path,
            contents: Data([0, 1, 2])
        )

        let saved = try store.toggleSavedStatus(id: entry.id)
        XCTAssertTrue(saved.saved)

        try store.deleteEntry(id: entry.id)

        XCTAssertTrue(try store.entries().entries.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: entry.fileName).path))
    }

    func testDeleteEntryIfPendingRemovesEmptyUnsavedEntryAndAudioFile() throws {
        let store = try makeStore()
        let entry = try store.saveEntry(
            fileName: "pending.wav",
            transcriptionText: ""
        )
        FileManager.default.createFile(
            atPath: store.audioFileURL(fileName: entry.fileName).path,
            contents: Data([0, 1, 2])
        )

        XCTAssertTrue(try store.deleteEntryIfPending(id: entry.id))

        XCTAssertTrue(try store.entries().entries.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: entry.fileName).path))
    }

    func testDeleteEntryIfPendingPreservesCompletedSavedAndProcessedEntries() throws {
        let store = try makeStore()
        let completed = try saveEntryWithAudio(
            store: store,
            fileName: "completed.wav",
            timestamp: Date(timeIntervalSince1970: 100)
        )
        let savedPending = try saveEntryWithAudio(
            store: store,
            fileName: "saved-pending.wav",
            transcriptionText: "",
            timestamp: Date(timeIntervalSince1970: 200)
        )
        _ = try store.toggleSavedStatus(id: savedPending.id)
        let processedPending = try store.saveEntry(
            fileName: "processed-pending.wav",
            transcriptionText: "",
            postProcessedText: "processed",
            timestamp: Date(timeIntervalSince1970: 300)
        )
        FileManager.default.createFile(
            atPath: store.audioFileURL(fileName: processedPending.fileName).path,
            contents: Data([0, 1, 2])
        )

        XCTAssertFalse(try store.deleteEntryIfPending(id: completed.id))
        XCTAssertFalse(try store.deleteEntryIfPending(id: savedPending.id))
        XCTAssertFalse(try store.deleteEntryIfPending(id: processedPending.id))
        XCTAssertFalse(try store.deleteEntryIfPending(id: 99_999))

        let entries = try store.entries().entries
        XCTAssertEqual(Set(entries.map(\.id)), Set([completed.id, savedPending.id, processedPending.id]))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: completed.fileName).path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: savedPending.fileName).path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: processedPending.fileName).path))
    }

    func testCleanupNeverKeepsUnsavedEntriesAndFiles() throws {
        let store = try makeStore()
        let entry = try saveEntryWithAudio(
            store: store,
            fileName: "keep.wav",
            timestamp: Date(timeIntervalSince1970: 100)
        )

        try store.cleanup(retentionPeriod: .never, historyLimit: 0)

        XCTAssertEqual(try store.entries().entries.map(\.id), [entry.id])
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: entry.fileName).path))
    }

    func testCleanupPreserveLimitDeletesOldestUnsavedEntriesOnly() throws {
        let store = try makeStore()
        let savedOld = try saveEntryWithAudio(
            store: store,
            fileName: "saved-old.wav",
            timestamp: Date(timeIntervalSince1970: 100)
        )
        _ = try store.toggleSavedStatus(id: savedOld.id)
        let oldUnsaved = try saveEntryWithAudio(
            store: store,
            fileName: "old-unsaved.wav",
            timestamp: Date(timeIntervalSince1970: 200)
        )
        let newestUnsaved = try saveEntryWithAudio(
            store: store,
            fileName: "newest-unsaved.wav",
            timestamp: Date(timeIntervalSince1970: 300)
        )

        try store.cleanup(retentionPeriod: .preserveLimit, historyLimit: 1)

        let entries = try store.entries().entries
        XCTAssertEqual(Set(entries.map(\.id)), Set([savedOld.id, newestUnsaved.id]))
        XCTAssertNil(try store.entry(id: oldUnsaved.id))
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: oldUnsaved.fileName).path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: savedOld.fileName).path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: newestUnsaved.fileName).path))
    }

    func testCleanupByRetentionPeriodDeletesOnlyOldUnsavedEntries() throws {
        let store = try makeStore()
        let now = Date(timeIntervalSince1970: 1_000_000)
        let oldTimestamp = now.addingTimeInterval(-(3 * 24 * 60 * 60) - 1)
        let recentTimestamp = now.addingTimeInterval(-(2 * 24 * 60 * 60))
        let oldUnsaved = try saveEntryWithAudio(
            store: store,
            fileName: "old-unsaved.wav",
            timestamp: oldTimestamp
        )
        let recentUnsaved = try saveEntryWithAudio(
            store: store,
            fileName: "recent-unsaved.wav",
            timestamp: recentTimestamp
        )
        let savedOld = try saveEntryWithAudio(
            store: store,
            fileName: "saved-old.wav",
            timestamp: oldTimestamp
        )
        _ = try store.toggleSavedStatus(id: savedOld.id)

        try store.cleanup(retentionPeriod: .days3, historyLimit: 0, now: now)

        let entries = try store.entries().entries
        XCTAssertEqual(Set(entries.map(\.id)), Set([recentUnsaved.id, savedOld.id]))
        XCTAssertNil(try store.entry(id: oldUnsaved.id))
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: oldUnsaved.fileName).path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: recentUnsaved.fileName).path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: savedOld.fileName).path))
    }

    func testOutputTextPrefersPostProcessedText() {
        let entry = HistoryEntry(
            id: 1,
            fileName: "sample.wav",
            timestamp: 0,
            saved: false,
            title: "Recording",
            transcriptionText: "raw",
            postProcessedText: "processed",
            postProcessPrompt: nil,
            postProcessRequested: true
        )

        XCTAssertEqual(entry.outputText, "processed")
    }

    private func makeStore() throws -> HistoryStore {
        try HistoryStore(
            databaseURL: temporaryDirectory.appendingPathComponent("history.db"),
            recordingsDirectory: temporaryDirectory.appendingPathComponent("recordings", isDirectory: true)
        )
    }

    @discardableResult
    private func saveEntryWithAudio(
        store: HistoryStore,
        fileName: String,
        transcriptionText: String? = nil,
        timestamp: Date
    ) throws -> HistoryEntry {
        let entry = try store.saveEntry(
            fileName: fileName,
            transcriptionText: transcriptionText ?? fileName,
            timestamp: timestamp
        )
        FileManager.default.createFile(
            atPath: store.audioFileURL(fileName: fileName).path,
            contents: Data([0, 1, 2])
        )
        return entry
    }
}
