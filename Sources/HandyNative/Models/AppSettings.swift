import Foundation

enum AudioFeedbackTheme: String, Codable, CaseIterable, Equatable {
    case marimba
    case pop
    case custom

    var title: String {
        switch self {
        case .marimba: "Marimba"
        case .pop: "Pop"
        case .custom: "Custom"
        }
    }
}

enum NativeLogLevel: String, Codable, CaseIterable, Equatable {
    case error
    case warn
    case info
    case debug
    case trace

    var title: String {
        switch self {
        case .error:
            "Error"
        case .warn:
            "Warn"
        case .info:
            "Info"
        case .debug:
            "Debug"
        case .trace:
            "Trace"
        }
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let rawValue = try? container.decode(String.self),
           let level = Self(rawValue: rawValue.lowercased()) {
            self = level
            return
        }

        if let rawValue = try? container.decode(Int.self),
           let level = Self.numericLevel(rawValue) {
            self = level
            return
        }

        throw DecodingError.dataCorruptedError(
            in: container,
            debugDescription: "Unsupported log level."
        )
    }

    private static func numericLevel(_ rawValue: Int) -> NativeLogLevel? {
        switch rawValue {
        case 1: .trace
        case 2: .debug
        case 3: .info
        case 4: .warn
        case 5: .error
        default: nil
        }
    }

    var severity: Int {
        switch self {
        case .trace: 0
        case .debug: 1
        case .info: 2
        case .warn: 3
        case .error: 4
        }
    }

    func allows(_ eventLevel: NativeLogLevel) -> Bool {
        eventLevel.severity >= severity
    }
}

enum KeyboardImplementationSetting: String, Codable, CaseIterable, Equatable {
    case nativeEventTap = "native_event_tap"

    var title: String {
        switch self {
        case .nativeEventTap:
            "Handy Native"
        }
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        let rawValue = (try? container.decode(String.self)) ?? ""

        self = Self(rawValue: rawValue) ?? .nativeEventTap
    }
}

enum RecordingRetentionPeriod: String, Codable, CaseIterable, Equatable {
    case never
    case preserveLimit = "preserve_limit"
    case days3
    case weeks2
    case months3

    func title(historyLimit: Int) -> String {
        switch self {
        case .never:
            "Never delete"
        case .preserveLimit:
            "Preserve last \(historyLimit)"
        case .days3:
            "3 days"
        case .weeks2:
            "2 weeks"
        case .months3:
            "3 months"
        }
    }

    var retentionInterval: TimeInterval? {
        switch self {
        case .never, .preserveLimit:
            nil
        case .days3:
            3 * 24 * 60 * 60
        case .weeks2:
            2 * 7 * 24 * 60 * 60
        case .months3:
            3 * 30 * 24 * 60 * 60
        }
    }
}

enum OverlayPosition: String, Codable, CaseIterable, Equatable {
    case none
    case top
    case bottom

    var title: String {
        switch self {
        case .none:
            "None"
        case .top:
            "Top"
        case .bottom:
            "Bottom"
        }
    }
}

enum ModelUnloadTimeout: String, Codable, CaseIterable, Equatable {
    case never
    case immediately
    case min2
    case min5
    case min10
    case min15
    case hour1
    case sec15

    static let standardCases: [ModelUnloadTimeout] = [.never, .immediately, .min2, .min5, .min10, .min15, .hour1]

    var title: String {
        switch self {
        case .never:
            "Never"
        case .immediately:
            "Immediately"
        case .min2:
            "After 2 minutes"
        case .min5:
            "After 5 minutes"
        case .min10:
            "After 10 minutes"
        case .min15:
            "After 15 minutes"
        case .hour1:
            "After 1 hour"
        case .sec15:
            "After 15 seconds (Debug)"
        }
    }

    var unloadDelaySeconds: UInt64? {
        switch self {
        case .never:
            nil
        case .immediately:
            0
        case .min2:
            2 * 60
        case .min5:
            5 * 60
        case .min10:
            10 * 60
        case .min15:
            15 * 60
        case .hour1:
            60 * 60
        case .sec15:
            15
        }
    }
}

