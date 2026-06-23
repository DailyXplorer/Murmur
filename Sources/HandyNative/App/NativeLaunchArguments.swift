import Foundation

struct NativeLaunchArguments: Equatable {
    var startHidden: Bool
    var noTray: Bool
    var debug: Bool
    var toggleTranscription: Bool
    var togglePostProcess: Bool
    var cancel: Bool
    var initialSection: AppSection?
    var smokeAudioRecordingRequest: NativeAudioRecordingSmokeRequest?
    var smokeRecordTranscriptionRequest: NativeRecordTranscriptionSmokeRequest?
    var smokePasteRequest: NativePasteSmokeRequest?
    var smokeTranscriptionRequest: NativeTranscriptionSmokeRequest?
    var smokeModelCacheRequest: NativeModelCacheSmokeRequest?
    var smokeModelRuntimeRequest: NativeModelRuntimeSmokeRequest?
    var smokePermissionStatusRequest: NativePermissionStatusSmokeRequest?
    var smokeReplacementReadinessRequest: NativeReplacementReadinessSmokeRequest?
    var smokeRemoteControlListenerRequest: NativeRemoteControlListenerSmokeRequest?
    var smokeRemoteControlSendRequest: NativeRemoteControlSendSmokeRequest?
    var smokeExternalPasteTargetRequest: NativeExternalPasteTargetSmokeRequest?
    var smokeExternalPasteRoundTripRequest: NativeExternalPasteRoundTripSmokeRequest?
    var smokeGlobalShortcutRequest: NativeGlobalShortcutSmokeRequest?
    var smokeGlobalShortcutRecordingRequest: NativeGlobalShortcutRecordingSmokeRequest?
    var smokeOverlayState: RecordingOverlayState?
    var smokeOverlayOutputPath: String?
    var smokeOverlayImageOutputPath: String?
    var smokeOnboardingStep: NativeOnboardingStep?

    static let none = NativeLaunchArguments(
        startHidden: false,
        noTray: false,
        debug: false,
        toggleTranscription: false,
        togglePostProcess: false,
        cancel: false,
        initialSection: nil,
        smokeAudioRecordingRequest: nil,
        smokeRecordTranscriptionRequest: nil,
        smokePasteRequest: nil,
        smokeTranscriptionRequest: nil,
        smokeModelCacheRequest: nil,
        smokeModelRuntimeRequest: nil,
        smokePermissionStatusRequest: nil,
        smokeReplacementReadinessRequest: nil,
        smokeRemoteControlListenerRequest: nil,
        smokeRemoteControlSendRequest: nil,
        smokeExternalPasteTargetRequest: nil,
        smokeExternalPasteRoundTripRequest: nil,
        smokeGlobalShortcutRequest: nil,
        smokeGlobalShortcutRecordingRequest: nil,
        smokeOverlayState: nil,
        smokeOverlayOutputPath: nil,
        smokeOverlayImageOutputPath: nil,
        smokeOnboardingStep: nil
    )

    static var current: NativeLaunchArguments {
        parse(ProcessInfo.processInfo.arguments)
    }

