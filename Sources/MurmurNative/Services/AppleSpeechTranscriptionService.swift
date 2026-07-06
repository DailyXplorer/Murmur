import Foundation
import Speech

enum TranscriptionServiceError: LocalizedError {
    case permissionDenied
    case missingSpeechUsageDescription
    case permissionRequestTimedOut
    case recognitionTimedOut
    case recognizerUnavailable(String)
    case emptyResult

    var errorDescription: String? {
        switch self {
        case .permissionDenied:
            "Speech recognition permission is required to transcribe recordings."
        case .missingSpeechUsageDescription:
            "Apple Speech requires a signed app bundle with NSSpeechRecognitionUsageDescription."
        case .permissionRequestTimedOut:
            "Speech recognition permission request timed out."
        case .recognitionTimedOut:
            "Speech recognition timed out."
        case let .recognizerUnavailable(localeIdentifier):
            "Speech recognition is unavailable for \(localeIdentifier)."
        case .emptyResult:
            "Recording contains no recognized speech."
        }
    }
}

final class AppleSpeechTranscriptionService: @unchecked Sendable {
    private let authorizationTimeout: TimeInterval?
    private let recognitionTimeout: TimeInterval?

    init(authorizationTimeout: TimeInterval? = nil, recognitionTimeout: TimeInterval? = nil) {
        self.authorizationTimeout = authorizationTimeout
        self.recognitionTimeout = recognitionTimeout
    }

    func transcribe(
        fileURL: URL,
        localeIdentifier: String,
        customWords: [String] = [],
        wordCorrectionThreshold: Double = AppSettings.defaults.wordCorrectionThreshold
    ) async throws -> String {
        try Self.ensureSpeechUsageDescription()

        let authorizationStatus = try await Self.authorizationStatusRequestingIfNeeded(
            timeout: authorizationTimeout
        )
        guard authorizationStatus == .authorized else {
            throw TranscriptionServiceError.permissionDenied
        }

        let locale = Self.locale(for: localeIdentifier)
        guard let recognizer = SFSpeechRecognizer(locale: locale), recognizer.isAvailable else {
            throw TranscriptionServiceError.recognizerUnavailable(locale.identifier)
        }

        let request = SFSpeechURLRecognitionRequest(url: fileURL)
        request.shouldReportPartialResults = false
        request.taskHint = .dictation
        request.contextualStrings = customWords
        if recognizer.supportsOnDeviceRecognition {
            request.requiresOnDeviceRecognition = true
        }

        let text = try await Self.recognizedText(
            recognizer: recognizer,
            request: request,
            timeout: recognitionTimeout
        )

        return CustomWordCorrectionService.applyCustomWords(
            to: text,
            customWords: customWords,
            threshold: wordCorrectionThreshold
        )
    }

    private static func authorizationStatusRequestingIfNeeded(
        timeout: TimeInterval?
    ) async throws -> SFSpeechRecognizerAuthorizationStatus {
        let status = SFSpeechRecognizer.authorizationStatus()
        guard status == .notDetermined else {
            return status
        }

        guard let timeout else {
            return await requestAuthorization()
        }

        return try await requestAuthorization(timeout: timeout)
    }

