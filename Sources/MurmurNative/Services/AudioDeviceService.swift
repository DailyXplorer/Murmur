import CoreAudio
import Foundation

enum AudioDeviceService {
    static func inputDevices() -> [AudioDevice] {
        devices(direction: .input)
    }

    static func outputDevices() -> [AudioDevice] {
        devices(direction: .output)
    }

    static func inputDeviceID(named name: String?) -> AudioDeviceID? {
        guard let name, name != "Default" else {
            return nil
        }
        return devices(direction: .input).first { $0.name == name }?.deviceID
    }

    static func outputDeviceUID(named name: String?) -> String? {
        guard let name, name != "Default" else {
            return nil
        }
        return devices(direction: .output).first { $0.name == name }?.uid
    }

    static func displayName(for selectedName: String?, devices: [AudioDevice]) -> String {
        guard let selectedName, devices.contains(where: { $0.name == selectedName }) else {
            return "Default"
        }
        return selectedName
    }

    static func effectiveInputDeviceName(
        selectedMicrophoneName: String?,
        clamshellMicrophoneName: String?,
        isClamshellClosed: Bool
    ) -> String? {
        if isClamshellClosed,
           let clamshellMicrophoneName = normalizedDeviceName(clamshellMicrophoneName) {
            return clamshellMicrophoneName
        }

        return normalizedDeviceName(selectedMicrophoneName)
    }

    static func isLaptop() -> Bool {
        run("/usr/bin/pmset", arguments: ["-g", "batt"])?.contains("InternalBattery") == true
    }

    static func isClamshellClosed() -> Bool {
        run("/usr/sbin/ioreg", arguments: ["-r", "-k", "AppleClamshellState", "-d", "4"])?.contains("\"AppleClamshellState\" = Yes") == true
    }

    private static func devices(direction: AudioDevice.Direction) -> [AudioDevice] {
        let ids = audioDeviceIDs()
        let defaultID = defaultDeviceID(direction: direction)
        let explicitDevices = ids.compactMap { id -> AudioDevice? in
            guard hasStreams(deviceID: id, direction: direction),
                  let name = stringProperty(deviceID: id, selector: kAudioObjectPropertyName)
            else {
                return nil
            }

            return AudioDevice(
                id: "\(id)",
                name: name,
                isDefault: id == defaultID,
                deviceID: id,
                uid: stringProperty(deviceID: id, selector: kAudioDevicePropertyDeviceUID)
            )
        }
        .sorted { lhs, rhs in
            if lhs.isDefault != rhs.isDefault {
                return lhs.isDefault
            }
            return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
        }

        var result: [AudioDevice] = [.defaultDevice(direction: direction)]
        result.append(contentsOf: explicitDevices)
        return result
    }

    private static func normalizedDeviceName(_ name: String?) -> String? {
        guard let name = name?.trimmingCharacters(in: .whitespacesAndNewlines),
              name.isEmpty == false,
              name != "Default",
              name != "default"
        else {
            return nil
        }

        return name
    }

    private static func run(_ executablePath: String, arguments: [String]) -> String? {
        let process = Process()
        let outputPipe = Pipe()
        process.executableURL = URL(fileURLWithPath: executablePath)
        process.arguments = arguments
        process.standardOutput = outputPipe
        process.standardError = Pipe()

        do {
            try process.run()
            process.waitUntilExit()
        } catch {
            return nil
        }

        guard process.terminationStatus == 0 else {
            return nil
        }

        let data = outputPipe.fileHandleForReading.readDataToEndOfFile()
        return String(data: data, encoding: .utf8)
    }

    private static func audioDeviceIDs() -> [AudioDeviceID] {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDevices,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var dataSize: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(
            AudioObjectID(kAudioObjectSystemObject),
            &address,
            0,
            nil,
            &dataSize
        ) == noErr, dataSize > 0 else {
            return []
        }

        let count = Int(dataSize) / MemoryLayout<AudioDeviceID>.size
        var deviceIDs = Array(repeating: AudioDeviceID(0), count: count)
        let status = deviceIDs.withUnsafeMutableBufferPointer { buffer -> OSStatus in
            guard let baseAddress = buffer.baseAddress else {
                return OSStatus(paramErr)
            }
            return AudioObjectGetPropertyData(
                AudioObjectID(kAudioObjectSystemObject),
                &address,
                0,
                nil,
                &dataSize,
                baseAddress
            )
        }

        guard status == noErr else {
            return []
        }

        return deviceIDs
    }

    private static func defaultDeviceID(direction: AudioDevice.Direction) -> AudioDeviceID? {
        var address = AudioObjectPropertyAddress(
            mSelector: direction == .input ? kAudioHardwarePropertyDefaultInputDevice : kAudioHardwarePropertyDefaultOutputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var deviceID = AudioDeviceID(0)
        var dataSize = UInt32(MemoryLayout<AudioDeviceID>.size)
        guard AudioObjectGetPropertyData(
            AudioObjectID(kAudioObjectSystemObject),
            &address,
            0,
            nil,
            &dataSize,
            &deviceID
        ) == noErr, deviceID != 0 else {
            return nil
        }
        return deviceID
    }

    private static func hasStreams(deviceID: AudioDeviceID, direction: AudioDevice.Direction) -> Bool {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioDevicePropertyStreams,
            mScope: direction == .input ? kAudioDevicePropertyScopeInput : kAudioDevicePropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMain
        )
        var dataSize: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(
            AudioObjectID(deviceID),
            &address,
            0,
            nil,
            &dataSize
        ) == noErr else {
            return false
        }

        return dataSize > 0
    }

    private static func stringProperty(deviceID: AudioDeviceID, selector: AudioObjectPropertySelector) -> String? {
        var address = AudioObjectPropertyAddress(
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var value: CFString = "" as CFString
        var dataSize = UInt32(MemoryLayout<CFString>.size)
        let status = withUnsafeMutablePointer(to: &value) { pointer in
            AudioObjectGetPropertyData(
                AudioObjectID(deviceID),
                &address,
                0,
                nil,
                &dataSize,
                pointer
            )
        }

        guard status == noErr else {
            return nil
        }

        return value as String
    }
}
