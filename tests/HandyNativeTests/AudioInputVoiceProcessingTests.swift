import CoreAudio
import Foundation
import XCTest
@testable import HandyNative

final class AudioInputVoiceProcessingTests: XCTestCase {
    func testConfiguratorEnablesAppleVoiceProcessingAndAGC() throws {
        let node = FakeAudioInputVoiceProcessingNode()

        let status = try AudioInputVoiceProcessingConfigurator.configure(node)

        XCTAssertEqual(status, .enabled(automaticGainControlEnabled: true))
        XCTAssertEqual(status.name, "enabled")
        XCTAssertTrue(status.isVoiceProcessingEnabled)
        XCTAssertEqual(status.automaticGainControlEnabled, true)
        XCTAssertNil(status.fallbackReason)
        XCTAssertEqual(node.voiceProcessingRequests, [true])
        XCTAssertTrue(node.isVoiceProcessingEnabled)
        XCTAssertFalse(node.isVoiceProcessingBypassed)
        XCTAssertTrue(node.isVoiceProcessingAGCEnabled)
    }

    func testConfiguratorFallsBackToRawAudioWhenVoiceProcessingIsUnavailable() throws {
        let node = FakeAudioInputVoiceProcessingNode()
        node.enableFailure = NSError(
            domain: "AudioInputVoiceProcessingTests",
            code: 1,
            userInfo: [NSLocalizedDescriptionKey: "Selected input device rejected voice processing."]
        )

        let status = try AudioInputVoiceProcessingConfigurator.configure(node)

        XCTAssertEqual(
            status,
            .unavailable(reason: "Selected input device rejected voice processing.")
        )
        XCTAssertEqual(status.name, "unavailable")
        XCTAssertFalse(status.isVoiceProcessingEnabled)
        XCTAssertNil(status.automaticGainControlEnabled)
        XCTAssertEqual(status.fallbackReason, "Selected input device rejected voice processing.")
        XCTAssertEqual(node.voiceProcessingRequests, [true, false])
        XCTAssertFalse(node.isVoiceProcessingEnabled)
    }

    func testConfiguratorThrowsWhenFallbackIsDisabled() {
        let node = FakeAudioInputVoiceProcessingNode()
        node.enableFailure = NSError(
            domain: "AudioInputVoiceProcessingTests",
            code: 2,
            userInfo: [NSLocalizedDescriptionKey: "Voice processing unavailable."]
        )
        let configuration = AudioInputVoiceProcessingConfiguration(
            isEnabled: true,
            fallbackToUnprocessedAudio: false
        )

        XCTAssertThrowsError(
            try AudioInputVoiceProcessingConfigurator.configure(node, configuration: configuration)
        )
        XCTAssertEqual(node.voiceProcessingRequests, [true])
    }

    func testConfiguratorCanDisableVoiceProcessingExplicitly() throws {
        let node = FakeAudioInputVoiceProcessingNode()
        node.isVoiceProcessingEnabled = true
        let configuration = AudioInputVoiceProcessingConfiguration(
            isEnabled: false,
            fallbackToUnprocessedAudio: true
        )

        let status = try AudioInputVoiceProcessingConfigurator.configure(node, configuration: configuration)

        XCTAssertEqual(status, .disabled)
        XCTAssertEqual(node.voiceProcessingRequests, [false])
        XCTAssertFalse(node.isVoiceProcessingEnabled)
    }

    func testInputPreparationAppliesSelectedDeviceBeforeAndAfterVoiceProcessing() throws {
        var events: [String] = []

        let status = try AudioInputVoiceProcessingInputPreparation.prepare(
            selectedDeviceID: AudioDeviceID(42),
            setInputDevice: { deviceID in
                events.append("device:\(deviceID)")
            },
            configureVoiceProcessing: {
                events.append("voice-processing")
                return .enabled(automaticGainControlEnabled: true)
            }
        )

        XCTAssertEqual(status, .enabled(automaticGainControlEnabled: true))
        XCTAssertEqual(events, ["device:42", "voice-processing", "device:42"])
    }
}

private final class FakeAudioInputVoiceProcessingNode: AudioInputVoiceProcessingNode {
    var isVoiceProcessingEnabled = false
    var isVoiceProcessingBypassed = true
    var isVoiceProcessingAGCEnabled = false
    var enableFailure: Error?
    private(set) var voiceProcessingRequests: [Bool] = []

    func setVoiceProcessingEnabled(_ enabled: Bool) throws {
        voiceProcessingRequests.append(enabled)
        if enabled, let enableFailure {
            throw enableFailure
        }
        isVoiceProcessingEnabled = enabled
    }
}
