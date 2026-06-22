import AVFoundation
import Foundation

enum AudioPlaybackError: LocalizedError {
    case missingAudioFile(String)

    var errorDescription: String? {
        switch self {
        case let .missingAudioFile(fileName):
            "Audio file '\(fileName)' was not found."
        }
    }
}

@MainActor
final class AudioPlaybackService: NSObject, AVAudioPlayerDelegate {
    var onStateChange: ((AudioPlaybackState) -> Void)?

    private var player: AVAudioPlayer?
    private var entryID: Int64?
    private var timer: Timer?

    var state: AudioPlaybackState {
        guard let player, let entryID else {
            return .idle
        }

        return AudioPlaybackState(
            entryID: entryID,
            isPlaying: player.isPlaying,
            currentTime: player.currentTime,
            duration: player.duration
        )
    }

    func toggle(entryID: Int64, fileURL: URL) throws {
        if self.entryID == entryID, let player {
            if player.isPlaying {
                player.pause()
                stopTimer()
                emitState()
            } else {
                if player.duration > 0, player.duration - player.currentTime < 0.1 {
                    player.currentTime = 0
                }
                player.play()
                startTimer()
                emitState()
            }
            return
        }

        guard FileManager.default.fileExists(atPath: fileURL.path) else {
            throw AudioPlaybackError.missingAudioFile(fileURL.lastPathComponent)
        }

        stop()
        let newPlayer = try AVAudioPlayer(contentsOf: fileURL)
        newPlayer.delegate = self
        newPlayer.prepareToPlay()
        player = newPlayer
        self.entryID = entryID
        newPlayer.play()
        startTimer()
        emitState()
    }

    func seek(entryID: Int64, to time: TimeInterval) {
        guard self.entryID == entryID, let player else {
            return
        }

        player.currentTime = min(max(0, time), player.duration)
        emitState()
    }

    func stopIfPlaying(entryID: Int64) {
        guard self.entryID == entryID else {
            return
        }
        stop()
    }

    func stop() {
        stopTimer()
        player?.stop()
        player = nil
        entryID = nil
        emitState()
    }

    nonisolated func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) {
        Task { @MainActor in
            stopTimer()
            emitState()
        }
    }

    private func startTimer() {
        stopTimer()
        timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            Task { @MainActor in
                self?.emitState()
            }
        }
    }

    private func stopTimer() {
        timer?.invalidate()
        timer = nil
    }

    private func emitState() {
        onStateChange?(state)
    }
}
