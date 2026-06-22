import Foundation
@testable import HandyNative
import XCTest

final class NativeLogStoreTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("HandyNativeLogs-\(UUID().uuidString)", isDirectory: true)
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

    func testLogLevelFilteringUsesMinimumSeverity() {
        XCTAssertTrue(NativeLogLevel.info.allows(.error))
        XCTAssertTrue(NativeLogLevel.info.allows(.warn))
        XCTAssertTrue(NativeLogLevel.info.allows(.info))
        XCTAssertFalse(NativeLogLevel.info.allows(.debug))
        XCTAssertFalse(NativeLogLevel.info.allows(.trace))
    }

    func testWriteCreatesLogFileAndSanitizesMultilineMessages() throws {
        let store = NativeLogStore(
            logsDirectory: temporaryDirectory,
            now: { Date(timeIntervalSince1970: 1_781_987_472) }
        )

        let wrote = try store.write(
            .info,
            "Recording started.\nSecond line",
            minimumLevel: .debug
        )

        XCTAssertTrue(wrote)
        let contents = try String(contentsOf: store.logURL, encoding: .utf8)
        XCTAssertTrue(contents.contains("[INFO] Recording started. Second line"))
        XCTAssertEqual(contents.split(separator: "\n").count, 1)
    }

    func testWriteFiltersEntriesBelowConfiguredLevel() throws {
        let store = NativeLogStore(logsDirectory: temporaryDirectory)

        let wrote = try store.write(
            .debug,
            "Filtered",
            minimumLevel: .warn
        )

        XCTAssertFalse(wrote)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.logURL.path))
    }

    func testWriteAppendsEntries() throws {
        let store = NativeLogStore(logsDirectory: temporaryDirectory)

        try store.write(.warn, "First", minimumLevel: .debug)
        try store.write(.error, "Second", minimumLevel: .debug)

        let contents = try String(contentsOf: store.logURL, encoding: .utf8)
        XCTAssertTrue(contents.contains("[WARN] First"))
        XCTAssertTrue(contents.contains("[ERROR] Second"))
        XCTAssertEqual(contents.split(separator: "\n").count, 2)
    }
}
