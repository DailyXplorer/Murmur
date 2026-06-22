import Foundation
import Security

protocol PostProcessCredentialStoring: Sendable {
    func readAPIKey(providerID: String) throws -> String?
    func hasAPIKey(providerID: String) -> Bool
    func saveAPIKey(_ apiKey: String, providerID: String) throws
    func deleteAPIKey(providerID: String) throws
}

extension PostProcessCredentialStoring {
    func hasAPIKey(providerID: String) -> Bool {
        guard let key = try? readAPIKey(providerID: providerID) else {
            return false
        }
        return !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    func importAPIKeys(_ apiKeys: [String: String], overwriteExisting: Bool = false) throws {
        for (providerID, apiKey) in apiKeys {
            let trimmedKey = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !providerID.isEmpty, !trimmedKey.isEmpty else {
                continue
            }
            if !overwriteExisting, hasAPIKey(providerID: providerID) {
                continue
            }
            try saveAPIKey(trimmedKey, providerID: providerID)
        }
    }
}

enum PostProcessCredentialStoreError: LocalizedError {
    case readFailed(URL, Error)
    case writeFailed(URL, Error)
    case deleteFailed(URL, Error)
    case keychainReadFailed(String, OSStatus)
    case keychainWriteFailed(String, OSStatus)
    case keychainDeleteFailed(String, OSStatus)

    var errorDescription: String? {
        switch self {
        case let .readFailed(url, error):
            "Unable to read API credentials at \(url.path): \(error.localizedDescription)"
        case let .writeFailed(url, error):
            "Unable to save API credentials at \(url.path): \(error.localizedDescription)"
        case let .deleteFailed(url, error):
            "Unable to delete API credentials at \(url.path): \(error.localizedDescription)"
        case let .keychainReadFailed(providerID, status):
            "Unable to read API credentials for \(providerID) from Keychain: \(Self.keychainMessage(status))."
        case let .keychainWriteFailed(providerID, status):
            "Unable to save API credentials for \(providerID) to Keychain: \(Self.keychainMessage(status))."
        case let .keychainDeleteFailed(providerID, status):
            "Unable to delete API credentials for \(providerID) from Keychain: \(Self.keychainMessage(status))."
        }
    }

    private static func keychainMessage(_ status: OSStatus) -> String {
        if let message = SecCopyErrorMessageString(status, nil) as String? {
            return "\(message) (\(status))"
        }
        return "OSStatus \(status)"
    }
}

final class LocalPostProcessCredentialStore: PostProcessCredentialStoring {
    enum StorageMode {
        case keychain
        case file
    }

    private let credentialsURL: URL
    private let keychainService: String
    private let storageMode: StorageMode

    init(
        paths: AppPaths? = try? AppPaths.resolve(),
        storageMode: StorageMode? = nil,
        keychainService: String = "\(AppPaths.sharedAppDataIdentifier).api-credentials"
    ) {
        let resolvedPaths = paths ?? AppPaths(
            appDataDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("HandyNative", isDirectory: true),
            recordingsDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("HandyNative/recordings", isDirectory: true),
            modelsDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("HandyNative/models", isDirectory: true),
            logsDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("HandyNative/logs", isDirectory: true)
        )
        credentialsURL = resolvedPaths.appDataDirectory.appendingPathComponent("api_credentials.json")
        self.keychainService = keychainService
        self.storageMode = storageMode ?? Self.defaultStorageMode()
    }

    func readAPIKey(providerID: String) throws -> String? {
        let normalizedProviderID = providerID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedProviderID.isEmpty else {
            return nil
        }

        switch storageMode {
        case .file:
            let credentials = try loadFileCredentials()
            return credentials[normalizedProviderID]
        case .keychain:
            if let key = try readKeychainAPIKey(providerID: normalizedProviderID) {
                return key
            }
            guard let legacyKey = try loadFileCredentials()[normalizedProviderID]?
                .trimmingCharacters(in: .whitespacesAndNewlines),
                !legacyKey.isEmpty
            else {
                return nil
            }
            try saveKeychainAPIKey(legacyKey, providerID: normalizedProviderID)
            try removeFileCredential(providerID: normalizedProviderID)
            return legacyKey
        }
    }