    static func parse(_ arguments: [String]) -> NativeLaunchArguments {
        let rawArguments = Array(arguments.dropFirst())
        let flags = Set(rawArguments.filter { !$0.contains("=") })
        let smokeTranscriptionUsesSelectedSettings = flags.contains("--smoke-transcribe-selected-settings")
        let smokeTranscriptionRequestsPostProcess = flags.contains("--smoke-post-process")
        let smokeRecordingPath = value(for: "--smoke-record-audio", in: rawArguments)
        let smokeRecordTranscriptionPath = value(for: "--smoke-record-transcribe", in: rawArguments)
        let smokePasteText = value(for: "--smoke-paste-text", in: rawArguments)
        let smokeFilePath = value(for: "--smoke-transcribe-file", in: rawArguments)
        let modelCacheStatusID = value(for: "--smoke-model-cache-status", in: rawArguments)
        let modelCacheDownloadID = value(for: "--smoke-download-model-cache", in: rawArguments)
        let modelCacheDeleteID = value(for: "--smoke-delete-model-cache", in: rawArguments)
        let modelRuntimeID = value(for: "--smoke-model-runtime-state", in: rawArguments) ??
            (flags.contains("--smoke-model-runtime-state") ? "tiny" : nil)
        let smokePermissionStatus = flags.contains("--smoke-permission-status") ||
            value(for: "--smoke-permission-status", in: rawArguments) != nil
        let smokeReplacementReadiness = flags.contains("--smoke-replacement-readiness") ||
            value(for: "--smoke-replacement-readiness", in: rawArguments) != nil
        let parsedSmokeRemoteControlCommand = remoteControlCommand(
            from: value(for: "--smoke-remote-control-command", in: rawArguments)
        )
        let smokeRemoteControlLaunchMethod: NativeRemoteControlSmokeLaunchMethod =
            flags.contains("--smoke-remote-control-launchservices") ? .launchServices : .executable
        let externalPasteTargetPath = value(for: "--smoke-external-paste-target", in: rawArguments)
        let externalPasteRoundTripText = value(for: "--smoke-external-paste-roundtrip", in: rawArguments) ??
            (flags.contains("--smoke-external-paste-roundtrip") ? smokePasteText : nil)
        let initialSection = value(for: "--open-section", in: rawArguments).flatMap(AppSection.section(forLaunchArgument:))
        let smokeOverlayState = value(for: "--smoke-overlay-state", in: rawArguments).flatMap(RecordingOverlayState.init(smokeArgument:))
        let smokeOnboardingStep = value(for: "--smoke-onboarding-step", in: rawArguments).flatMap(NativeOnboardingStep.init(smokeArgument:))
        let smokePasteAfterTranscriptionRequest = smokePasteAfterTranscriptionRequest(
            flags: flags,
            rawArguments: rawArguments
        )
        return NativeLaunchArguments(
            startHidden: flags.contains("--start-hidden"),
            noTray: flags.contains("--no-tray"),
            debug: flags.contains("--debug"),
            toggleTranscription: flags.contains("--toggle-transcription"),
            togglePostProcess: flags.contains("--toggle-post-process"),
            cancel: flags.contains("--cancel"),
            initialSection: initialSection,
            smokeAudioRecordingRequest: smokeRecordingPath.map {
                NativeAudioRecordingSmokeRequest(
                    outputPath: $0,
                    durationMilliseconds: durationMilliseconds(
                        from: value(for: "--smoke-record-duration-ms", in: rawArguments)
                    ),
                    microphoneName: value(for: "--smoke-record-microphone", in: rawArguments)
                )
            },
            smokeRecordTranscriptionRequest: smokeRecordTranscriptionPath.map {
                NativeRecordTranscriptionSmokeRequest(
                    outputPath: $0,
                    durationMilliseconds: durationMilliseconds(
                        from: value(for: "--smoke-record-duration-ms", in: rawArguments)
                    ),
                    microphoneName: value(for: "--smoke-record-microphone", in: rawArguments),
                    modelID: value(for: "--smoke-transcribe-model", in: rawArguments) ?? "tiny",
                    language: value(for: "--smoke-transcribe-language", in: rawArguments),
                    useSelectedSettings: smokeTranscriptionUsesSelectedSettings,
                    postProcessRequested: smokeTranscriptionRequestsPostProcess,
                    recordHistory: flags.contains("--smoke-record-history"),
                    pasteRequest: smokePasteAfterTranscriptionRequest,
                    outputJSONPath: value(for: "--smoke-output-json", in: rawArguments)
                )
            },
            smokePasteRequest: externalPasteRoundTripText == nil ? smokePasteText.map {
                NativePasteSmokeRequest(
                    text: $0,
                    pasteMethod: pasteMethod(from: value(for: "--smoke-paste-method", in: rawArguments)),
                    clipboardHandling: clipboardHandling(
                        from: value(for: "--smoke-clipboard-handling", in: rawArguments)
                    ),
                    pasteDelayMilliseconds: pasteDelayMilliseconds(
                        from: value(for: "--smoke-paste-delay-ms", in: rawArguments)
                    ),
                    startDelayMilliseconds: smokeDelayMilliseconds(
                        from: value(for: "--smoke-paste-start-delay-ms", in: rawArguments)
                    ),
                    appendTrailingSpace: flags.contains("--smoke-append-trailing-space"),
                    autoSubmitKey: autoSubmitKey(from: value(for: "--smoke-auto-submit", in: rawArguments)),
                    targetWindow: flags.contains("--smoke-paste-target-window"),
                    activationProcessIdentifier: processIdentifier(
                        from: value(for: "--smoke-paste-activate-pid", in: rawArguments)
                    ),
                    outputPath: value(for: "--smoke-output-json", in: rawArguments)
                )
            } : nil,
            smokeTranscriptionRequest: smokeFilePath.map {
                NativeTranscriptionSmokeRequest(
                    filePath: $0,
                    modelID: value(for: "--smoke-transcribe-model", in: rawArguments) ?? "tiny",
                    language: value(for: "--smoke-transcribe-language", in: rawArguments),
                    useSelectedSettings: smokeTranscriptionUsesSelectedSettings,
                    postProcessRequested: smokeTranscriptionRequestsPostProcess,
                    recordHistory: flags.contains("--smoke-record-history"),
                    pasteRequest: smokePasteAfterTranscriptionRequest,
                    outputPath: value(for: "--smoke-output-json", in: rawArguments)
                )
            },
            smokeModelCacheRequest: modelCacheDownloadID.map {
                NativeModelCacheSmokeRequest(modelID: $0, operation: .download)
            } ?? modelCacheDeleteID.map {
                NativeModelCacheSmokeRequest(modelID: $0, operation: .delete)
            } ?? modelCacheStatusID.map {
                NativeModelCacheSmokeRequest(modelID: $0, operation: .status)
            },
            smokeModelRuntimeRequest: modelRuntimeID.map {
                NativeModelRuntimeSmokeRequest(
                    modelID: $0,
                    unloadTimeout: modelUnloadTimeout(
                        from: value(for: "--smoke-model-runtime-unload-timeout", in: rawArguments)
                    ),
                    waitMilliseconds: modelRuntimeWaitMilliseconds(
                        from: value(for: "--smoke-model-runtime-wait-ms", in: rawArguments)
                    ),
                    explicitUnload: flags.contains("--smoke-model-runtime-explicit-unload"),
                    outputPath: value(for: "--smoke-output-json", in: rawArguments)
                )
            },
            smokePermissionStatusRequest: smokePermissionStatus
                ? NativePermissionStatusSmokeRequest(
                    outputPath: value(for: "--smoke-permission-status", in: rawArguments) ??
                        value(for: "--smoke-output-json", in: rawArguments)
                )
                : nil,
            smokeReplacementReadinessRequest: smokeReplacementReadiness
                ? NativeReplacementReadinessSmokeRequest(
                    outputPath: value(for: "--smoke-replacement-readiness", in: rawArguments) ??
                        value(for: "--smoke-output-json", in: rawArguments),
                    strict: flags.contains("--smoke-replacement-readiness-strict")
                )
                : nil,
            smokeRemoteControlListenerRequest: flags.contains("--smoke-remote-control-listener")
                ? NativeRemoteControlListenerSmokeRequest(
                    command: parsedSmokeRemoteControlCommand,
                    timeoutMilliseconds: remoteControlTimeoutMilliseconds(
                        from: value(for: "--smoke-remote-control-timeout-ms", in: rawArguments)
                    ),
                    senderLaunchMethod: smokeRemoteControlLaunchMethod,
                    outputPath: value(for: "--smoke-output-json", in: rawArguments)
                )
                : nil,
            smokeRemoteControlSendRequest: flags.contains("--smoke-remote-control-send") ||
                value(for: "--smoke-remote-control-send", in: rawArguments) != nil
                ? NativeRemoteControlSendSmokeRequest(
                    command: value(for: "--smoke-remote-control-send", in: rawArguments)
                        .map(remoteControlCommand(from:)) ?? parsedSmokeRemoteControlCommand,
                    outputPath: value(for: "--smoke-output-json", in: rawArguments)
                )
                : nil,
            smokeExternalPasteTargetRequest: externalPasteTargetPath.map {
                NativeExternalPasteTargetSmokeRequest(
                    outputPath: $0,
                    readyPath: value(for: "--smoke-external-paste-ready", in: rawArguments),
                    expectedText: value(for: "--smoke-external-paste-expected", in: rawArguments),
                    durationMilliseconds: externalTargetDurationMilliseconds(
                        from: value(for: "--smoke-external-paste-duration-ms", in: rawArguments)
                    )
                )
            },
            smokeExternalPasteRoundTripRequest: externalPasteRoundTripText.map {
                NativeExternalPasteRoundTripSmokeRequest(
                    text: $0,
                    pasteMethod: pasteMethod(from: value(for: "--smoke-paste-method", in: rawArguments)),
                    clipboardHandling: clipboardHandling(
                        from: value(for: "--smoke-clipboard-handling", in: rawArguments)
                    ),
                    pasteDelayMilliseconds: pasteDelayMilliseconds(
                        from: value(for: "--smoke-paste-delay-ms", in: rawArguments)
                    ),
                    startDelayMilliseconds: smokeDelayMilliseconds(
                        from: value(for: "--smoke-paste-start-delay-ms", in: rawArguments)
                    ),
                    appendTrailingSpace: flags.contains("--smoke-append-trailing-space"),
                    autoSubmitKey: autoSubmitKey(from: value(for: "--smoke-auto-submit", in: rawArguments)),
                    durationMilliseconds: externalTargetDurationMilliseconds(
                        from: value(for: "--smoke-external-paste-duration-ms", in: rawArguments)
                    ),
                    outputPath: value(for: "--smoke-output-json", in: rawArguments)
                )
            },
            smokeGlobalShortcutRequest: flags.contains("--smoke-global-shortcut-event-tap")
                ? NativeGlobalShortcutSmokeRequest(
                    bindingID: value(for: "--smoke-global-shortcut-id", in: rawArguments) ??
                        ShortcutBinding.transcribeID,
                    binding: value(for: "--smoke-global-shortcut-binding", in: rawArguments) ??
                        ShortcutBinding.defaults[ShortcutBinding.transcribeID]?.currentBinding ??
                        "option+space",
                    outputPath: value(for: "--smoke-output-json", in: rawArguments)
                )
                : nil,
            smokeGlobalShortcutRecordingRequest: flags.contains("--smoke-global-shortcut-recording")
                ? NativeGlobalShortcutRecordingSmokeRequest(
                    bindingID: value(for: "--smoke-global-shortcut-id", in: rawArguments) ??
                        ShortcutBinding.transcribeID,
                    binding: value(for: "--smoke-global-shortcut-binding", in: rawArguments) ??
                        ShortcutBinding.defaults[ShortcutBinding.transcribeID]?.currentBinding ??
                        "option+space",
                    durationMilliseconds: durationMilliseconds(
                        from: value(for: "--smoke-record-duration-ms", in: rawArguments)
                    ),
                    microphoneName: value(for: "--smoke-record-microphone", in: rawArguments),
                    recordingOutputPath: value(for: "--smoke-global-shortcut-recording-output", in: rawArguments),
                    transcribeAfterRecording: flags.contains("--smoke-transcribe-after-shortcut-recording"),
                    modelID: value(for: "--smoke-transcribe-model", in: rawArguments) ?? "tiny",
                    language: value(for: "--smoke-transcribe-language", in: rawArguments),
                    useSelectedSettings: smokeTranscriptionUsesSelectedSettings,
                    postProcessRequested: smokeTranscriptionRequestsPostProcess,
                    recordHistory: flags.contains("--smoke-record-history"),
                    pasteRequest: smokePasteAfterTranscriptionRequest,
                    outputPath: value(for: "--smoke-output-json", in: rawArguments)
                )
                : nil,
            smokeOverlayState: smokeOverlayState,
            smokeOverlayOutputPath: smokeOverlayState == nil ? nil : value(for: "--smoke-output-json", in: rawArguments),
            smokeOverlayImageOutputPath: smokeOverlayState == nil ? nil : value(for: "--smoke-output-image", in: rawArguments),
            smokeOnboardingStep: smokeOnboardingStep
        )
    }

