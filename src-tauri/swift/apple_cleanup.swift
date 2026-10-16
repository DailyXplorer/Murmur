// C ABI over Apple's on-device Foundation Models, for AI cleanup.
// Status codes are mirrored in src/cleanup/apple.rs.
//
// The framework is weak-linked and every use sits behind
// `canImport(FoundationModels)` and `#available(macOS 26.0, *)`, so the app
// builds with older SDKs and launches on older macOS.

import Foundation
#if canImport(FoundationModels)
import FoundationModels
#endif

private let available: Int32 = 0
private let deviceNotEligible: Int32 = 1
private let appleIntelligenceNotEnabled: Int32 = 2
private let modelNotReady: Int32 = 3
private let unsupportedOS: Int32 = 4
private let unavailable: Int32 = 5

private let ok: Int32 = 0
private let guardrail: Int32 = 1
private let contextTooLong: Int32 = 2
private let timedOut: Int32 = 3
private let refused: Int32 = 4
private let notAvailable: Int32 = 5
private let failed: Int32 = 6
private let unsupportedLanguage: Int32 = 7
private let rateLimited: Int32 = 8

public typealias MurmurAppleCallback =
    @convention(c) (UnsafeMutableRawPointer?, Int32, UnsafePointer<CChar>?) -> Void

/// Carries the caller's context and callback into the generation task.
/// The caller hands over the context and expects exactly one callback.
private struct Reply: @unchecked Sendable {
    let context: UnsafeMutableRawPointer?
    let callback: MurmurAppleCallback

    func send(_ status: Int32, _ text: String? = nil) {
        guard let text else {
            callback(context, status, nil)
            return
        }
        text.withCString { callback(context, status, $0) }
    }
}

@_cdecl("murmur_apple_availability")
public func murmurAppleAvailability() -> Int32 {
    #if canImport(FoundationModels)
    if #available(macOS 26.0, *) {
        let availability = SystemLanguageModel.default.availability
        if availability == .available { return available }
        if availability == .unavailable(.deviceNotEligible) { return deviceNotEligible }
        if availability == .unavailable(.appleIntelligenceNotEnabled) {
            return appleIntelligenceNotEnabled
        }
        if availability == .unavailable(.modelNotReady) { return modelNotReady }
        return unavailable
    }
    #endif
    let macOS26 = OperatingSystemVersion(majorVersion: 26, minorVersion: 0, patchVersion: 0)
    return ProcessInfo.processInfo.isOperatingSystemAtLeast(macOS26) ? unavailable : unsupportedOS
}

/// Whether the model works in `language`, a code such as "fr" or "zh-Hans".
/// `supportsLocale` also counts the model's language fallbacks.
@_cdecl("murmur_apple_supports_language")
public func murmurAppleSupportsLanguage(_ language: UnsafePointer<CChar>) -> Bool {
    #if canImport(FoundationModels)
    if #available(macOS 26.0, *) {
        let locale = Locale(identifier: String(cString: language))
        return SystemLanguageModel.default.supportsLocale(locale)
    }
    #endif
    return false
}

/// Runs one fresh session and calls `callback` once, from a background task,
/// with the reply text or an error status. Returns immediately.
@_cdecl("murmur_apple_cleanup")
public func murmurAppleCleanup(
    _ instructions: UnsafePointer<CChar>,
    _ prompt: UnsafePointer<CChar>,
    _ maxResponseTokens: Int32,
    _ context: UnsafeMutableRawPointer?,
    _ callback: MurmurAppleCallback
) {
    let reply = Reply(context: context, callback: callback)
    #if canImport(FoundationModels)
    if #available(macOS 26.0, *) {
        let instructions = String(cString: instructions)
        let prompt = String(cString: prompt)
        let maxResponseTokens = Int(maxResponseTokens)
        Task.detached(priority: .userInitiated) {
            do {
                let session = LanguageModelSession(instructions: instructions)
                let options = GenerationOptions(
                    temperature: 0,
                    maximumResponseTokens: maxResponseTokens
                )
                let response = try await session.respond(to: prompt, options: options)
                reply.send(ok, response.content)
            } catch {
                reply.send(status(of: error))
            }
        }
        return
    }
    #endif
    reply.send(notAvailable)
}

/// Classifies by enum case name, which is the same in the macOS 26 SDK's
/// `GenerationError` and the macOS 27 SDK's `LanguageModelError`. Only the
/// case name is read; the error's payload may quote the prompt.
private func status(of error: Error) -> Int32 {
    switch Mirror(reflecting: error).children.first?.label {
    case "guardrailViolation": return guardrail
    case "exceededContextWindowSize", "contextSizeExceeded": return contextTooLong
    case "timeout": return timedOut
    case "refusal": return refused
    case "assetsUnavailable": return notAvailable
    case "unsupportedLanguageOrLocale": return unsupportedLanguage
    case "rateLimited": return rateLimited
    default: return failed
    }
}