    func saveAPIKey(_ apiKey: String, providerID: String) throws {
        let normalizedProviderID = providerID.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedKey = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedProviderID.isEmpty else {
            return
        }
        guard !trimmedKey.isEmpty else {
            try deleteAPIKey(providerID: normalizedProviderID)
            return
        }

        switch storageMode {
        case .file:
            var credentials = try loadFileCredentials()
            credentials[normalizedProviderID] = trimmedKey
            try saveFileCredentials(credentials)
        case .keychain:
            try saveKeychainAPIKey(trimmedKey, providerID: normalizedProviderID)
            try removeFileCredential(providerID: normalizedProviderID)
        }
    }

    func deleteAPIKey(providerID: String) throws {
        let normalizedProviderID = providerID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedProviderID.isEmpty else {
            return
        }

        switch storageMode {
        case .file:
            try removeFileCredential(providerID: normalizedProviderID)
        case .keychain:
            try deleteKeychainAPIKey(providerID: normalizedProviderID)
            try removeFileCredential(providerID: normalizedProviderID)
        }
    }

    private static func defaultStorageMode() -> StorageMode {
        let smokeDataDirectory = ProcessInfo.processInfo.environment["HANDY_APP_DATA_DIR"]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return smokeDataDirectory?.isEmpty == false ? .file : .keychain
    }

    private func loadFileCredentials() throws -> [String: String] {
        guard FileManager.default.fileExists(atPath: credentialsURL.path) else {
            return [:]
        }

        do {
            let data = try Data(contentsOf: credentialsURL)
            return (try? JSONDecoder().decode([String: String].self, from: data)) ?? [:]
        } catch {
            throw PostProcessCredentialStoreError.readFailed(credentialsURL, error)
        }
    }

    private func saveFileCredentials(_ credentials: [String: String]) throws {
        guard !credentials.isEmpty else {
            do {
                try FileManager.default.removeItem(at: credentialsURL)
            } catch CocoaError.fileNoSuchFile {
            } catch {
                throw PostProcessCredentialStoreError.deleteFailed(credentialsURL, error)
            }
            return
        }

        do {
            try FileManager.default.createDirectory(
                at: credentialsURL.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            let data = try JSONEncoder().encode(credentials)
            try data.write(to: credentialsURL, options: [.atomic])
            try FileManager.default.setAttributes(
                [.posixPermissions: 0o600],
                ofItemAtPath: credentialsURL.path
            )
        } catch {
            throw PostProcessCredentialStoreError.writeFailed(credentialsURL, error)
        }
    }

    private func removeFileCredential(providerID: String) throws {
        var credentials = try loadFileCredentials()
        guard credentials.removeValue(forKey: providerID) != nil else {
            return
        }
        try saveFileCredentials(credentials)
    }

    private func keychainQuery(providerID: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: providerID,
        ]
    }

    private func readKeychainAPIKey(providerID: String) throws -> String? {
        var query = keychainQuery(providerID: providerID)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess else {
            throw PostProcessCredentialStoreError.keychainReadFailed(providerID, status)
        }
        guard let data = item as? Data else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    private func saveKeychainAPIKey(_ apiKey: String, providerID: String) throws {
        let valueData = Data(apiKey.utf8)
        let query = keychainQuery(providerID: providerID)
        let updateStatus = SecItemUpdate(
            query as CFDictionary,
            [kSecValueData as String: valueData] as CFDictionary
        )

        if updateStatus == errSecSuccess {
            return
        }
        if updateStatus != errSecItemNotFound {
            throw PostProcessCredentialStoreError.keychainWriteFailed(providerID, updateStatus)
        }

        var addQuery = query
        addQuery[kSecValueData as String] = valueData
        let addStatus = SecItemAdd(addQuery as CFDictionary, nil)
        guard addStatus == errSecSuccess else {
            throw PostProcessCredentialStoreError.keychainWriteFailed(providerID, addStatus)
        }
    }

    private func deleteKeychainAPIKey(providerID: String) throws {
        let status = SecItemDelete(keychainQuery(providerID: providerID) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw PostProcessCredentialStoreError.keychainDeleteFailed(providerID, status)
        }
    }
}