enum PasteMethod: String, Codable, CaseIterable, Equatable {
    case commandV = "ctrl_v"
    case direct
    case none
    case commandShiftV = "ctrl_shift_v"
    case shiftInsert = "shift_insert"
    case externalScript = "external_script"

    static let macOSCases: [PasteMethod] = [.commandV, .direct, .none]

    var title: String {
        switch self {
        case .commandV:
            "Clipboard (Cmd+V)"
        case .direct:
            "Direct"
        case .none:
            "None"
        case .commandShiftV:
            "Clipboard (Cmd+Shift+V)"
        case .shiftInsert:
            "Clipboard (Shift+Insert)"
        case .externalScript:
            "External Script"
        }
    }

    var macOSCompatible: PasteMethod {
        switch self {
        case .commandV, .direct, .none, .commandShiftV:
            self
        case .shiftInsert, .externalScript:
            .commandV
        }
    }
}

enum ClipboardHandling: String, Codable, CaseIterable, Equatable {
    case dontModify = "dont_modify"
    case copyToClipboard = "copy_to_clipboard"

    var title: String {
        switch self {
        case .dontModify:
            "Don't modify"
        case .copyToClipboard:
            "Copy to clipboard"
        }
    }
}

enum AutoSubmitKey: String, Codable, CaseIterable, Equatable {
    case enter
    case controlEnter = "ctrl_enter"
    case commandEnter = "cmd_enter"

    var title: String {
        switch self {
        case .enter:
            "Enter"
        case .controlEnter:
            "Ctrl+Enter"
        case .commandEnter:
            "Cmd+Enter"
        }
    }
}

enum WhisperAcceleratorSetting: String, Codable, CaseIterable, Equatable {
    case auto
    case cpu
    case gpu

    var title: String {
        switch self {
        case .auto:
            "Auto"
        case .cpu:
            "CPU"
        case .gpu:
            "GPU"
        }
    }
}

enum OrtAcceleratorSetting: String, Codable, CaseIterable, Equatable {
    case auto
    case cpu
    case cuda
    case directml
    case rocm

    var title: String {
        switch self {
        case .auto:
            "Auto"
        case .cpu:
            "CPU"
        case .cuda:
            "CUDA"
        case .directml:
            "DirectML"
        case .rocm:
            "ROCm"
        }
    }
}

struct AppSettings: Codable, Equatable {
    var shortcutBindings: [String: ShortcutBinding]
    var pushToTalk: Bool
    var audioFeedback: Bool
    var audioFeedbackVolume: Double
    var soundTheme: AudioFeedbackTheme
    var selectedMicrophoneName: String?
    var clamshellMicrophoneName: String?
    var selectedOutputDeviceName: String?
    var alwaysOnMicrophone: Bool
    var muteWhileRecording: Bool
    var startHidden: Bool
    var autostartEnabled: Bool
    var updateChecksEnabled: Bool
    var showMenuBarIcon: Bool
    var showOverlay: Bool
    var overlayPosition: OverlayPosition
    var appTheme: AppTheme
    var appLanguage: String
    var modelUnloadTimeout: ModelUnloadTimeout
    var selectedModel: String
    var transcriptionAPIProviderID: String
    var transcriptionAPIProviders: [TranscriptionAPIProvider]
    var transcriptionAPIModels: [TranscriptionAPIModel]
    var selectedLanguage: String
    var translateToEnglish: Bool
    var customWords: [String]
    var customFillerWords: [String]?
    var wordCorrectionThreshold: Double
    var pasteMethod: PasteMethod
    var pasteDelayMilliseconds: Int
    var extraRecordingBufferMilliseconds: Int
    var appendTrailingSpace: Bool
    var clipboardHandling: ClipboardHandling
    var restoreClipboardAfterPaste: Bool
    var autoSubmitAfterPaste: Bool
    var autoSubmitKey: AutoSubmitKey
    var historyLimit: Int
    var recordingRetentionPeriod: RecordingRetentionPeriod
    var debugMode: Bool
    var logLevel: NativeLogLevel
    var experimentalEnabled: Bool
    var lazyStreamClose: Bool
    var keyboardImplementation: KeyboardImplementationSetting
    var whisperAccelerator: WhisperAcceleratorSetting
    var ortAccelerator: OrtAcceleratorSetting
    var whisperGPUDevice: Int
    var postProcessEnabled: Bool
    var postProcessProviderID: String
    var postProcessProviders: [PostProcessProvider]
    var postProcessModels: [String: String]
    var postProcessPrompts: [PostProcessPrompt]
    var postProcessSelectedPromptID: String?
    var nativeOnboardingCompleted: Bool

