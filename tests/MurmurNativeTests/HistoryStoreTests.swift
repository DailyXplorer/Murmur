import Foundation
@testable import MurmurNative
import XCTest

final class HistoryStoreTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("MurmurNativeTests-\(UUID().uuidString)", isDirectory: true)
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

    func testCleanupByCountKeepsNewestInsertOnTimestampTie() throws {
        let store = try makeStore()
        let sharedTimestamp = Date(timeIntervalSince1970: 100)
        let first = try saveEntryWithAudio(
            store: store,
            fileName: "tie-first.wav",
            timestamp: sharedTimestamp
        )
        let second = try saveEntryWithAudio(
            store: store,
            fileName: "tie-second.wav",
            timestamp: sharedTimestamp
        )
        let third = try saveEntryWithAudio(
            store: store,
            fileName: "tie-third.wav",
            timestamp: sharedTimestamp
        )
        XCTAssertLessThan(first.id, second.id)
        XCTAssertLessThan(second.id, third.id)

        try store.cleanupByCount(limit: 2)

        let survivors = try store.entries().entries
        XCTAssertEqual(Set(survivors.map(\.id)), Set([second.id, third.id]))
        XCTAssertNil(try store.entry(id: first.id))
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: first.fileName).path))
    }

    func testCleanupExcludesActiveEntry() throws {
        let store = try makeStore()
        let oldest = try saveEntryWithAudio(
            store: store,
            fileName: "active-oldest.wav",
            timestamp: Date(timeIntervalSince1970: 100)
        )
        let middle = try saveEntryWithAudio(
            store: store,
            fileName: "middle.wav",
            timestamp: Date(timeIntervalSince1970: 200)
        )
        let newest = try saveEntryWithAudio(
            store: store,
            fileName: "newest.wav",
            timestamp: Date(timeIntervalSince1970: 300)
        )

        try store.cleanup(retentionPeriod: .preserveLimit, historyLimit: 1, excludingID: oldest.id)

        let survivors = try store.entries().entries
        XCTAssertEqual(Set(survivors.map(\.id)), Set([oldest.id, newest.id]))
        XCTAssertNil(try store.entry(id: middle.id))
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: middle.fileName).path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: oldest.fileName).path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: newest.fileName).path))
    }

    func testCleanupLimitZeroWithExclusionKeepsExcluded() throws {
        let store = try makeStore()
        let excluded = try saveEntryWithAudio(
            store: store,
            fileName: "excluded.wav",
            timestamp: Date(timeIntervalSince1970: 100)
        )
        let other = try saveEntryWithAudio(
            store: store,
            fileName: "other.wav",
            timestamp: Date(timeIntervalSince1970: 200)
        )

        try store.cleanupByCount(limit: 0, excludingID: excluded.id)

        let survivors = try store.entries().entries
        XCTAssertEqual(survivors.map(\.id), [excluded.id])
        XCTAssertNil(try store.entry(id: other.id))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: excluded.fileName).path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.audioFileURL(fileName: other.fileName).path))
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
