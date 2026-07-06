import Foundation

struct LocalModelDeletionConfirmation: Equatable {
    var title: String
    var message: String
    var destructiveButtonTitle: String
    var cancelButtonTitle: String

    static func request(for model: LocalTranscriptionModel, isActive: Bool) -> LocalModelDeletionConfirmation {
        LocalModelDeletionConfirmation(
            title: "Delete Model",
            message: isActive
                ? "\(model.name) is your active model. Deleting it will stop transcriptions until you select a new model. Are you sure?"
                : "Are you sure you want to delete \(model.name)? You will need to download it again to use it.",
            destructiveButtonTitle: "Delete",
            cancelButtonTitle: "Cancel"
        )
    }
}