    static let defaults = AppSettings(
        shortcutBindings: ShortcutBinding.defaults,
        pushToTalk: true,
        audioFeedback: false,
        audioFeedbackVolume: 0.25,
        soundTheme: .marimba,
        selectedMicrophoneName: nil,
        clamshellMicrophoneName: nil,
        selectedOutputDeviceName: nil,
        alwaysOnMicrophone: false,
        muteWhileRecording: false,
        startHidden: false,
        autostartEnabled: false,
        updateChecksEnabled: false,
        showMenuBarIcon: true,
        showOverlay: true,
        overlayPosition: .bottom,
        appTheme: .pink,
        appLanguage: AppSettings.defaultAppLanguage(),
        modelUnloadTimeout: .min5,
        selectedModel: TranscriptionAPIProvider.mistralVoxtralModelID,
        transcriptionAPIProviderID: TranscriptionAPIProvider.mistralProviderID,
        transcriptionAPIProviders: TranscriptionAPIProvider.defaults,
        transcriptionAPIModels: TranscriptionAPIModel.defaults,
        selectedLanguage: "auto",
        translateToEnglish: false,
        customWords: [],
        customFillerWords: nil,
        wordCorrectionThreshold: 0.18,
        pasteMethod: .commandV,
        pasteDelayMilliseconds: 60,
        extraRecordingBufferMilliseconds: 0,
        appendTrailingSpace: false,
        clipboardHandling: .dontModify,
        restoreClipboardAfterPaste: true,
        autoSubmitAfterPaste: false,
        autoSubmitKey: .enter,
        historyLimit: 5,
        recordingRetentionPeriod: .preserveLimit,
        debugMode: false,
        logLevel: .debug,
        experimentalEnabled: false,
        lazyStreamClose: false,
        keyboardImplementation: .nativeEventTap,
        whisperAccelerator: .auto,
        ortAccelerator: .auto,
        whisperGPUDevice: -1,
        postProcessEnabled: false,
        postProcessProviderID: PostProcessProvider.mistralProviderID,
        postProcessProviders: PostProcessProvider.defaults,
        postProcessModels: PostProcessProvider.defaultModels,
        postProcessPrompts: [.defaultImproveTranscriptions],
        postProcessSelectedPromptID: nil,
        nativeOnboardingCompleted: false
    )