    private static func value(for flag: String, in arguments: [String]) -> String? {
        for (index, argument) in arguments.enumerated() {
            if argument == flag,
               arguments.indices.contains(index + 1),
               !arguments[index + 1].hasPrefix("--") {
                return arguments[index + 1]
            }

            let prefix = "\(flag)="
            if argument.hasPrefix(prefix) {
                return String(argument.dropFirst(prefix.count))
            }
        }

        return nil
    }

    private static func durationMilliseconds(from rawValue: String?) -> Int {
        guard let rawValue,
              let parsed = Int(rawValue.trimmingCharacters(in: .whitespacesAndNewlines))
        else {
            return 1_000
        }

        return min(max(parsed, 100), 10_000)
    }

    private static func pasteDelayMilliseconds(from rawValue: String?) -> Int {
        guard let rawValue,
              let parsed = Int(rawValue.trimmingCharacters(in: .whitespacesAndNewlines))
        else {
            return AppSettings.defaults.pasteDelayMilliseconds
        }

        return min(max(parsed, 0), 2_000)
    }

    private static func smokeDelayMilliseconds(from rawValue: String?) -> Int {
        guard let rawValue,
              let parsed = Int(rawValue.trimmingCharacters(in: .whitespacesAndNewlines))
        else {
            return 500
        }

        return min(max(parsed, 0), 10_000)
    }

