import Foundation

struct AudioPlaybackState: Equatable {
    var entryID: Int64?
    var isPlaying: Bool
    var currentTime: TimeInterval
    var duration: TimeInterval

    static let idle = AudioPlaybackState(
        entryID: nil,
        isPlaying: false,
        currentTime: 0,
        duration: 0
    )

    var progress: Double {
        guard duration > 0 else {
            return 0
        }
        return min(1, max(0, currentTime / duration))
    }
}

enum AudioPlaybackTimeFormatter {
    static func formatted(_ time: TimeInterval) -> String {
        guard time.isFinite, time >= 0 else {
            return "0:00"
        }

        let totalSeconds = Int(time.rounded(.down))
        let minutes = totalSeconds / 60
        let seconds = totalSeconds % 60
        return "\(minutes):\(String(format: "%02d", seconds))"
    }
}
