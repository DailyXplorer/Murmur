import Foundation

struct NativeSmokeTranscriptionSettingsConfiguration {
    var settings: AppSettings
    var credentialStore: any PostProcessCredentialStoring
    var appleSpeechTranscriptionService: AppleSpeechTranscriptionService?
}

enum NativeSmokeTranscriptionSettingsResolver {
    static func configuration(
        modelID: String,
        language: String?,
        useSelectedSettings: Bool,
        paths: AppPaths
    ) async throws -> NativeSmokeTranscriptionSettingsConfiguration {
        let credentialStore: any PostProcessCredentialStoring
        var settings: AppSettings

        if useSelectedSettings {
            let localCredentialStore = LocalPostProcessCredentialStore(paths: paths)
            credentialStore = localCredentialStore
            settings = SettingsStore(paths: paths, credentialStore: localCredentialStore).load()
        } else if let model = LocalTranscriptionModel.model(for: modelID) {
            credentialStore = InMemorySmokeTranscriptionCredentialStore()
            settings = AppSettings.defaults
            settings.selectedModel = model.id
        } else if modelID == TranscriptionAPIProvider.appleSpeechModelID {
            credentialStore = InMemorySmokeTranscriptionCredentialStore()
            settings = AppSettings.defaults
            settings.selectedModel = TranscriptionAPIProvider.appleSpeechModelID
        } else {
            throw NativeSmokeTranscriptionSettingsError.unsupportedModel(modelID)
        }

        if let language = language?.trimmingCharacters(in: .whitespacesAndNewlines),
           !language.isEmpty {
            settings.selectedLanguage = language
        }

        let appleSpeechService: AppleSpeechTranscriptionService?
        if settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID {
            appleSpeechService = AppleSpeechTranscriptionService(authorizationTimeout: 5, recognitionTimeout: 10)
        } else {
            appleSpeechService = nil
        }

        return NativeSmokeTranscriptionSettingsConfiguration(
            settings: settings,
            credentialStore: credentialStore,
            appleSpeechTranscriptionService: appleSpeechService
        )
    }
}

enum NativeSmokeTranscriptionSettingsError: LocalizedError {
    case unsupportedModel(String)

    var errorDescription: String? {
        switch self {
        case let .unsupportedModel(modelID):
            "Transcription model '\(modelID)' is not available in the native Swift smoke runner."
        }
    }
}

private struct InMemorySmokeTranscriptionCredentialStore: PostProcessCredentialStoring {
    func readAPIKey(providerID _: String) throws -> String? {
        nil
    }

    func saveAPIKey(_: String, providerID _: String) throws {}

    func deleteAPIKey(providerID _: String) throws {}
}
