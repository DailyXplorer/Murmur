import Foundation
@testable import MurmurNative
import XCTest

final class NativeRemoteControlServiceTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("MurmurNativeRemoteControl-\(UUID().uuidString)", isDirectory: true)
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

    func testAuthorizedCommandReturnsCommandForValidToken() {
        let command = NativeRemoteControlService.authorizedCommand(
            from: ["command": "toggle-transcription", "token": "expected-test-token"],
            expectedToken: "expected-test-token"
        )

        XCTAssertEqual(command, .toggleTranscription)
    }

    func testAuthorizedCommandReturnsNilWhenTokenIsMissing() {
        let command = NativeRemoteControlService.authorizedCommand(
            from: ["command": "toggle-transcription"],
            expectedToken: "expected-test-token"
        )

        XCTAssertNil(command)
    }

    func testAuthorizedCommandReturnsNilWhenUserInfoIsNil() {
        let command = NativeRemoteControlService.authorizedCommand(
            from: nil,
            expectedToken: "expected-test-token"
        )

        XCTAssertNil(command)
    }

    func testAuthorizedCommandReturnsNilWhenTokenIsWrong() {
        let command = NativeRemoteControlService.authorizedCommand(
            from: ["command": "cancel", "token": "wrong-test-token"],
            expectedToken: "expected-test-token"
        )

        XCTAssertNil(command)
    }

    func testAuthorizedCommandReturnsNilForUnknownCommand() {
        let command = NativeRemoteControlService.authorizedCommand(
            from: ["command": "not-a-real-command", "token": "expected-test-token"],
            expectedToken: "expected-test-token"
        )

        XCTAssertNil(command)
    }

    func testTokenFileIsCreatedWith0600() throws {
        let token = NativeRemoteControlService.localAuthorizationToken(
            appDataDirectory: temporaryDirectory
        )

        XCTAssertFalse(token.isEmpty)
        let tokenURL = temporaryDirectory.appendingPathComponent(".remote-control-token")
        XCTAssertTrue(FileManager.default.fileExists(atPath: tokenURL.path))

        let attributes = try FileManager.default.attributesOfItem(atPath: tokenURL.path)
        let permissions = (attributes[.posixPermissions] as? NSNumber)?.intValue ?? 0
        XCTAssertEqual(permissions & 0o777, 0o600)

        let secondToken = NativeRemoteControlService.localAuthorizationToken(
            appDataDirectory: temporaryDirectory
        )
        XCTAssertEqual(secondToken, token)
    }
}
