import AVFoundation
import Foundation

protocol AudioInputVoiceProcessingNode: AnyObject {
    var isVoiceProcessingEnabled: Bool { get }
    var isVoiceProcessingBypassed: Bool { get set }
    var isVoiceProcessingAGCEnabled: Bool { get set }

    func setVoiceProcessingEnabled(_ enabled: Bool) throws
}

extension AVAudioInputNode: AudioInputVoiceProcessingNode {}

struct AudioInputVoiceProcessingConfiguration: Equatable {
    var isEnabled: Bool
    var fallbackToUnprocessedAudio: Bool

    static let handyDefault = AudioInputVoiceProcessingConfiguration(
        isEnabled: true,
        fallbackToUnprocessedAudio: true
    )
}

extension AppSettings {
    var audioInputVoiceProcessingConfiguration: AudioInputVoiceProcessingConfiguration {
        AudioInputVoiceProcessingConfiguration(
            isEnabled: appleVoiceProcessingEnabled,
            fallbackToUnprocessedAudio: true
        )
    }
}

enum AudioInputVoiceProcessingStatus: Equatable {
    case notConfigured
    case disabled
    case enabled(automaticGainControlEnabled: Bool)
    case unavailable(reason: String)

    var isVoiceProcessingEnabled: Bool {
        if case .enabled = self {
            return true
        }

        return false
    }

    var name: String {
        switch self {
        case .notConfigured:
            "not_configured"
        case .disabled:
            "disabled"
        case .enabled:
            "enabled"
        case .unavailable:
            "unavailable"
        }
    }

    var automaticGainControlEnabled: Bool? {
        switch self {
        case let .enabled(automaticGainControlEnabled):
            automaticGainControlEnabled
        case .notConfigured, .disabled, .unavailable:
            nil
        }
    }

    var fallbackReason: String? {
        switch self {
        case let .unavailable(reason):
            reason
        case .notConfigured, .disabled, .enabled:
            nil
        }
    }
}

enum AudioInputVoiceProcessingError: LocalizedError {
    case didNotEnable

    var errorDescription: String? {
        switch self {
        case .didNotEnable:
            "Apple voice processing did not report an enabled state."
        }
    }
}

enum AudioInputVoiceProcessingConfigurator {
    static func configure(
        _ node: any AudioInputVoiceProcessingNode,
        configuration: AudioInputVoiceProcessingConfiguration = .handyDefault
    ) throws -> AudioInputVoiceProcessingStatus {
        guard configuration.isEnabled else {
            try node.setVoiceProcessingEnabled(false)
            return .disabled
        }

        do {
            try node.setVoiceProcessingEnabled(true)
            guard node.isVoiceProcessingEnabled else {
                throw AudioInputVoiceProcessingError.didNotEnable
            }

            node.isVoiceProcessingBypassed = false
            node.isVoiceProcessingAGCEnabled = true
            return .enabled(automaticGainControlEnabled: node.isVoiceProcessingAGCEnabled)
        } catch {
            guard configuration.fallbackToUnprocessedAudio else {
                throw error
            }

            try? node.setVoiceProcessingEnabled(false)
            return .unavailable(reason: error.localizedDescription)
        }
    }
}

enum AudioInputVoiceProcessingInputPreparation {
    static func prepare(
        selectedDeviceID: AudioDeviceID?,
        setInputDevice: (AudioDeviceID) throws -> Void,
        configureVoiceProcessing: () throws -> AudioInputVoiceProcessingStatus
    ) throws -> AudioInputVoiceProcessingStatus {
        if let selectedDeviceID {
            try setInputDevice(selectedDeviceID)
        }
        let status = try configureVoiceProcessing()
        if let selectedDeviceID {
            try setInputDevice(selectedDeviceID)
        }
        return status
    }
}
