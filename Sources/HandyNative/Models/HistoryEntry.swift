import Foundation

struct PaginatedHistory: Equatable {
    var entries: [HistoryEntry]
    var hasMore: Bool
}

struct HistoryEntry: Identifiable, Equatable {
    let id: Int64
    var fileName: String
    var timestamp: Int64
    var saved: Bool
    var title: String
    var transcriptionText: String
    var postProcessedText: String?
    var postProcessPrompt: String?
    var postProcessRequested: Bool

    var date: Date {
        Date(timeIntervalSince1970: TimeInterval(timestamp))
    }

    var hasTranscription: Bool {
        transcriptionText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
    }

    var outputText: String {
        if let postProcessedText,
           postProcessedText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false {
            return postProcessedText
        }
        return transcriptionText
    }
}
