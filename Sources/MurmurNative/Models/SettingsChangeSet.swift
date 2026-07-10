import Foundation

/// Answers "which expensive side-effects does this settings mutation require?"
/// Pure value logic so it can be exhaustively unit-tested.
struct SettingsChangeSet {
    let shortcutsChanged: Bool          // bindings or postProcessEnabled (its shortcut registers conditionally)
    let transcriptionCredentialsChanged: Bool
    let postProcessCredentialsChanged: Bool
    let historyRetentionChanged: Bool

    init(old: AppSettings, new: AppSettings) {
        shortcutsChanged = old.shortcutBindings != new.shortcutBindings
            || old.postProcessEnabled != new.postProcessEnabled
        transcriptionCredentialsChanged = old.transcriptionAPIProviderID != new.transcriptionAPIProviderID
            || old.transcriptionAPIProviders != new.transcriptionAPIProviders
            || old.selectedModel != new.selectedModel
            || old.transcriptionAPIModels != new.transcriptionAPIModels
        postProcessCredentialsChanged = old.postProcessProviderID != new.postProcessProviderID
            || old.postProcessProviders != new.postProcessProviders
        historyRetentionChanged = old.historyLimit != new.historyLimit
            || old.recordingRetentionPeriod != new.recordingRetentionPeriod
    }
}
