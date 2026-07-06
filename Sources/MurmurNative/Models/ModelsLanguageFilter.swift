import Foundation

enum ModelsLanguageFilter {
    static let allLanguagesID = "all"

    static var selectableLanguages: [TranscriptionLanguage] {
        TranscriptionLanguage.all.filter { $0.code != "auto" }
    }

    static func label(for languageCode: String) -> String {
        if languageCode == allLanguagesID {
            return "All Languages"
        }
        return TranscriptionLanguage.displayName(for: languageCode)
    }

    static func filteredLanguages(searchText: String) -> [TranscriptionLanguage] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard query.isEmpty == false else {
            return selectableLanguages
        }

        return selectableLanguages.filter {
            $0.name.localizedCaseInsensitiveContains(query) ||
                $0.code.localizedCaseInsensitiveContains(query)
        }
    }

    static func supports(languageCode: String, supportedLanguageCodes: Set<String>) -> Bool {
        languageCode == allLanguagesID || supportedLanguageCodes.contains(languageCode)
    }
}

extension TranscriptionLanguage {
    static var multilingualModelLanguageCodes: Set<String> {
        Set(ModelsLanguageFilter.selectableLanguages.map(\.code)).union(["zh"])
    }
}

extension LocalTranscriptionModel {
    var supportedLanguageCodes: Set<String> {
        TranscriptionLanguage.multilingualModelLanguageCodes
    }

    func supports(languageFilter: String) -> Bool {
        ModelsLanguageFilter.supports(
            languageCode: languageFilter,
            supportedLanguageCodes: supportedLanguageCodes
        )
    }
}

extension TranscriptionAPIModel {
    var supportedLanguageCodes: Set<String> {
        TranscriptionLanguage.multilingualModelLanguageCodes
    }

    func supports(languageFilter: String) -> Bool {
        ModelsLanguageFilter.supports(
            languageCode: languageFilter,
            supportedLanguageCodes: supportedLanguageCodes
        )
    }
}

enum ModelsListItem: Equatable, Identifiable {
    case local(String)
    case appleSpeech
    case api(String)

    var id: String {
        switch self {
        case let .local(id):
            "local:\(id)"
        case .appleSpeech:
            "system:\(TranscriptionAPIProvider.appleSpeechModelID)"
        case let .api(id):
            "api:\(id)"
        }
    }
}

struct ModelsListSections: Equatable {
    var yourModels: [ModelsListItem]
    var availableModels: [ModelsListItem]

    var hasAnyVisibleModel: Bool {
        yourModels.isEmpty == false || availableModels.isEmpty == false
    }

    static func make(
        settings: AppSettings,
        localStorageStates: [String: LocalModelStorageState],
        localDownloadStates: [String: LocalModelDownloadState],
        languageFilter: String
    ) -> ModelsListSections {
        var yourModels: [ModelsListItem] = []
        var availableModels: [ModelsListItem] = []
        var assignedIDs = Set<String>()

        for model in LocalTranscriptionModel.catalog where model.supports(languageFilter: languageFilter) {
            let isActive = settings.selectedModel == model.id
            let isDownloaded = localStorageStates[model.id]?.isDownloaded == true
            let isDownloading = localDownloadStates[model.id] != nil
            let item = ModelsListItem.local(model.id)

            if isActive || isDownloaded || isDownloading {
                yourModels.append(item)
            } else {
                availableModels.append(item)
            }
            assignedIDs.insert(item.id)
        }

        let appleSpeechItem = ModelsListItem.appleSpeech
        if ModelsLanguageFilter.supports(
            languageCode: languageFilter,
            supportedLanguageCodes: TranscriptionLanguage.multilingualModelLanguageCodes
        ) {
            if settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID {
                yourModels.append(appleSpeechItem)
            } else {
                availableModels.append(appleSpeechItem)
            }
            assignedIDs.insert(appleSpeechItem.id)
        }

        for model in settings.transcriptionAPIModels where model.supports(languageFilter: languageFilter) {
            let item = ModelsListItem.api(model.id)
            if assignedIDs.insert(item.id).inserted {
                yourModels.append(item)
            }
        }

        yourModels.sort {
            sortKey(for: $0, selectedModel: settings.selectedModel) <
                sortKey(for: $1, selectedModel: settings.selectedModel)
        }

        return ModelsListSections(yourModels: yourModels, availableModels: availableModels)
    }

    private static func sortKey(for item: ModelsListItem, selectedModel: String) -> String {
        let selectedPrefix = item.matches(modelID: selectedModel) ? "0" : "1"
        switch item {
        case let .local(id):
            return "\(selectedPrefix):1:\(id)"
        case .appleSpeech:
            return "\(selectedPrefix):2:\(TranscriptionAPIProvider.appleSpeechModelID)"
        case let .api(id):
            return "\(selectedPrefix):3:\(id)"
        }
    }
}

private extension ModelsListItem {
    func matches(modelID: String) -> Bool {
        switch self {
        case let .local(id), let .api(id):
            id == modelID
        case .appleSpeech:
            modelID == TranscriptionAPIProvider.appleSpeechModelID
        }
    }
}
