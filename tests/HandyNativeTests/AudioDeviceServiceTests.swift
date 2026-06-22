import CoreAudio
@testable import HandyNative
import XCTest

final class AudioDeviceServiceTests: XCTestCase {
    func testDisplayNameFallsBackToDefaultWhenSelectionIsMissing() {
        let devices: [AudioDevice] = [
            .defaultDevice(direction: .input),
            AudioDevice(
                id: "1",
                name: "Studio Mic",
                isDefault: false,
                deviceID: AudioDeviceID(1),
                uid: "studio-mic"
            )
        ]

        XCTAssertEqual(AudioDeviceService.displayName(for: nil, devices: devices), "Default")
        XCTAssertEqual(AudioDeviceService.displayName(for: "Removed Mic", devices: devices), "Default")
        XCTAssertEqual(AudioDeviceService.displayName(for: "Studio Mic", devices: devices), "Studio Mic")
    }

    func testDeviceListsAlwaysIncludeDefaultEntry() {
        XCTAssertEqual(AudioDeviceService.inputDevices().first?.name, "Default")
        XCTAssertEqual(AudioDeviceService.outputDevices().first?.name, "Default")
    }

    func testEffectiveInputDeviceUsesClamshellMicrophoneOnlyWhenClosed() {
        XCTAssertEqual(
            AudioDeviceService.effectiveInputDeviceName(
                selectedMicrophoneName: "Studio Mic",
                clamshellMicrophoneName: "Desk Mic",
                isClamshellClosed: false
            ),
            "Studio Mic"
        )
        XCTAssertEqual(
            AudioDeviceService.effectiveInputDeviceName(
                selectedMicrophoneName: "Studio Mic",
                clamshellMicrophoneName: "Desk Mic",
                isClamshellClosed: true
            ),
            "Desk Mic"
        )
        XCTAssertNil(
            AudioDeviceService.effectiveInputDeviceName(
                selectedMicrophoneName: "Default",
                clamshellMicrophoneName: "default",
                isClamshellClosed: true
            )
        )
    }
}
