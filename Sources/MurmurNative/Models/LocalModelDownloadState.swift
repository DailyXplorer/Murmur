import Foundation

struct LocalModelDownloadState: Equatable {
    var modelID: String
    var fractionCompleted: Double?
    var isCancelling: Bool

    init(modelID: String, fractionCompleted: Double? = nil, isCancelling: Bool = false) {
        self.modelID = modelID
        self.fractionCompleted = fractionCompleted.map { min(max($0, 0), 1) }
        self.isCancelling = isCancelling
    }

    init(modelID: String, progress: Progress) {
        self.init(modelID: modelID, fractionCompleted: progress.fractionCompleted)
    }

    var percentComplete: Int? {
        fractionCompleted.map { Int(($0 * 100).rounded()) }
    }

    var statusLabel: String {
        if isCancelling {
            return "Canceling"
        }
        if let percentComplete {
            return "Downloading \(percentComplete)%"
        }
        return "Downloading"
    }
}