    private static func externalTargetDurationMilliseconds(from rawValue: String?) -> Int {
        guard let rawValue,
              let parsed = Int(rawValue.trimmingCharacters(in: .whitespacesAndNewlines))
        else {
            return 5_000
        }

        return min(max(parsed, 500), 30_000)
    }

    private static func modelRuntimeWaitMilliseconds(from rawValue: String?) -> Int {
        guard let rawValue,
              let parsed = Int(rawValue.trimmingCharacters(in: .whitespacesAndNewlines))
        else {
            return 0
        }

        return min(max(parsed, 0), 30_000)
    }

    private static func remoteControlTimeoutMilliseconds(from rawValue: String?) -> Int {
        guard let rawValue,
              let parsed = Int(rawValue.trimmingCharacters(in: .whitespacesAndNewlines))
        else {
            return 5_000
        }

        return min(max(parsed, 500), 15_000)
    }

    private static func remoteControlCommand(from rawValue: String?) -> RemoteControlCommand {
        guard let rawValue else {
            return .toggleTranscription
        }

        switch normalizedLaunchValue(rawValue) {
        case "toggletranscription", "transcribe":
            return .toggleTranscription
        case "togglepostprocess", "postprocess", "transcribewithpostprocess":
            return .togglePostProcess
        case "cancel":
            return .cancel
        default:
            return RemoteControlCommand(rawValue: rawValue) ?? .toggleTranscription
        }
    }

