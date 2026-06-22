import Foundation
@testable import HandyNative
import XCTest

final class AppPathsTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("HandyNativePaths-\(UUID().uuidString)", isDirectory: true)
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

    func testStandardPathsReuseHandyAppDataIdentifier() throws {
        let paths = try AppPaths.resolve(
            applicationSupportDirectory: temporaryDirectory.appendingPathComponent("Application Support", isDirectory: true),
            logsDirectory: temporaryDirectory.appendingPathComponent("Logs", isDirectory: true),
            executableDirectory: nil
        )

        XCTAssertEqual(paths.appDataDirectory.lastPathComponent, "com.pais.handy")
        XCTAssertEqual(paths.recordingsDirectory.lastPathComponent, "recordings")
        XCTAssertEqual(paths.modelsDirectory.lastPathComponent, "models")
        XCTAssertEqual(paths.logsDirectory.lastPathComponent, "com.pais.handy")
    }

    func testPortableMarkerUsesDataDirectoryNextToExecutable() throws {
        let executableDirectory = temporaryDirectory.appendingPathComponent("Handy.app/Contents/MacOS", isDirectory: true)
        let dataDirectory = executableDirectory.appendingPathComponent("Data", isDirectory: true)
        try FileManager.default.createDirectory(at: executableDirectory, withIntermediateDirectories: true)
        try "Handy Portable Mode".write(
            to: executableDirectory.appendingPathComponent("portable"),
            atomically: true,
            encoding: .utf8
        )

        let paths = try AppPaths.resolve(
            applicationSupportDirectory: temporaryDirectory.appendingPathComponent("Application Support", isDirectory: true),
            logsDirectory: temporaryDirectory.appendingPathComponent("Logs", isDirectory: true),
            executableDirectory: executableDirectory
        )

        XCTAssertEqual(paths.appDataDirectory.standardizedFileURL, dataDirectory.standardizedFileURL)
        XCTAssertEqual(paths.logsDirectory.standardizedFileURL, dataDirectory.appendingPathComponent("logs", isDirectory: true).standardizedFileURL)
    }

    func testPortableMarkerWithExistingDataDirectoryIsRepaired() throws {
        let executableDirectory = temporaryDirectory.appendingPathComponent("Handy.app/Contents/MacOS", isDirectory: true)
        let markerURL = executableDirectory.appendingPathComponent("portable")
        let dataDirectory = executableDirectory.appendingPathComponent("Data", isDirectory: true)
        try FileManager.default.createDirectory(at: dataDirectory, withIntermediateDirectories: true)
        FileManager.default.createFile(atPath: markerURL.path, contents: Data())

        let resolved = try AppPaths.resolvePortableDataDirectory(executableDirectory: executableDirectory)

        XCTAssertEqual(resolved?.standardizedFileURL, dataDirectory.standardizedFileURL)
        XCTAssertTrue(AppPaths.isValidPortableMarker(at: markerURL))
    }
}