    init(
        shortcutBindings: [String: ShortcutBinding],
        pushToTalk: Bool,
        audioFeedback: Bool,
        audioFeedbackVolume: Double,
        soundTheme: AudioFeedbackTheme,
        selectedMicrophoneName: String?,
        clamshellMicrophoneName: String?,
        selectedOutputDeviceName: String?,
        alwaysOnMicrophone: Bool,
        muteWhileRecording: Bool,
        startHidden: Bool,
        autostartEnabled: Bool,
        updateChecksEnabled: Bool,
        showMenuBarIcon: Bool,
        showOverlay: Bool,
        overlayPosition: OverlayPosition,
        appTheme: AppTheme,
        appLanguage: String,
        modelUnloadTimeout: ModelUnloadTimeout,
        selectedModel: String,
        transcriptionAPIProviderID: String,
        transcriptionAPIProviders: [TranscriptionAPIProvider],
        transcriptionAPIModels: [TranscriptionAPIModel],
        selectedLanguage: String,
        translateToEnglish: Bool,
        customWords: [String],
        customFillerWords: [String]?,
        wordCorrectionThreshold: Double,
        pasteMethod: PasteMethod,
        pasteDelayMilliseconds: Int,
        extraRecordingBufferMilliseconds: Int,
        appendTrailingSpace: Bool,
        clipboardHandling: ClipboardHandling,
        restoreClipboardAfterPaste: Bool,
        autoSubmitAfterPaste: Bool,
        autoSubmitKey: AutoSubmitKey,
        historyLimit: Int,
        recordingRetentionPeriod: RecordingRetentionPeriod,
        debugMode: Bool,
        logLevel: NativeLogLevel,
        experimentalEnabled: Bool,
        lazyStreamClose: Bool,
        keyboardImplementation: KeyboardImplementationSetting,
        whisperAccelerator: WhisperAcceleratorSetting,
        ortAccelerator: OrtAcceleratorSetting,
        whisperGPUDevice: Int,
        postProcessEnabled: Bool,
        postProcessProviderID: String,
        postProcessProviders: [PostProcessProvider],
        postProcessModels: [String: String],
        postProcessPrompts: [PostProcessPrompt],
        postProcessSelectedPromptID: String?,
        nativeOnboardingCompleted: Bool
    ) {
        self.shortcutBindings = shortcutBindings.mergedWithShortcutDefaults
        self.pushToTalk = pushToTalk
        self.audioFeedback = audioFeedback
        self.audioFeedbackVolume = audioFeedbackVolume
        self.soundTheme = soundTheme
        self.selectedMicrophoneName = selectedMicrophoneName
        self.clamshellMicrophoneName = clamshellMicrophoneName
        self.selectedOutputDeviceName = selectedOutputDeviceName
        self.alwaysOnMicrophone = alwaysOnMicrophone
        self.muteWhileRecording = muteWhileRecording
        self.startHidden = startHidden
        self.autostartEnabled = autostartEnabled
        self.updateChecksEnabled = false
        self.showMenuBarIcon = showMenuBarIcon
        self.overlayPosition = showOverlay ? overlayPosition : .none
        self.showOverlay = self.overlayPosition != .none
        self.appTheme = appTheme
        self.appLanguage = Self.normalizedAppLanguage(appLanguage)
        self.modelUnloadTimeout = modelUnloadTimeout
        self.selectedModel = selectedModel
        self.transcriptionAPIProviderID = transcriptionAPIProviderID
        self.transcriptionAPIProviders = transcriptionAPIProviders
        self.transcriptionAPIModels = transcriptionAPIModels
        self.selectedLanguage = selectedLanguage
        self.translateToEnglish = translateToEnglish
        self.customWords = Self.normalizedCustomWordsForImport(customWords)
        self.customFillerWords = customFillerWords.map(Self.normalizedCustomFillerWordsForImport)
        self.wordCorrectionThreshold = Self.clampedWordCorrectionThreshold(wordCorrectionThreshold)
        self.pasteMethod = pasteMethod.macOSCompatible
        self.pasteDelayMilliseconds = pasteDelayMilliseconds
        self.extraRecordingBufferMilliseconds = Self.clampedExtraRecordingBufferMilliseconds(extraRecordingBufferMilliseconds)
        self.appendTrailingSpace = appendTrailingSpace
        self.clipboardHandling = clipboardHandling
        self.restoreClipboardAfterPaste = restoreClipboardAfterPaste && clipboardHandling == .dontModify
        self.autoSubmitAfterPaste = autoSubmitAfterPaste
        self.autoSubmitKey = autoSubmitKey
        self.historyLimit = historyLimit
        self.recordingRetentionPeriod = recordingRetentionPeriod
        self.debugMode = debugMode
        self.logLevel = logLevel
        self.experimentalEnabled = experimentalEnabled
        self.lazyStreamClose = lazyStreamClose
        self.keyboardImplementation = keyboardImplementation
        self.whisperAccelerator = whisperAccelerator
        self.ortAccelerator = ortAccelerator
        self.whisperGPUDevice = max(-1, whisperGPUDevice)
        self.postProcessEnabled = postProcessEnabled
        self.postProcessProviderID = postProcessProviderID
        self.postProcessProviders = postProcessProviders
        self.postProcessModels = postProcessModels
        self.postProcessPrompts = postProcessPrompts
        self.postProcessSelectedPromptID = postProcessSelectedPromptID
        self.nativeOnboardingCompleted = nativeOnboardingCompleted
        ensureTranscriptionAPIDefaults()
        ensurePostProcessDefaults()
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let defaults = AppSettings.defaults

        shortcutBindings = (try container.decodeIfPresent([String: ShortcutBinding].self, forKey: .shortcutBindings) ?? defaults.shortcutBindings).mergedWithShortcutDefaults
        pushToTalk = try container.decodeIfPresent(Bool.self, forKey: .pushToTalk) ?? defaults.pushToTalk
        audioFeedback = try container.decodeIfPresent(Bool.self, forKey: .audioFeedback) ?? defaults.audioFeedback
        audioFeedbackVolume = try container.decodeIfPresent(Double.self, forKey: .audioFeedbackVolume) ?? defaults.audioFeedbackVolume
        soundTheme = try container.decodeIfPresent(AudioFeedbackTheme.self, forKey: .soundTheme) ?? defaults.soundTheme
        selectedMicrophoneName = try container.decodeIfPresent(String.self, forKey: .selectedMicrophoneName) ?? defaults.selectedMicrophoneName
        clamshellMicrophoneName = try container.decodeIfPresent(String.self, forKey: .clamshellMicrophoneName) ?? defaults.clamshellMicrophoneName
        selectedOutputDeviceName = try container.decodeIfPresent(String.self, forKey: .selectedOutputDeviceName) ?? defaults.selectedOutputDeviceName
        alwaysOnMicrophone = try container.decodeIfPresent(Bool.self, forKey: .alwaysOnMicrophone) ?? defaults.alwaysOnMicrophone
        muteWhileRecording = try container.decodeIfPresent(Bool.self, forKey: .muteWhileRecording) ?? defaults.muteWhileRecording
        startHidden = try container.decodeIfPresent(Bool.self, forKey: .startHidden) ?? defaults.startHidden
        autostartEnabled = try container.decodeIfPresent(Bool.self, forKey: .autostartEnabled) ?? defaults.autostartEnabled
        _ = try container.decodeIfPresent(Bool.self, forKey: .updateChecksEnabled)
        updateChecksEnabled = false
        showMenuBarIcon = try container.decodeIfPresent(Bool.self, forKey: .showMenuBarIcon) ?? defaults.showMenuBarIcon
        if let decodedOverlayPosition = try container.decodeIfPresent(OverlayPosition.self, forKey: .overlayPosition) {
            overlayPosition = decodedOverlayPosition
        } else if let decodedShowOverlay = try container.decodeIfPresent(Bool.self, forKey: .showOverlay) {
            overlayPosition = decodedShowOverlay ? defaults.overlayPosition : .none
        } else {
            overlayPosition = defaults.overlayPosition
        }
        showOverlay = overlayPosition != .none
        appTheme = try container.decodeIfPresent(AppTheme.self, forKey: .appTheme) ?? defaults.appTheme
        appLanguage = Self.normalizedAppLanguage(try container.decodeIfPresent(String.self, forKey: .appLanguage) ?? defaults.appLanguage)
        modelUnloadTimeout = try container.decodeIfPresent(ModelUnloadTimeout.self, forKey: .modelUnloadTimeout) ?? defaults.modelUnloadTimeout
        selectedModel = try container.decodeIfPresent(String.self, forKey: .selectedModel) ?? defaults.selectedModel
        transcriptionAPIProviderID = try container.decodeIfPresent(String.self, forKey: .transcriptionAPIProviderID) ?? defaults.transcriptionAPIProviderID
        transcriptionAPIProviders = try container.decodeIfPresent([TranscriptionAPIProvider].self, forKey: .transcriptionAPIProviders) ?? defaults.transcriptionAPIProviders
        transcriptionAPIModels = try container.decodeIfPresent([TranscriptionAPIModel].self, forKey: .transcriptionAPIModels) ?? defaults.transcriptionAPIModels
        selectedLanguage = try container.decodeIfPresent(String.self, forKey: .selectedLanguage) ?? defaults.selectedLanguage
        translateToEnglish = try container.decodeIfPresent(Bool.self, forKey: .translateToEnglish) ?? defaults.translateToEnglish
        customWords = Self.normalizedCustomWordsForImport(try container.decodeIfPresent([String].self, forKey: .customWords) ?? defaults.customWords)
        if container.contains(.customFillerWords) {
            customFillerWords = try container.decodeIfPresent([String].self, forKey: .customFillerWords).map(Self.normalizedCustomFillerWordsForImport)
        } else {
            customFillerWords = defaults.customFillerWords
        }
        wordCorrectionThreshold = Self.clampedWordCorrectionThreshold(
            try container.decodeIfPresent(Double.self, forKey: .wordCorrectionThreshold) ?? defaults.wordCorrectionThreshold
        )
        pasteMethod = (try container.decodeIfPresent(PasteMethod.self, forKey: .pasteMethod) ?? defaults.pasteMethod).macOSCompatible
        pasteDelayMilliseconds = try container.decodeIfPresent(Int.self, forKey: .pasteDelayMilliseconds) ?? defaults.pasteDelayMilliseconds
        extraRecordingBufferMilliseconds = Self.clampedExtraRecordingBufferMilliseconds(
            try container.decodeIfPresent(Int.self, forKey: .extraRecordingBufferMilliseconds) ?? defaults.extraRecordingBufferMilliseconds
        )
        appendTrailingSpace = try container.decodeIfPresent(Bool.self, forKey: .appendTrailingSpace) ?? defaults.appendTrailingSpace
        if let decodedClipboardHandling = try container.decodeIfPresent(ClipboardHandling.self, forKey: .clipboardHandling) {
            clipboardHandling = decodedClipboardHandling
        } else if let restoreClipboard = try container.decodeIfPresent(Bool.self, forKey: .restoreClipboardAfterPaste) {
            clipboardHandling = restoreClipboard ? .dontModify : .copyToClipboard
        } else {
            clipboardHandling = defaults.clipboardHandling
        }
        restoreClipboardAfterPaste = clipboardHandling == .dontModify
        autoSubmitAfterPaste = try container.decodeIfPresent(Bool.self, forKey: .autoSubmitAfterPaste) ?? defaults.autoSubmitAfterPaste
        autoSubmitKey = try container.decodeIfPresent(AutoSubmitKey.self, forKey: .autoSubmitKey) ?? defaults.autoSubmitKey
        historyLimit = try container.decodeIfPresent(Int.self, forKey: .historyLimit) ?? defaults.historyLimit
        recordingRetentionPeriod = try container.decodeIfPresent(RecordingRetentionPeriod.self, forKey: .recordingRetentionPeriod) ?? defaults.recordingRetentionPeriod
        debugMode = try container.decodeIfPresent(Bool.self, forKey: .debugMode) ?? defaults.debugMode
        logLevel = try container.decodeIfPresent(NativeLogLevel.self, forKey: .logLevel) ?? defaults.logLevel
        experimentalEnabled = try container.decodeIfPresent(Bool.self, forKey: .experimentalEnabled) ?? defaults.experimentalEnabled
        lazyStreamClose = try container.decodeIfPresent(Bool.self, forKey: .lazyStreamClose) ?? defaults.lazyStreamClose
        keyboardImplementation = try container.decodeIfPresent(KeyboardImplementationSetting.self, forKey: .keyboardImplementation) ?? defaults.keyboardImplementation
        whisperAccelerator = try container.decodeIfPresent(WhisperAcceleratorSetting.self, forKey: .whisperAccelerator) ?? defaults.whisperAccelerator
        ortAccelerator = try container.decodeIfPresent(OrtAcceleratorSetting.self, forKey: .ortAccelerator) ?? defaults.ortAccelerator
        whisperGPUDevice = max(-1, try container.decodeIfPresent(Int.self, forKey: .whisperGPUDevice) ?? defaults.whisperGPUDevice)
        postProcessEnabled = try container.decodeIfPresent(Bool.self, forKey: .postProcessEnabled) ?? defaults.postProcessEnabled
        postProcessProviderID = try container.decodeIfPresent(String.self, forKey: .postProcessProviderID) ?? defaults.postProcessProviderID
        postProcessProviders = try container.decodeIfPresent([PostProcessProvider].self, forKey: .postProcessProviders) ?? defaults.postProcessProviders
        postProcessModels = try container.decodeIfPresent([String: String].self, forKey: .postProcessModels) ?? defaults.postProcessModels
        postProcessPrompts = try container.decodeIfPresent([PostProcessPrompt].self, forKey: .postProcessPrompts) ?? defaults.postProcessPrompts
        postProcessSelectedPromptID = try container.decodeIfPresent(String.self, forKey: .postProcessSelectedPromptID) ?? defaults.postProcessSelectedPromptID
        nativeOnboardingCompleted = try container.decodeIfPresent(Bool.self, forKey: .nativeOnboardingCompleted) ?? defaults.nativeOnboardingCompleted
        ensureTranscriptionAPIDefaults()
        ensurePostProcessDefaults()
    }

