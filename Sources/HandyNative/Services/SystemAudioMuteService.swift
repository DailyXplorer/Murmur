import Foundation

@MainActor
final class SystemAudioMuteService {
    private var didMute = false

    func applyMuteIfNeeded(settings: AppSettings) {
        guard settings.muteWhileRecording, didMute == false else {
            return
        }

        runMuteCommand(muted: true)
        didMute = true
    }

    func removeMuteIfNeeded() {
        guard didMute else {
            return
        }

        runMuteCommand(muted: false)
        didMute = false
    }

    nonisolated static func appleScript(muted: Bool) -> String {
        "set volume output muted \(muted ? "true" : "false")"
    }

    private func runMuteCommand(muted: Bool) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", Self.appleScript(muted: muted)]
        try? process.run()
    }
}