    private static func modelUnloadTimeout(from rawValue: String?) -> ModelUnloadTimeout {
        guard let rawValue else {
            return .never
        }

        switch normalizedLaunchValue(rawValue) {
        case "never":
            return .never
        case "immediately", "immediate", "now":
            return .immediately
        case "2min", "2mins", "2minutes", "min2":
            return .min2
        case "5min", "5mins", "5minutes", "min5":
            return .min5
        case "10min", "10mins", "10minutes", "min10":
            return .min10
        case "15min", "15mins", "15minutes", "min15":
            return .min15
        case "1h", "1hour", "hour1":
            return .hour1
        case "15s", "15sec", "15secs", "15seconds", "sec15":
            return .sec15
        default:
            return ModelUnloadTimeout(rawValue: rawValue) ?? .never
        }
    }

    private static func smokePasteAfterTranscriptionRequest(
        flags: Set<String>,
        rawArguments: [String]
    ) -> NativeTranscriptionPasteSmokeRequest? {
        let externalRoundTrip = flags.contains("--smoke-external-paste-after-transcribe")
        guard flags.contains("--smoke-paste-after-transcribe") || externalRoundTrip else {
            return nil
        }

        return NativeTranscriptionPasteSmokeRequest(
            pasteMethod: pasteMethod(from: value(for: "--smoke-paste-method", in: rawArguments)),
            clipboardHandling: clipboardHandling(
                from: value(for: "--smoke-clipboard-handling", in: rawArguments)
            ),
            pasteDelayMilliseconds: pasteDelayMilliseconds(
                from: value(for: "--smoke-paste-delay-ms", in: rawArguments)
            ),
            startDelayMilliseconds: smokeDelayMilliseconds(
                from: value(for: "--smoke-paste-start-delay-ms", in: rawArguments)
            ),
            appendTrailingSpace: flags.contains("--smoke-append-trailing-space"),
            autoSubmitKey: autoSubmitKey(from: value(for: "--smoke-auto-submit", in: rawArguments)),
            targetWindow: flags.contains("--smoke-paste-target-window"),
            externalRoundTrip: externalRoundTrip,
            externalRoundTripDurationMilliseconds: externalTargetDurationMilliseconds(
                from: value(for: "--smoke-external-paste-duration-ms", in: rawArguments)
            )
        )
    }

