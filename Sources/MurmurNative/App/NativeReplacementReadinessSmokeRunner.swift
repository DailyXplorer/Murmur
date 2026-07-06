import Foundation

enum NativeReplacementReadinessSmokeRunner {
    static func runSynchronouslyAndExit(_ request: NativeReplacementReadinessSmokeRequest) -> Never {
        do {
            let paths = try AppPaths.resolve()
            let credentialStore = LocalPostProcessCredentialStore(paths: paths)
            let settings = SettingsStore(paths: paths).load()
            let output = output(
                settings: settings,
                permissionSnapshot: PermissionService().snapshot(),
                hasAPIKey: { credentialStore.hasAPIKey(providerID: $0) }
            )
            try write(output: output, to: request.outputPath)
            if request.outputPath == nil {
                FileHandle.standardOutput.writeLine(try outputString(for: output))
            }
            exit(request.strict && !output.success ? 1 : 0)
        } catch {
            FileHandle.standardError.writeLine(error.localizedDescription)
            exit(1)
        }
    }

    static func output(
        settings: AppSettings,
        permissionSnapshot: PermissionSnapshot,
        hasAPIKey: (String) -> Bool
    ) -> NativeReplacementReadinessSmokeOutput {
        let modelAssessment = selectedModelAssessment(
            settings: settings,
            permissionSnapshot: permissionSnapshot,
            hasAPIKey: hasAPIKey
        )
        let liveDictationPrerequisitesReady = permissionSnapshot.accessibilityTrusted &&
            permissionSnapshot.microphone == .granted
        var blockingIssues: [String] = []
        var warnings: [String] = []

        if !liveDictationPrerequisitesReady {
            blockingIssues.append("Microphone and Accessibility permissions are required for shortcut-driven dictation.")
        }
        if !modelAssessment.runnable {
            blockingIssues.append(modelAssessment.blockingIssue)
        }
        if permissionSnapshot.speechRecognition != .granted {
            warnings.append("Apple Speech is available only after Speech Recognition permission is granted.")
        }

        let replacementReady = blockingIssues.isEmpty

        return NativeReplacementReadinessSmokeOutput(
            success: replacementReady,
            replacementReady: replacementReady,
            prompted: false,
            selectedModelID: settings.selectedModel,
            selectedModelDisplayName: settings.selectedTranscriptionModelDisplayName,
            selectedModelKind: modelAssessment.kind.rawValue,
            selectedModelRunnable: modelAssessment.runnable,
            selectedModelBlockingIssue: modelAssessment.runnable ? nil : modelAssessment.blockingIssue,
            accessibilityTrusted: permissionSnapshot.accessibilityTrusted,
            microphone: permissionSnapshot.microphone.rawValue,
            speechRecognition: permissionSnapshot.speechRecognition.rawValue,
            liveDictationPrerequisitesReady: liveDictationPrerequisitesReady,
            appleSpeechReady: permissionSnapshot.speechRecognition == .granted,
            nativeLocalModelIDs: LocalTranscriptionModel.catalog.map(\.id).sorted(),
            nativeTranscriptionPaths: [
                "api_transcription",
                "apple_speech",
                "whisperkit_coreml",
            ],
            blockingIssues: blockingIssues,
            warnings: warnings
        )
    }

    private static func selectedModelAssessment(
        settings: AppSettings,
        permissionSnapshot: PermissionSnapshot,
        hasAPIKey: (String) -> Bool
    ) -> SelectedModelAssessment {
        if settings.selectedLocalTranscriptionModel != nil {
            return SelectedModelAssessment(kind: .nativeLocalWhisper, runnable: true)
        }

        if let apiModel = settings.selectedTranscriptionAPIModel,
           let provider = settings.transcriptionAPIProviders.first(where: { $0.id == apiModel.providerID }) {
            if provider.requiresAPIKey && !hasAPIKey(provider.id) {
                return SelectedModelAssessment(
                    kind: .nativeAPI,
                    runnable: false,
                    blockingIssue: "Selected API transcription model requires an API key."
                )
            }
            return SelectedModelAssessment(kind: .nativeAPI, runnable: true)
        }

        if settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID {
            if permissionSnapshot.speechRecognition == .granted {
                return SelectedModelAssessment(kind: .nativeAppleSpeech, runnable: true)
            }
            return SelectedModelAssessment(
                kind: .nativeAppleSpeech,
                runnable: false,
                blockingIssue: "Selected Apple Speech model requires Speech Recognition permission."
            )
        }

        return SelectedModelAssessment(
            kind: .unknown,
            runnable: false,
            blockingIssue: "Selected model is unknown to the native Swift app."
        )
    }

    private static func write(output: NativeReplacementReadinessSmokeOutput, to path: String?) throws {
        guard let path else {
            return
        }

        let outputURL = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try outputString(for: output).write(to: outputURL, atomically: true, encoding: .utf8)
    }

    private static func outputString(for output: NativeReplacementReadinessSmokeOutput) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

struct NativeReplacementReadinessSmokeRequest: Equatable {
    var outputPath: String?
    var strict: Bool
}

struct NativeReplacementReadinessSmokeOutput: Encodable, Equatable {
    var success: Bool
    var replacementReady: Bool
    var prompted: Bool
    var selectedModelID: String
    var selectedModelDisplayName: String
    var selectedModelKind: String
    var selectedModelRunnable: Bool
    var selectedModelBlockingIssue: String?
    var accessibilityTrusted: Bool
    var microphone: String
    var speechRecognition: String
    var liveDictationPrerequisitesReady: Bool
    var appleSpeechReady: Bool
    var nativeLocalModelIDs: [String]
    var nativeTranscriptionPaths: [String]
    var blockingIssues: [String]
    var warnings: [String]
}

private struct SelectedModelAssessment {
    var kind: SelectedModelKind
    var runnable: Bool
    var blockingIssue: String = ""
}

private enum SelectedModelKind: String {
    case nativeLocalWhisper = "native_local_whisper"
    case nativeAPI = "native_api"
    case nativeAppleSpeech = "native_apple_speech"
    case unknown
}
