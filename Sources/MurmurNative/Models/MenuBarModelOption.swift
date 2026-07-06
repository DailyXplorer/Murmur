import Foundation

struct MenuBarModelOption: Identifiable, Equatable {
    var id: String
    var title: String
    var isSelected: Bool
    var isEnabled: Bool
}

enum MenuBarModelOptions {
    static func make(
        settings: AppSettings,
        localModelStorageStates: [String: LocalModelStorageState]
    ) -> [MenuBarModelOption] {
        var options: [MenuBarModelOption] = [
            MenuBarModelOption(
                id: TranscriptionAPIProvider.appleSpeechModelID,
                title: "Apple Speech",
                isSelected: settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID,
                isEnabled: true
            )
        ]

        for model in LocalTranscriptionModel.catalog {
            let isDownloaded = localModelStorageStates[model.id]?.isDownloaded == true
            let isSelected = settings.selectedModel == model.id
            guard isDownloaded || isSelected else {
                continue
            }

            options.append(
                MenuBarModelOption(
                    id: model.id,
                    title: model.name,
                    isSelected: isSelected,
                    isEnabled: isDownloaded
                )
            )
        }

        options.append(
            contentsOf: settings.transcriptionAPIModels.map { model in
                MenuBarModelOption(
                    id: model.id,
                    title: model.displayName,
                    isSelected: settings.selectedModel == model.id,
                    isEnabled: true
                )
            }
        )

        if !options.contains(where: { $0.id == settings.selectedModel }) {
            options.insert(
                MenuBarModelOption(
                    id: settings.selectedModel,
                    title: settings.selectedTranscriptionModelDisplayName,
                    isSelected: true,
                    isEnabled: false
                ),
                at: 0
            )
        }

        return options
    }
}
