import AppKit
import Foundation

enum AudioFeedbackSound: String {
    case start
    case stop
}

@MainActor
final class AudioFeedbackService {
    private var sounds: [NSSound] = []

    func play(_ sound: AudioFeedbackSound, settings: AppSettings, paths: AppPaths) {
        guard settings.audioFeedback,
              let url = Self.soundURL(for: sound, settings: settings, paths: paths)
        else {
            return
        }

        sounds.removeAll { $0.isPlaying == false }
        guard let sound = NSSound(contentsOf: url, byReference: true) else {
            // Audio feedback should never interrupt recording/transcription.
            return
        }
        sound.volume = Float(min(1, max(0, settings.audioFeedbackVolume)))
        sound.playbackDeviceIdentifier = AudioDeviceService.outputDeviceUID(named: settings.selectedOutputDeviceName)
        sounds.append(sound)
        sound.play()
    }

    nonisolated static func soundFileName(for sound: AudioFeedbackSound, theme: AudioFeedbackTheme) -> String {
        switch theme {
        case .custom:
            "custom_\(sound.rawValue).wav"
        case .marimba, .pop:
            "\(theme.rawValue)_\(sound.rawValue).wav"
        }
    }

    nonisolated static func soundURL(for sound: AudioFeedbackSound, settings: AppSettings, paths: AppPaths) -> URL? {
        let fileName = soundFileName(for: sound, theme: settings.soundTheme)

        if settings.soundTheme == .custom {
            return paths.appDataDirectory.appendingPathComponent(fileName)
        }

        return Bundle.main.url(forResource: fileName.replacingOccurrences(of: ".wav", with: ""), withExtension: "wav")
    }
}
