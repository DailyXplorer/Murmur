import CoreAudio
import Foundation

struct AudioDevice: Identifiable, Equatable {
    enum Direction {
        case input
        case output
    }

    let id: String
    let name: String
    let isDefault: Bool
    let deviceID: AudioDeviceID?
    let uid: String?

    static func defaultDevice(direction: Direction) -> AudioDevice {
        AudioDevice(
            id: "default-\(direction)",
            name: "Default",
            isDefault: true,
            deviceID: nil,
            uid: nil
        )
    }
}