    private static func pasteMethod(from rawValue: String?) -> PasteMethod {
        guard let rawValue else {
            return AppSettings.defaults.pasteMethod
        }

        switch normalizedLaunchValue(rawValue) {
        case "ctrlv", "cmdv", "commandv":
            return .commandV
        case "ctrlshiftv", "cmdshiftv", "commandshiftv":
            return .commandShiftV
        case "direct":
            return .direct
        case "none":
            return .none
        case "shiftinsert":
            return .shiftInsert
        case "externalscript":
            return .externalScript
        default:
            return PasteMethod(rawValue: rawValue) ?? AppSettings.defaults.pasteMethod
        }
    }

    private static func clipboardHandling(from rawValue: String?) -> ClipboardHandling {
        guard let rawValue else {
            return AppSettings.defaults.clipboardHandling
        }

        switch normalizedLaunchValue(rawValue) {
        case "dontmodify", "restore":
            return .dontModify
        case "copytoclipboard", "copy":
            return .copyToClipboard
        default:
            return ClipboardHandling(rawValue: rawValue) ?? AppSettings.defaults.clipboardHandling
        }
    }

    private static func autoSubmitKey(from rawValue: String?) -> AutoSubmitKey? {
        guard let rawValue else {
            return nil
        }

        switch normalizedLaunchValue(rawValue) {
        case "enter", "return":
            return .enter
        case "ctrlenter", "controlenter":
            return .controlEnter
        case "cmdenter", "commandenter":
            return .commandEnter
        default:
            return AutoSubmitKey(rawValue: rawValue)
        }
    }

    private static func processIdentifier(from rawValue: String?) -> Int32? {
        guard let rawValue,
              let parsed = Int32(rawValue.trimmingCharacters(in: .whitespacesAndNewlines)),
              parsed > 0
        else {
            return nil
        }

        return parsed
    }

    private static func normalizedLaunchValue(_ rawValue: String) -> String {
        rawValue
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .replacingOccurrences(of: " ", with: "")
            .replacingOccurrences(of: "-", with: "")
            .replacingOccurrences(of: "_", with: "")
            .replacingOccurrences(of: "+", with: "")
    }

    var remoteCommand: RemoteControlCommand? {
        if toggleTranscription {
            return .toggleTranscription
        }
        if togglePostProcess {
            return .togglePostProcess
        }
        if cancel {
            return .cancel
        }
        return nil
    }
}

extension AppSection {
    static func section(forLaunchArgument rawValue: String) -> AppSection? {
        let normalized = rawValue
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .replacingOccurrences(of: " ", with: "")
            .replacingOccurrences(of: "-", with: "")
            .replacingOccurrences(of: "_", with: "")

        if normalized == "postprocess" {
            return .postProcessing
        }

        return allCases.first { section in
            section.rawValue.lowercased() == normalized ||
                section.title.lowercased().replacingOccurrences(of: " ", with: "") == normalized
        }
    }
}

extension RecordingOverlayState {
    init?(smokeArgument rawValue: String) {
        switch rawValue.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "recording":
            self = .recording
        case "transcribing":
            self = .transcribing
        case "processing":
            self = .processing
        default:
            return nil
        }
    }
}

extension NativeOnboardingStep {
    init?(smokeArgument rawValue: String) {
        switch rawValue.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "checking":
            self = .checking
        case "permissions":
            self = .permissions(returningUser: false)
        case "model":
            self = .model
        case "done":
            self = .done
        default:
            return nil
        }
    }
}