    @discardableResult
    mutating func addCustomWord(_ word: String) -> Bool {
        guard let sanitized = Self.sanitizeCustomWord(word),
              customWords.contains(sanitized) == false
        else {
            return false
        }

        customWords.append(sanitized)
        return true
    }

    mutating func removeCustomWord(_ word: String) {
        customWords.removeAll { $0 == word }
    }

    static func sanitizeCustomWord(_ word: String) -> String? {
        let sanitized = word
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .filter { "<>\"'&".contains($0) == false }

        guard sanitized.isEmpty == false,
              sanitized.contains(where: \.isWhitespace) == false,
              sanitized.count <= 50
        else {
            return nil
        }

        return sanitized
    }

    static func normalizedCustomWordsForImport(_ words: [String]) -> [String] {
        var seen = Set<String>()
        var normalized: [String] = []

        for word in words {
            let sanitized = word
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .filter { "<>\"'&".contains($0) == false }

            guard sanitized.isEmpty == false,
                  sanitized.count <= 50,
                  seen.contains(sanitized) == false
            else {
                continue
            }
            seen.insert(sanitized)
            normalized.append(sanitized)
        }

        return normalized
    }

    static func defaultAppLanguage() -> String {
        normalizedAppLanguage(Locale.current.identifier)
    }

    static func normalizedAppLanguage(_ language: String) -> String {
        let normalized = language
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "_", with: "-")
        return normalized.isEmpty ? "en" : normalized
    }

    static func normalizedCustomFillerWordsForImport(_ words: [String]) -> [String] {
        var seen = Set<String>()
        var normalized: [String] = []

        for word in words {
            let sanitized = word.trimmingCharacters(in: .whitespacesAndNewlines)
            guard sanitized.isEmpty == false,
                  seen.contains(sanitized) == false
            else {
                continue
            }
            seen.insert(sanitized)
            normalized.append(sanitized)
        }

        return normalized
    }

    static func clampedWordCorrectionThreshold(_ threshold: Double) -> Double {
        min(1, max(0, threshold))
    }

    static func clampedExtraRecordingBufferMilliseconds(_ milliseconds: Int) -> Int {
        min(1_500, max(0, milliseconds))
    }
}
