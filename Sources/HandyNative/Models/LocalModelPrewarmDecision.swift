import Foundation

enum LocalModelPrewarmDecision {
    static func modelToPrewarm(
        settings: AppSettings,
        storageStates: [String: LocalModelStorageState]
    ) -> LocalTranscriptionModel? {
        guard let model = settings.selectedLocalTranscriptionModel,
              storageStates[model.id]?.isDownloaded == true
        else {
            return nil
        }

        return model
    }
}
