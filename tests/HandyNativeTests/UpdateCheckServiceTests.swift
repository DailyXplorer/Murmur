import Foundation
@testable import HandyNative
import XCTest

final class UpdateCheckServiceTests: XCTestCase {
    func testVersionComparisonHandlesNumericSegmentsAndPrerelease() {
        XCTAssertTrue(UpdateCheckService.isVersion("0.10.0", newerThan: "0.9.9"))
        XCTAssertTrue(UpdateCheckService.isVersion("v1.0.0", newerThan: "1.0.0-beta.1"))
        XCTAssertFalse(UpdateCheckService.isVersion("1.0.0-beta.1", newerThan: "1.0.0"))
        XCTAssertFalse(UpdateCheckService.isVersion("1.2.0", newerThan: "1.2"))
    }

    func testManifestProducesUpdateWhenLatestVersionIsNewer() throws {
        let data = """
        {
          "version": "0.8.4",
          "notes": "Native-ready release",
          "platforms": {
            "darwin-aarch64": {
              "url": "https://example.com/releases/download/v0.8.4/Handy.dmg"
            }
          }
        }
        """.data(using: .utf8)!

        let result = try UpdateCheckService.result(from: data, currentVersion: "0.8.3")

        XCTAssertTrue(result.isUpdateAvailable)
        XCTAssertEqual(result.latestVersion, "0.8.4")
        XCTAssertEqual(result.update?.version, "0.8.4")
        XCTAssertEqual(result.update?.notes, "Native-ready release")
        XCTAssertEqual(result.update?.releaseURL.absoluteString, "https://example.com/releases/download/v0.8.4/Handy.dmg")
    }

    func testManifestUsesMacOSAppArtifactWhenPlainArtifactIsMissing() throws {
        let data = """
        {
          "version": "0.8.4",
          "platforms": {
            "darwin-aarch64-app": {
              "url": "https://example.com/releases/download/v0.8.4/Handy.app.tar.gz"
            }
          }
        }
        """.data(using: .utf8)!

        let result = try UpdateCheckService.result(from: data, currentVersion: "0.8.3")

        XCTAssertEqual(result.update?.releaseURL.absoluteString, "https://example.com/releases/download/v0.8.4/Handy.app.tar.gz")
    }

    func testManifestReturnsUpToDateWhenCurrentVersionMatches() throws {
        let data = #"{"version":"0.8.3"}"#.data(using: .utf8)!

        let result = try UpdateCheckService.result(from: data, currentVersion: "0.8.3")

        XCTAssertFalse(result.isUpdateAvailable)
        XCTAssertEqual(result.latestVersion, "0.8.3")
        XCTAssertNil(result.update)
    }
}