    private static func ensureSpeechUsageDescription() throws {
        let value = Bundle.main.object(forInfoDictionaryKey: "NSSpeechRecognitionUsageDescription") as? String
        guard let value,
              !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            throw TranscriptionServiceError.missingSpeechUsageDescription
        }
    }

    private static func requestAuthorization() async -> SFSpeechRecognizerAuthorizationStatus {
        return await withCheckedContinuation { continuation in
            SFSpeechRecognizer.requestAuthorization { requestedStatus in
                continuation.resume(returning: requestedStatus)
            }
        }
    }

    private static func requestAuthorization(timeout: TimeInterval) async throws -> SFSpeechRecognizerAuthorizationStatus {
        try await withCheckedThrowingContinuation { continuation in
            var timeoutWorkItem: DispatchWorkItem?
            let box = AuthorizationContinuationBox(continuation: continuation) {
                timeoutWorkItem?.cancel()
            }

            let item = DispatchWorkItem {
                box.resume(throwing: TranscriptionServiceError.permissionRequestTimedOut)
            }
            timeoutWorkItem = item
            DispatchQueue.global().asyncAfter(
                deadline: .now() + max(0.1, timeout),
                execute: item
            )

            SFSpeechRecognizer.requestAuthorization { requestedStatus in
                box.resume(returning: requestedStatus)
            }
        }
    }

    private static func timeoutNanoseconds(_ timeout: TimeInterval) -> UInt64 {
        UInt64(max(0.1, timeout) * 1_000_000_000)
    }

    private static func recognizedText(
        recognizer: SFSpeechRecognizer,
        request: SFSpeechURLRecognitionRequest,
        timeout: TimeInterval?
    ) async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
            let taskBox = RecognitionTaskBox()
            var timeoutWorkItem: DispatchWorkItem?
            let box = RecognitionContinuationBox(continuation: continuation) {
                timeoutWorkItem?.cancel()
                taskBox.cancel()
            }

            if let timeout {
                let item = DispatchWorkItem {
                    box.resume(throwing: TranscriptionServiceError.recognitionTimedOut)
                }
                timeoutWorkItem = item
                DispatchQueue.global().asyncAfter(
                    deadline: .now() + max(0.1, timeout),
                    execute: item
                )
            }

            let task = recognizer.recognitionTask(with: request) { result, error in
                if let error {
                    box.resume(throwing: error)
                    return
                }

                guard let result else {
                    return
                }

                if result.isFinal {
                    let transcription = result.bestTranscription.formattedString
                        .trimmingCharacters(in: .whitespacesAndNewlines)
                    if transcription.isEmpty {
                        box.resume(throwing: TranscriptionServiceError.emptyResult)
                    } else {
                        box.resume(returning: transcription)
                    }
                }
            }
            taskBox.set(task)
        }
    }

    private static func locale(for identifier: String) -> Locale {
        if identifier == "auto" || identifier.isEmpty {
            return Locale.current
        }

        return Locale(identifier: identifier)
    }
}

private final class AuthorizationContinuationBox: @unchecked Sendable {
    private let lock = NSLock()
    private var didResume = false
    private let continuation: CheckedContinuation<SFSpeechRecognizerAuthorizationStatus, Error>
    private let onResume: () -> Void

    init(
        continuation: CheckedContinuation<SFSpeechRecognizerAuthorizationStatus, Error>,
        onResume: @escaping () -> Void
    ) {
        self.continuation = continuation
        self.onResume = onResume
    }

    func resume(returning value: SFSpeechRecognizerAuthorizationStatus) {
        lock.lock()
        guard didResume == false else {
            lock.unlock()
            return
        }
        didResume = true
        lock.unlock()

        onResume()
        continuation.resume(returning: value)
    }

    func resume(throwing error: Error) {
        lock.lock()
        guard didResume == false else {
            lock.unlock()
            return
        }
        didResume = true
        lock.unlock()

        onResume()
        continuation.resume(throwing: error)
    }
}

private final class RecognitionContinuationBox {
    private let lock = NSLock()
    private var didResume = false
    private let continuation: CheckedContinuation<String, Error>
    private let onResume: () -> Void

    init(
        continuation: CheckedContinuation<String, Error>,
        onResume: @escaping () -> Void
    ) {
        self.continuation = continuation
        self.onResume = onResume
    }

    func resume(returning value: String) {
        lock.lock()
        guard didResume == false else {
            lock.unlock()
            return
        }
        didResume = true
        lock.unlock()

        onResume()
        continuation.resume(returning: value)
    }

    func resume(throwing error: Error) {
        lock.lock()
        guard didResume == false else {
            lock.unlock()
            return
        }
        didResume = true
        lock.unlock()

        onResume()
        continuation.resume(throwing: error)
    }
}

private final class RecognitionTaskBox {
    private let lock = NSLock()
    private var task: SFSpeechRecognitionTask?
    private var didCancel = false

    func set(_ task: SFSpeechRecognitionTask) {
        lock.lock()
        guard didCancel == false else {
            lock.unlock()
            task.cancel()
            return
        }
        self.task = task
        lock.unlock()
    }

    func cancel() {
        lock.lock()
        guard didCancel == false else {
            lock.unlock()
            return
        }
        didCancel = true
        let task = task
        self.task = nil
        lock.unlock()

        task?.cancel()
    }
}
