import Foundation

struct ProcessedAudioTranscription {
    var transcriptionText: String
    var outputText: String
    var postProcessedText: String?
    var postProcessPrompt: String?
}

enum AudioFileTranscriptionPipeline {
    static func transcribe(
        fileURL: URL,
        settings: AppSettings,
        paths: AppPaths,
        credentialStore: any PostProcessCredentialStoring,
        appleSpeechTranscriptionService: AppleSpeechTranscriptionService? = nil,
        whisperKitTranscriptionService: WhisperKitTranscriptionService = WhisperKitTranscriptionService(),
        postProcessRequested: Bool
    ) async throws -> ProcessedAudioTranscription {
        let text: String
        if settings.selectedTranscriptionAPIModel != nil {
            let rawText = try await APITranscriptionService.transcribe(
                fileURL: fileURL,
                settings: settings,
                credentialStore: credentialStore
            )
            text = CustomWordCorrectionService.applyCustomWords(
                to: rawText,
                customWords: settings.customWords,
                threshold: settings.wordCorrectionThreshold
            )
        } else if let localModel = settings.selectedLocalTranscriptionModel {
            let rawText = try await whisperKitTranscriptionService.transcribe(
                fileURL: fileURL,
                model: localModel,
                settings: settings,
                paths: paths
            )
            text = CustomWordCorrectionService.applyCustomWords(
                to: rawText,
                customWords: settings.customWords,
                threshold: settings.wordCorrectionThreshold
            )
        } else if settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID {
            guard let appleSpeechTranscriptionService else {
                throw TranscriptionEngineSelectionError.appleSpeechUnavailable
            }
            text = try await appleSpeechTranscriptionService.transcribe(
                fileURL: fileURL,
                localeIdentifier: settings.selectedLanguage,
                customWords: settings.customWords,
                wordCorrectionThreshold: settings.wordCorrectionThreshold
            )
        } else {
            throw TranscriptionEngineSelectionError.unsupportedModel(settings.selectedModel)
        }

        return await processRecognizedText(
            text,
            settings: settings,
            credentialStore: credentialStore,
            postProcessRequested: postProcessRequested
        )
    }

    static func processRecognizedText(
        _ text: String,
        settings: AppSettings,
        credentialStore: any PostProcessCredentialStoring,
        postProcessRequested: Bool
    ) async -> ProcessedAudioTranscription {
        let cleanText = PostProcessingService.stripInvisibleCharacters(text)
        let filterLanguage = settings.selectedLanguage == "auto"
            ? settings.appLanguage
            : settings.selectedLanguage
        var filteredText = TranscriptionOutputFilterService.filter(
            cleanText,
            language: filterLanguage,
            customFillerWords: settings.customFillerWords
        )
        if filteredText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
           !cleanText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            // Filtering must never turn a real transcription into silent nothing;
            // if it removed everything, output the unfiltered text instead.
            filteredText = cleanText
        }
        let variantText = ChineseVariantConversionService.convertedText(
            filteredText,
            selectedLanguage: settings.selectedLanguage
        ) ?? filteredText
        let postProcessPrompt = postProcessRequested ? settings.selectedPostProcessPrompt?.prompt : nil
        var outputText = variantText
        var postProcessedText: String?

        if postProcessRequested,
           let processedText = await PostProcessingService.postProcessTranscription(
               settings: settings,
               transcription: variantText,
               credentialStore: credentialStore
           ),
           !processedText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            outputText = processedText
            postProcessedText = processedText
        } else if postProcessRequested == false,
                  variantText != cleanText {
            postProcessedText = variantText
        }

        return ProcessedAudioTranscription(
            transcriptionText: cleanText,
            outputText: outputText,
            postProcessedText: postProcessedText,
            postProcessPrompt: postProcessRequested && postProcessedText != nil ? postProcessPrompt : nil
        )
    }
}

enum TranscriptionEngineSelectionError: LocalizedError {
    case unsupportedModel(String)
    case appleSpeechUnavailable

    var errorDescription: String? {
        switch self {
        case let .unsupportedModel(modelID):
            "The selected transcription model '\(modelID)' is not available in the native Swift app yet."
        case .appleSpeechUnavailable:
            "Apple Speech is unavailable in this native validation context."
        }
    }
}
