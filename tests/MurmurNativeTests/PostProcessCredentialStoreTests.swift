import Foundation
import Security
@testable import MurmurNative
import XCTest

final class PostProcessCredentialStoreTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("MurmurNativeCredentialStore-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: temporaryDirectory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        if let temporaryDirectory {
            try? FileManager.default.removeItem(at: temporaryDirectory)
        }
    }

    func testFileBackedStoreWritesRestrictedCredentialsAndDeletesEmptyStore() throws {
        let paths = makePaths()
        let store = LocalPostProcessCredentialStore(paths: paths, storageMode: .file)
        let credentialsURL = paths.appDataDirectory.appendingPathComponent("api_credentials.json")

        try store.saveAPIKey(" test-key ", providerID: "mistral")

        XCTAssertEqual(try store.readAPIKey(providerID: "mistral"), "test-key")
        let attributes = try FileManager.default.attributesOfItem(atPath: credentialsURL.path)
        XCTAssertEqual(attributes[.posixPermissions] as? Int, 0o600)

        try store.deleteAPIKey(providerID: "mistral")

        XCTAssertFalse(FileManager.default.fileExists(atPath: credentialsURL.path))
    }

    func testKeychainStoreRoundTripsWithoutWritingCredentialFile() throws {
        let paths = makePaths()
        let service = "com.pais.murmur.tests.\(UUID().uuidString)"
        let store = LocalPostProcessCredentialStore(
            paths: paths,
            storageMode: .keychain,
            keychainService: service
        )
        defer { try? store.deleteAPIKey(providerID: "mistral") }

        do {
            try store.saveAPIKey("keychain-key", providerID: "mistral")
        } catch let error as PostProcessCredentialStoreError
            where error.isKeychainUnavailableInTestEnvironment {
            throw XCTSkip("Keychain is unavailable in this test environment: \(error.localizedDescription)")
        }

        XCTAssertEqual(try store.readAPIKey(providerID: "mistral"), "keychain-key")
        XCTAssertFalse(
            FileManager.default.fileExists(
                atPath: paths.appDataDirectory.appendingPathComponent("api_credentials.json").path
            )
        )
    }

    func testKeychainStoreMigratesLegacyCredentialFileAndRemovesIt() throws {
        let paths = makePaths()
        let credentialsURL = paths.appDataDirectory.appendingPathComponent("api_credentials.json")
        try #"{"mistral":"legacy-key"}"#.write(to: credentialsURL, atomically: true, encoding: .utf8)
        let service = "com.pais.murmur.tests.\(UUID().uuidString)"
        let store = LocalPostProcessCredentialStore(
            paths: paths,
            storageMode: .keychain,
            keychainService: service
        )
        defer { try? store.deleteAPIKey(providerID: "mistral") }

        let migratedKey: String?
        do {
            migratedKey = try store.readAPIKey(providerID: "mistral")
        } catch let error as PostProcessCredentialStoreError
            where error.isKeychainUnavailableInTestEnvironment {
            throw XCTSkip("Keychain is unavailable in this test environment: \(error.localizedDescription)")
        }

        XCTAssertEqual(migratedKey, "legacy-key")
        XCTAssertFalse(FileManager.default.fileExists(atPath: credentialsURL.path))
    }

    private func makePaths() -> AppPaths {
        let appDataDirectory = temporaryDirectory.appendingPathComponent("app-data", isDirectory: true)
        try? FileManager.default.createDirectory(at: appDataDirectory, withIntermediateDirectories: true)
        return AppPaths(
            appDataDirectory: appDataDirectory,
            recordingsDirectory: appDataDirectory.appendingPathComponent("recordings", isDirectory: true),
            modelsDirectory: appDataDirectory.appendingPathComponent("models", isDirectory: true),
            logsDirectory: temporaryDirectory.appendingPathComponent("logs", isDirectory: true)
        )
    }
}

private extension PostProcessCredentialStoreError {
    var isKeychainUnavailableInTestEnvironment: Bool {
        switch self {
        case let .keychainReadFailed(_, status),
             let .keychainWriteFailed(_, status),
             let .keychainDeleteFailed(_, status):
            return status == errSecInteractionNotAllowed ||
                status == errSecNoDefaultKeychain ||
                status == errSecAuthFailed
        case .readFailed, .writeFailed, .deleteFailed:
            return false
        }
    }
}
