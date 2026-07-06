import Foundation

enum NativePermissionStatusSmokeRunner {
    static func runSynchronouslyAndExit(_ request: NativePermissionStatusSmokeRequest) -> Never {
        do {
            let output = output(for: PermissionService().snapshot())
            try write(output: output, to: request.outputPath)
            if request.outputPath == nil {
                FileHandle.standardOutput.writeLine(try outputString(for: output))
            }
            exit(0)
        } catch {
            FileHandle.standardError.writeLine(error.localizedDescription)
            exit(1)
        }
    }

    static func output(for snapshot: PermissionSnapshot) -> NativePermissionStatusSmokeOutput {
        NativePermissionStatusSmokeOutput(
            success: true,
            prompted: false,
            accessibilityTrusted: snapshot.accessibilityTrusted,
            microphone: snapshot.microphone.rawValue,
            speechRecognition: snapshot.speechRecognition.rawValue,
            shortcutReady: snapshot.accessibilityTrusted,
            pasteKeyboardReady: snapshot.accessibilityTrusted,
            microphoneReady: snapshot.microphone == .granted,
            appleSpeechReady: snapshot.speechRecognition == .granted,
            liveDictationPrerequisitesReady: snapshot.accessibilityTrusted && snapshot.microphone == .granted
        )
    }

    private static func write(output: NativePermissionStatusSmokeOutput, to path: String?) throws {
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

    private static func outputString(for output: NativePermissionStatusSmokeOutput) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

struct NativePermissionStatusSmokeRequest: Equatable {
    var outputPath: String?
}

struct NativePermissionStatusSmokeOutput: Encodable, Equatable {
    var success: Bool
    var prompted: Bool
    var accessibilityTrusted: Bool
    var microphone: String
    var speechRecognition: String
    var shortcutReady: Bool
    var pasteKeyboardReady: Bool
    var microphoneReady: Bool
    var appleSpeechReady: Bool
    var liveDictationPrerequisitesReady: Bool
}
