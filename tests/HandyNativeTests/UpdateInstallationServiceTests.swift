import Foundation
@testable import HandyNative
import XCTest

final class UpdateInstallationServiceTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("HandyNativeUpdate-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: temporaryDirectory, withIntermediateDirectories: true)
        MockUpdateDownloadURLProtocol.reset()
    }

    override func tearDownWithError() throws {
        MockUpdateDownloadURLProtocol.reset()
        if let temporaryDirectory {
            try? FileManager.default.removeItem(at: temporaryDirectory)
        }
    }

    func testDownloadStoresArtifactAndReportsProgress() async throws {
        let artifactData = Data("native update artifact".utf8)
        MockUpdateDownloadURLProtocol.statusCode = 200
        MockUpdateDownloadURLProtocol.data = artifactData
        let session = makeMockSession()
        let update = UpdateInfo(
            version: "0.9.0",
            notes: nil,
            releaseURL: URL(string: "https://updates.example.test/Handy.dmg")!
        )
        let progressRecorder = UpdateProgressRecorder()

        let artifactURL = try await UpdateInstallationService.download(
            update: update,
            destinationDirectory: temporaryDirectory,
            urlSession: session,
            progress: { await progressRecorder.append($0) }
        )
        let progressEvents = await progressRecorder.events

        XCTAssertEqual(artifactURL.lastPathComponent, "Handy.dmg")
        XCTAssertEqual(try Data(contentsOf: artifactURL), artifactData)
        XCTAssertEqual(progressEvents.first?.downloadedBytes, 0)
        XCTAssertEqual(progressEvents.last?.downloadedBytes, Int64(artifactData.count))
        XCTAssertEqual(progressEvents.last?.percent, 100)
    }

    func testDownloadRejectsHTTPFailure() async throws {
        MockUpdateDownloadURLProtocol.statusCode = 503
        MockUpdateDownloadURLProtocol.data = Data("nope".utf8)
        let session = makeMockSession()
        let update = UpdateInfo(
            version: "0.9.0",
            notes: nil,
            releaseURL: URL(string: "https://updates.example.test/Handy.dmg")!
        )

        do {
            _ = try await UpdateInstallationService.download(
                update: update,
                destinationDirectory: temporaryDirectory,
                urlSession: session
            )
            XCTFail("Expected HTTP failure.")
        } catch let error as UpdateInstallationServiceError {
            XCTAssertEqual(error, .httpStatus(503))
        }
    }

    func testDownloadRejectsEmptyArtifact() async throws {
        MockUpdateDownloadURLProtocol.statusCode = 200
        MockUpdateDownloadURLProtocol.data = Data()
        let session = makeMockSession()
        let update = UpdateInfo(
            version: "0.9.0",
            notes: nil,
            releaseURL: URL(string: "https://updates.example.test/Handy.dmg")!
        )

        do {
            _ = try await UpdateInstallationService.download(
                update: update,
                destinationDirectory: temporaryDirectory,
                urlSession: session
            )
            XCTFail("Expected empty artifact failure.")
        } catch let error as UpdateInstallationServiceError {
            XCTAssertEqual(error, .emptyArtifact)
        }
    }

    func testArtifactFileNameFallsBackWhenURLHasNoFileName() {
        let update = UpdateInfo(
            version: "0.9.0",
            notes: nil,
            releaseURL: URL(string: "https://updates.example.test/download/")!
        )

        XCTAssertEqual(UpdateInstallationService.artifactFileName(for: update), "Handy-0.9.0.dmg")
    }

    func testPrepareZipArtifactCreatesInstallScriptForAppBundle() async throws {
        let sourceDirectory = temporaryDirectory.appendingPathComponent("zip-source", isDirectory: true)
        let sourceApp = sourceDirectory.appendingPathComponent("Handy.app", isDirectory: true)
        try createFakeAppBundle(at: sourceApp)

        let zipURL = temporaryDirectory.appendingPathComponent("Handy_0.9.0_aarch64.app.zip")
        try runProcess(
            executable: "/usr/bin/ditto",
            arguments: ["-c", "-k", "--norsrc", "--noextattr", sourceDirectory.path, zipURL.path]
        )

        let currentApp = temporaryDirectory.appendingPathComponent("Current Handy.app", isDirectory: true)
        try FileManager.default.createDirectory(at: currentApp, withIntermediateDirectories: true)
        let update = UpdateInfo(
            version: "0.9.0",
            notes: nil,
            releaseURL: URL(string: "https://updates.example.test/Handy_0.9.0_aarch64.app.zip")!
        )

        let artifact = try await UpdateInstallationService.prepareForInstallation(
            artifactURL: zipURL,
            update: update,
            workDirectory: temporaryDirectory.appendingPathComponent("updates", isDirectory: true),
            currentAppBundleURL: currentApp,
            currentProcessID: 1234
        )

        XCTAssertEqual(artifact.artifactURL, zipURL)
        XCTAssertTrue(artifact.canInstallAndRelaunch)
        XCTAssertEqual(artifact.preparedAppBundleURL?.lastPathComponent, "Handy.app")
        XCTAssertTrue(FileManager.default.fileExists(atPath: artifact.preparedAppBundleURL?.path ?? ""))
        XCTAssertTrue(FileManager.default.fileExists(atPath: artifact.installerScriptURL?.path ?? ""))

        let script = try String(contentsOf: XCTUnwrap(artifact.installerScriptURL), encoding: .utf8)
        XCTAssertTrue(script.contains("APP_PID=1234"))
        XCTAssertTrue(script.contains("/usr/bin/ditto --norsrc --noextattr --noqtn"))
        XCTAssertTrue(script.contains("/usr/bin/open -n"))
        XCTAssertTrue(script.contains("with administrator privileges"))
        XCTAssertTrue(script.contains("Current Handy.app"))
    }

    func testReplacementScriptUsesUserRelaunchAfterAdminInstallHelper() throws {
        let stagedApp = temporaryDirectory
            .appendingPathComponent("staged", isDirectory: true)
            .appendingPathComponent("Handy.app", isDirectory: true)
        let currentApp = temporaryDirectory
            .appendingPathComponent("Current Handy.app", isDirectory: true)
        let scriptDirectory = temporaryDirectory.appendingPathComponent("updates", isDirectory: true)
        try createFakeAppBundle(at: stagedApp)
        try createFakeAppBundle(at: currentApp)

        let scriptURL = try UpdateInstallationService.createReplacementScript(
            stagedAppBundleURL: stagedApp,
            currentAppBundleURL: currentApp,
            scriptDirectory: scriptDirectory,
            currentProcessID: 2468
        )
        try runProcess(executable: "/bin/sh", arguments: ["-n", scriptURL.path])

        let script = try String(contentsOf: scriptURL, encoding: .utf8)
        XCTAssertTrue(script.contains("INSTALL_HELPER="))
        XCTAssertTrue(script.contains("cat > \"$INSTALL_HELPER\" <<'HANDY_INSTALL_HELPER'"))
        XCTAssertTrue(script.contains("if [ -w \"$TARGET_PARENT\" ]; then"))
        XCTAssertTrue(script.contains("/usr/bin/osascript <<'HANDY_ADMIN_OSASCRIPT'"))
        XCTAssertTrue(script.contains("with administrator privileges"))
        XCTAssertTrue(script.contains("rm -f \"$INSTALL_HELPER\" \"$0\"\n/usr/bin/open -n \"$TARGET_APP\""))

        let helperStart = try XCTUnwrap(script.range(of: "HANDY_INSTALL_HELPER")?.upperBound)
        let helperEnd = try XCTUnwrap(
            script.range(of: "HANDY_INSTALL_HELPER", range: helperStart..<script.endIndex)?.lowerBound
        )
        let helperSection = String(script[helperStart..<helperEnd])
        XCTAssertFalse(helperSection.contains("/usr/bin/open -n"))
    }

    func testPrepareDmgArtifactCreatesInstallScriptForAppBundle() async throws {
        let sourceDirectory = temporaryDirectory.appendingPathComponent("dmg-source", isDirectory: true)
        let sourceApp = sourceDirectory.appendingPathComponent("Handy.app", isDirectory: true)
        try createFakeAppBundle(at: sourceApp)

        let dmgURL = temporaryDirectory.appendingPathComponent("Handy_0.9.0_aarch64.dmg")
        try runProcess(
            executable: "/usr/bin/hdiutil",
            arguments: [
                "create",
                "-quiet",
                "-format",
                "UDZO",
                "-fs",
                "HFS+",
                "-srcfolder",
                sourceDirectory.path,
                dmgURL.path
            ]
        )

        let currentApp = temporaryDirectory.appendingPathComponent("Current Handy.app", isDirectory: true)
        try FileManager.default.createDirectory(at: currentApp, withIntermediateDirectories: true)
        let update = UpdateInfo(
            version: "0.9.0",
            notes: nil,
            releaseURL: URL(string: "https://updates.example.test/Handy.dmg")!
        )

        let artifact = try await UpdateInstallationService.prepareForInstallation(
            artifactURL: dmgURL,
            update: update,
            workDirectory: temporaryDirectory.appendingPathComponent("updates", isDirectory: true),
            currentAppBundleURL: currentApp,
            currentProcessID: 5678
        )

        XCTAssertEqual(artifact.artifactURL, dmgURL)
        XCTAssertTrue(artifact.canInstallAndRelaunch)
        XCTAssertEqual(artifact.preparedAppBundleURL?.lastPathComponent, "Handy.app")
        XCTAssertTrue(FileManager.default.fileExists(atPath: artifact.preparedAppBundleURL?.path ?? ""))
        XCTAssertTrue(FileManager.default.fileExists(atPath: artifact.installerScriptURL?.path ?? ""))

        let script = try String(contentsOf: XCTUnwrap(artifact.installerScriptURL), encoding: .utf8)
        XCTAssertTrue(script.contains("APP_PID=5678"))
        XCTAssertTrue(script.contains("/usr/bin/ditto --norsrc --noextattr --noqtn"))
        XCTAssertTrue(script.contains("Current Handy.app"))
        try assertNoMountedDiskImageReferencesTemporaryDirectory()
    }

    func testPrepareUnsupportedArtifactKeepsManualHandoff() async throws {
        let pkgURL = temporaryDirectory.appendingPathComponent("Handy.pkg")
        try Data("manual installer artifact".utf8).write(to: pkgURL)
        let update = UpdateInfo(
            version: "0.9.0",
            notes: nil,
            releaseURL: URL(string: "https://updates.example.test/Handy.pkg")!
        )

        let artifact = try await UpdateInstallationService.prepareForInstallation(
            artifactURL: pkgURL,
            update: update,
            workDirectory: temporaryDirectory.appendingPathComponent("updates", isDirectory: true)
        )

        XCTAssertEqual(artifact.artifactURL, pkgURL)
        XCTAssertFalse(artifact.canInstallAndRelaunch)
        XCTAssertNil(artifact.preparedAppBundleURL)
        XCTAssertNil(artifact.installerScriptURL)
    }

    private func makeMockSession() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockUpdateDownloadURLProtocol.self]
        return URLSession(configuration: configuration)
    }

    private func createFakeAppBundle(at appURL: URL) throws {
        let sourceExecutable = appURL
            .appendingPathComponent("Contents", isDirectory: true)
            .appendingPathComponent("MacOS", isDirectory: true)
            .appendingPathComponent("Handy", isDirectory: false)
        try FileManager.default.createDirectory(
            at: sourceExecutable.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try Data("binary".utf8).write(to: sourceExecutable)
    }

    private func runProcess(executable: String, arguments: [String]) throws {
        _ = try runProcessOutput(executable: executable, arguments: arguments)
    }

    private func assertNoMountedDiskImageReferencesTemporaryDirectory() throws {
        let diskImageInfo = try runProcessOutput(executable: "/usr/bin/hdiutil", arguments: ["info"])
        let privateTemporaryPath = temporaryDirectory.path.replacingOccurrences(
            of: "/var/folders/",
            with: "/private/var/folders/"
        )
        XCTAssertFalse(diskImageInfo.contains(temporaryDirectory.path))
        XCTAssertFalse(diskImageInfo.contains(privateTemporaryPath))
    }

    private func runProcessOutput(executable: String, arguments: [String]) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        let outputPipe = Pipe()
        let errorPipe = Pipe()
        process.standardOutput = outputPipe
        process.standardError = errorPipe
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            let output = String(data: outputPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
            let error = String(data: errorPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
            let message = [output, error]
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { $0.isEmpty == false }
                .joined(separator: "\n")
            XCTFail("\(executable) exited with \(process.terminationStatus): \(message)")
            throw NSError(
                domain: "UpdateInstallationServiceTests",
                code: Int(process.terminationStatus),
                userInfo: [NSLocalizedDescriptionKey: message]
            )
        }

        return String(data: outputPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
    }
}

private final class MockUpdateDownloadURLProtocol: URLProtocol {
    nonisolated(unsafe) static var statusCode = 200
    nonisolated(unsafe) static var data = Data()

    static func reset() {
        statusCode = 200
        data = Data()
    }

    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "updates.example.test"
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        guard let url = request.url,
              let response = HTTPURLResponse(
                  url: url,
                  statusCode: Self.statusCode,
                  httpVersion: "HTTP/1.1",
                  headerFields: ["Content-Length": "\(Self.data.count)"]
              )
        else {
            client?.urlProtocol(self, didFailWithError: URLError(.badURL))
            return
        }

        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        if Self.data.isEmpty == false {
            client?.urlProtocol(self, didLoad: Self.data)
        }
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}

private actor UpdateProgressRecorder {
    private var storedEvents: [UpdateInstallationProgress] = []

    var events: [UpdateInstallationProgress] {
        storedEvents
    }

    func append(_ progress: UpdateInstallationProgress) {
        storedEvents.append(progress)
    }
}
