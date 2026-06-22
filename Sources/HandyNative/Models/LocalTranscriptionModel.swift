import Foundation

struct LocalTranscriptionModel: Identifiable, Equatable {
    var id: String
    var name: String
    var description: String
    var whisperKitModelID: String
    var sizeDescription: String
    var speedScore: Double
    var accuracyScore: Double
    var supportsTranslation: Bool
    var isRecommended: Bool

    static let appleSpeechID = TranscriptionAPIProvider.appleSpeechModelID

    static let catalog: [LocalTranscriptionModel] = [
        LocalTranscriptionModel(
            id: "tiny",
            name: "Whisper Tiny",
            description: "Smallest local Core ML model. Useful for quick tests.",
            whisperKitModelID: "tiny",
            sizeDescription: "Tiny",
            speedScore: 0.95,
            accuracyScore: 0.45,
            supportsTranslation: true,
            isRecommended: false
        ),
        LocalTranscriptionModel(
            id: "base",
            name: "Whisper Base",
            description: "Fast local Core ML model with better accuracy than Tiny.",
            whisperKitModelID: "base",
            sizeDescription: "Base",
            speedScore: 0.85,
            accuracyScore: 0.55,
            supportsTranslation: true,
            isRecommended: false
        ),
        LocalTranscriptionModel(
            id: "small",
            name: "Whisper Small",
            description: "Balanced local Core ML model compatible with Handy's Small selection.",
            whisperKitModelID: "small",
            sizeDescription: "Small",
            speedScore: 0.72,
            accuracyScore: 0.68,
            supportsTranslation: true,
            isRecommended: false
        ),
        LocalTranscriptionModel(
            id: "medium",
            name: "Whisper Medium",
            description: "Good accuracy, medium speed.",
            whisperKitModelID: "medium",
            sizeDescription: "Medium",
            speedScore: 0.60,
            accuracyScore: 0.75,
            supportsTranslation: true,
            isRecommended: false
        ),
        LocalTranscriptionModel(
            id: "turbo",
            name: "Whisper Turbo",
            description: "Fast, high quality local Core ML model for Apple Silicon.",
            whisperKitModelID: "large-v3-v20240930_turbo",
            sizeDescription: "Large Turbo",
            speedScore: 0.62,
            accuracyScore: 0.86,
            supportsTranslation: false,
            isRecommended: true
        ),
        LocalTranscriptionModel(
            id: "large",
            name: "Whisper Large",
            description: "Highest accuracy local Core ML model, with a larger first-use download.",
            whisperKitModelID: "large-v3-v20240930_626MB",
            sizeDescription: "Large",
            speedScore: 0.45,
            accuracyScore: 0.90,
            supportsTranslation: true,
            isRecommended: false
        ),
    ]

    static func model(for id: String) -> LocalTranscriptionModel? {
        catalog.first { $0.id == id }
    }

    var cacheNameHints: [String] {
        [
            whisperKitModelID,
            "openai_whisper-\(whisperKitModelID)",
        ]
    }
}

enum AppleSpeechModelPresentation {
    static let title = "Apple Speech"
    static let description = "Improved macOS 27 dictation on Apple silicon Macs: " +
        "MacBook Neo, MacBook Air/Pro 2020+, iMac 2021+, Mac mini 2020+, " +
        "Mac Studio 2022+, Mac Pro with Apple silicon."
    static let sizeDescription = "System"
    static let speedScore = 0.90
    static let accuracyScore = 0.84
}