struct NativeAudioRecordingSmokeRequest: Equatable {
    var outputPath: String
    var durationMilliseconds: Int
    var microphoneName: String?
}

struct NativeRecordTranscriptionSmokeRequest: Equatable {
    var outputPath: String
    var durationMilliseconds: Int
    var microphoneName: String?
    var modelID: String
    var language: String?
    var useSelectedSettings = false
    var postProcessRequested = false
    var recordHistory: Bool
    var pasteRequest: NativeTranscriptionPasteSmokeRequest?
    var outputJSONPath: String?
}

struct NativePasteSmokeRequest: Equatable {
    var text: String
    var pasteMethod: PasteMethod
    var clipboardHandling: ClipboardHandling
    var pasteDelayMilliseconds: Int
    var startDelayMilliseconds: Int
    var appendTrailingSpace: Bool
    var autoSubmitKey: AutoSubmitKey?
    var targetWindow: Bool
    var activationProcessIdentifier: Int32?
    var outputPath: String?
}

struct NativeTranscriptionSmokeRequest: Equatable {
    var filePath: String
    var modelID: String
    var language: String?
    var useSelectedSettings = false
    var postProcessRequested = false
    var recordHistory: Bool
    var pasteRequest: NativeTranscriptionPasteSmokeRequest?
    var outputPath: String?
}

struct NativeTranscriptionPasteSmokeRequest: Equatable {
    var pasteMethod: PasteMethod
    var clipboardHandling: ClipboardHandling
    var pasteDelayMilliseconds: Int
    var startDelayMilliseconds: Int
    var appendTrailingSpace: Bool
    var autoSubmitKey: AutoSubmitKey?
    var targetWindow: Bool
    var externalRoundTrip = false
    var externalRoundTripDurationMilliseconds = 5_000
}

struct NativeModelCacheSmokeRequest: Equatable {
    enum Operation: String, Equatable {
        case status
        case download
        case delete
    }

    var modelID: String
    var operation: Operation
}

struct NativeModelRuntimeSmokeRequest: Equatable {
    var modelID: String
    var unloadTimeout: ModelUnloadTimeout
    var waitMilliseconds: Int
    var explicitUnload: Bool
    var outputPath: String?
}

struct NativeRemoteControlListenerSmokeRequest: Equatable {
    var command: RemoteControlCommand
    var timeoutMilliseconds: Int
    var senderLaunchMethod: NativeRemoteControlSmokeLaunchMethod = .executable
    var outputPath: String?
}

struct NativeRemoteControlSendSmokeRequest: Equatable {
    var command: RemoteControlCommand
    var outputPath: String?
}

enum NativeRemoteControlSmokeLaunchMethod: String, Codable, Equatable {
    case executable
    case launchServices = "launch-services"
}

struct NativeExternalPasteTargetSmokeRequest: Equatable {
    var outputPath: String
    var readyPath: String?
    var expectedText: String?
    var durationMilliseconds: Int
}

struct NativeExternalPasteRoundTripSmokeRequest: Equatable {
    var text: String
    var pasteMethod: PasteMethod
    var clipboardHandling: ClipboardHandling
    var pasteDelayMilliseconds: Int
    var startDelayMilliseconds: Int
    var appendTrailingSpace: Bool
    var autoSubmitKey: AutoSubmitKey? = nil
    var durationMilliseconds: Int
    var outputPath: String?
}

struct NativeGlobalShortcutSmokeRequest: Equatable {
    var bindingID: String
    var binding: String
    var outputPath: String?
}

struct NativeGlobalShortcutRecordingSmokeRequest: Equatable {
    var bindingID: String
    var binding: String
    var durationMilliseconds: Int
    var microphoneName: String?
    var recordingOutputPath: String?
    var transcribeAfterRecording = false
    var modelID = "tiny"
    var language: String? = nil
    var useSelectedSettings = false
    var postProcessRequested = false
    var recordHistory = false
    var pasteRequest: NativeTranscriptionPasteSmokeRequest? = nil
    var outputPath: String?
}
