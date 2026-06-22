import ApplicationServices
import AVFoundation
import Foundation
import Speech

struct PermissionService {
    func snapshot() -> PermissionSnapshot {
        PermissionSnapshot(
            accessibilityTrusted: AXIsProcessTrusted(),
            microphone: microphoneStatus(),
            speechRecognition: speechRecognitionStatus()
        )
    }

    @discardableResult
    func requestAccessibilityPrompt() -> Bool {
        let options = ["AXTrustedCheckOptionPrompt": true] as CFDictionary
        return AXIsProcessTrustedWithOptions(options)
    }

    func requestMicrophoneAccess() async -> Bool {
        await AVCaptureDevice.requestAccess(for: .audio)
    }

    func requestSpeechRecognitionAccess() async -> Bool {
        await withCheckedContinuation { continuation in
            SFSpeechRecognizer.requestAuthorization { status in
                continuation.resume(returning: status == .authorized)
            }
        }
    }

    private func microphoneStatus() -> PermissionSnapshot.Microphone {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .notDetermined: .notDetermined
        case .authorized: .granted
        case .denied: .denied
        case .restricted: .restricted
        @unknown default: .unknown
        }
    }

    private func speechRecognitionStatus() -> PermissionSnapshot.SpeechRecognition {
        switch SFSpeechRecognizer.authorizationStatus() {
        case .notDetermined: .notDetermined
        case .authorized: .granted
        case .denied: .denied
        case .restricted: .restricted
        @unknown default: .unknown
        }
    }
}
