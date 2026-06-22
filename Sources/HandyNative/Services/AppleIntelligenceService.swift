import Foundation

#if canImport(FoundationModels)
import FoundationModels
#endif

#if canImport(FoundationModels) && arch(arm64)
@available(macOS 26.0, *)
@Generable
private struct CleanedTranscript: Sendable {
    let cleanedText: String
}
#endif

protocol AppleIntelligenceProcessing: Sendable {
    func process(
        systemPrompt: String,
        userContent: String,
        tokenLimit: Int?
    ) async throws -> String
}

enum AppleIntelligenceServiceError: LocalizedError, Equatable {
    case unsupportedRuntime
    case unavailable
    case emptyResponse

    var errorDescription: String? {
        switch self {
        case .unsupportedRuntime:
            "Apple Intelligence requires an Apple Silicon Mac running macOS 26 or newer."
        case .unavailable:
            "Apple Intelligence is not currently available on this device."
        case .emptyResponse:
            "Apple Intelligence returned an empty response."
        }
    }
}

struct AppleIntelligenceService: AppleIntelligenceProcessing {
    static func tokenLimit(from modelValue: String?) -> Int? {
        guard let modelValue else {
            return nil
        }

        let trimmed = modelValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let value = Int(trimmed), value > 0 else {
            return nil
        }
        return value
    }

    static func availability() -> AppleIntelligenceAvailability {
        #if canImport(FoundationModels) && arch(arm64)
        guard #available(macOS 26.0, *) else {
            return .unavailable(AppleIntelligenceServiceError.unsupportedRuntime.localizedDescription)
        }

        let model = SystemLanguageModel.default
        switch model.availability {
        case .available:
            return .available
        case .unavailable:
            return .unavailable(AppleIntelligenceServiceError.unavailable.localizedDescription)
        }
        #else
        return .unavailable(AppleIntelligenceServiceError.unsupportedRuntime.localizedDescription)
        #endif
    }

    func process(
        systemPrompt: String,
        userContent: String,
        tokenLimit: Int?
    ) async throws -> String {
        #if canImport(FoundationModels) && arch(arm64)
        guard #available(macOS 26.0, *) else {
            throw AppleIntelligenceServiceError.unsupportedRuntime
        }

        let model = SystemLanguageModel.default
        guard case .available = model.availability else {
            throw AppleIntelligenceServiceError.unavailable
        }

        let session = LanguageModelSession(
            model: model,
            instructions: systemPrompt
        )

        let output: String
        do {
            let structured = try await session.respond(
                to: userContent,
                generating: CleanedTranscript.self
            )
            output = structured.content.cleanedText
        } catch {
            let fallback = try await session.respond(to: userContent)
            output = fallback.content
        }

        let cleaned = Self.truncatedText(output, limit: tokenLimit)
        guard cleaned.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false else {
            throw AppleIntelligenceServiceError.emptyResponse
        }
        return PostProcessingService.stripInvisibleCharacters(cleaned)
        #else
        throw AppleIntelligenceServiceError.unsupportedRuntime
        #endif
    }

    static func truncatedText(_ text: String, limit: Int?) -> String {
        guard let limit, limit > 0 else {
            return text
        }

        let words = text.split(
            maxSplits: .max,
            omittingEmptySubsequences: true,
            whereSeparator: { $0.isWhitespace || $0.isNewline }
        )
        guard words.count > limit else {
            return text
        }
        return words.prefix(limit).joined(separator: " ")
    }
}
