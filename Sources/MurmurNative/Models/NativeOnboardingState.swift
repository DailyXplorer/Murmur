import Foundation

enum NativeOnboardingStep: Equatable {
    case checking
    case permissions(returningUser: Bool)
    case model
    case done
}

enum NativeOnboardingEvaluator {
    static func nextStep(
        permissionSnapshot: PermissionSnapshot,
        settings: AppSettings,
        localModelStorageStates: [String: LocalModelStorageState],
        transcriptionAPIKeyConfigured: Bool,
        bypass: Bool
    ) -> NativeOnboardingStep {
        guard bypass == false else {
            return .done
        }

        if permissionSnapshot.microphone == .unknown {
            return .checking
        }

        let hasUsableModel = hasUsableTranscriptionModel(
            settings: settings,
            localModelStorageStates: localModelStorageStates,
            transcriptionAPIKeyConfigured: transcriptionAPIKeyConfigured,
            speechRecognition: permissionSnapshot.speechRecognition
        )
        let needsSpeechRecognition = settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID &&
            permissionSnapshot.speechRecognition != .granted

        guard permissionSnapshot.accessibilityTrusted,
              permissionSnapshot.microphone == .granted,
              needsSpeechRecognition == false
        else {
            return .permissions(returningUser: hasUsableModel)
        }

        if settings.nativeOnboardingCompleted {
            return .done
        }

        return hasUsableModel ? .done : .model
    }

    static func hasUsableTranscriptionModel(
        settings: AppSettings,
        localModelStorageStates: [String: LocalModelStorageState],
        transcriptionAPIKeyConfigured: Bool,
        speechRecognition: PermissionSnapshot.SpeechRecognition = .granted
    ) -> Bool {
        if selectedTranscriptionModelIsUsable(
            settings: settings,
            localModelStorageStates: localModelStorageStates,
            transcriptionAPIKeyConfigured: transcriptionAPIKeyConfigured,
            speechRecognition: speechRecognition
        ) {
            return true
        }

        if localModelStorageStates.values.contains(where: \.isDownloaded) {
            return true
        }

        if speechRecognition == .granted {
            return true
        }

        return settings.transcriptionAPIModels.contains { apiModel in
            guard let provider = settings.transcriptionAPIProviders.first(where: { $0.id == apiModel.providerID }) else {
                return false
            }

            if provider.requiresAPIKey == false {
                return true
            }

            return apiModel.providerID == settings.transcriptionAPIProviderID && transcriptionAPIKeyConfigured
        }
    }

    private static func selectedTranscriptionModelIsUsable(
        settings: AppSettings,
        localModelStorageStates: [String: LocalModelStorageState],
        transcriptionAPIKeyConfigured: Bool,
        speechRecognition: PermissionSnapshot.SpeechRecognition
    ) -> Bool {
        if settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID {
            return speechRecognition == .granted
        }

        if let localModel = settings.selectedLocalTranscriptionModel {
            return localModelStorageStates[localModel.id]?.isDownloaded == true
        }

        guard let apiModel = settings.selectedTranscriptionAPIModel,
              let provider = settings.transcriptionAPIProviders.first(where: { $0.id == apiModel.providerID })
        else {
            return false
        }

        if provider.requiresAPIKey == false {
            return true
        }

        return apiModel.providerID == settings.transcriptionAPIProviderID && transcriptionAPIKeyConfigured
    }
}
