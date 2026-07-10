import AppKit
import SwiftUI

struct GeneralView: View {
    @Environment(\.colorScheme) private var colorScheme
    @EnvironmentObject private var appModel: AppModel
    @Environment(\.murmurTheme) private var murmurTheme

    var body: some View {
        VStack(alignment: .leading, spacing: 24) {
            MurmurSettingsGroup("GENERAL") {
                shortcutRow(appModel.settings.transcribeShortcutBinding, width: 170)
                shortcutStatusRow

                MurmurDivider()
                MurmurSettingRow("Push To Talk", description: "Hold to record, release to stop") {
                    MurmurToggle(isOn: binding(\.pushToTalk))
                }

                if appModel.settings.pushToTalk == false {
                    MurmurDivider()
                    shortcutRow(appModel.settings.cancelShortcutBinding, width: 140)
                }
            }

            if appModel.settings.selectedTranscriptionHasModelSettings {
                modelSettingsGroup
            }

            MurmurSettingsGroup("SOUND") {
                MurmurSettingRow("Microphone", description: "Select your preferred microphone device") {
                    microphoneControls
                }
                MurmurDivider()
                MurmurSettingRow("Apple Voice Processing", description: appleVoiceProcessingDescription) {
                    MurmurToggle(isOn: binding(\.appleVoiceProcessingEnabled))
                }
                MurmurDivider()
                MurmurSettingRow("Mute While Recording", description: "Mute system audio during recording") {
                    MurmurToggle(isOn: binding(\.muteWhileRecording))
                }
                MurmurDivider()
                MurmurSettingRow("Audio Feedback", description: "Play sound when recording starts and stops") {
                    MurmurToggle(isOn: binding(\.audioFeedback))
                }
                MurmurDivider()
                MurmurSettingRow("Output Device", description: "Select your preferred audio output device for feedback sounds") {
                    outputDeviceControls
                }
                MurmurDivider()
                MurmurSettingRow("Volume", description: "Adjust the volume of audio feedback sounds") {
                    HStack(spacing: 8) {
                        Slider(value: binding(\.audioFeedbackVolume), in: 0...1)
                            .tint(murmurTheme.logoPrimary(for: colorScheme))
                            .frame(width: 180)
                            .disabled(!appModel.settings.audioFeedback)
                        MurmurPill(text: "\(Int((appModel.settings.audioFeedbackVolume * 100).rounded()))%")
                    }
                }
            }
        }
        .frame(maxWidth: .infinity)
        .background(GeneralInitialFocusSink())
    }

    private var modelSettingsGroup: some View {
        MurmurSettingsGroup("MODEL SETTINGS") {
            if appModel.settings.selectedTranscriptionSupportsLanguageSelection {
                MurmurSettingRow("Language", description: "Select the language for speech recognition. Auto will automatically determine the language, while selecting a specific language can improve accuracy for that language.") {
                    languageControls
                }
            }

            if appModel.settings.selectedTranscriptionSupportsTranslation {
                if appModel.settings.selectedTranscriptionSupportsLanguageSelection {
                    MurmurDivider()
                }
                MurmurSettingRow("Translate to English", description: "Translate supported transcriptions into English.") {
                    MurmurToggle(isOn: binding(\.translateToEnglish))
                }
            }
        }
    }

    private var languageControls: some View {
        HStack(spacing: 8) {
            Menu(TranscriptionLanguage.displayName(for: appModel.settings.selectedLanguage)) {
                ForEach(TranscriptionLanguage.all) { language in
                    Button(language.name) {
                        appModel.updateSettings {
                            $0.selectedLanguage = language.code
                        }
                    }
                }
            }
            .buttonStyle(MurmurButtonStyle(variant: .secondary))

            Button("Reset") {
                appModel.updateSettings {
                    $0.selectedLanguage = AppSettings.defaults.selectedLanguage
                }
            }
            .buttonStyle(MurmurButtonStyle(variant: .secondary))
            .disabled(appModel.settings.selectedLanguage == AppSettings.defaults.selectedLanguage)
        }
    }

    private var microphoneControls: some View {
        HStack(spacing: 8) {
            Menu(appModel.selectedMicrophoneDisplayName) {
                ForEach(appModel.inputDevices) { device in
                    Button(device.name) {
                        appModel.selectMicrophone(device)
                    }
                }
                Divider()
                Button("Refresh") {
                    appModel.refreshAudioDevices()
                }
            }
            .buttonStyle(MurmurButtonStyle(variant: .secondary))

            Button("Reset") {
                appModel.selectMicrophone(.defaultDevice(direction: .input))
            }
            .buttonStyle(MurmurButtonStyle(variant: .secondary))
            .disabled(appModel.settings.selectedMicrophoneName == nil)
        }
    }

    private var appleVoiceProcessingDescription: String {
        if appModel.settings.appleVoiceProcessingEnabled == false {
            return "Apple microphone processing is off."
        }

        switch appModel.audioInputVoiceProcessingStatus {
        case .enabled:
            return "Apple noise suppression and automatic gain control are active."
        case .unavailable:
            return "Unavailable for the current microphone; Murmur is using raw input."
        case .disabled:
            return "Apple microphone processing is off."
        case .notConfigured:
            return "Use macOS voice processing before transcription."
        }
    }

    private var outputDeviceControls: some View {
        HStack(spacing: 8) {
            Menu(appModel.selectedOutputDeviceDisplayName) {
                ForEach(appModel.outputDevices) { device in
                    Button(device.name) {
                        appModel.selectOutputDevice(device)
                    }
                }
                Divider()
                Button("Refresh") {
                    appModel.refreshAudioDevices()
                }
            }
            .buttonStyle(MurmurButtonStyle(variant: .secondary))
            .disabled(!appModel.settings.audioFeedback)

            Button("Reset") {
                appModel.selectOutputDevice(.defaultDevice(direction: .output))
            }
            .buttonStyle(MurmurButtonStyle(variant: .secondary))
            .disabled(!appModel.settings.audioFeedback || appModel.settings.selectedOutputDeviceName == nil)
        }
    }

    private var shortcutStatusRow: some View {
        Text(appModel.globalShortcutStatus)
            .font(MurmurDesign.font(size: 12))
            .foregroundStyle(shortcutStatusIsFailure ? Color.red : MurmurDesign.midGray)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.bottom, 8)
    }

    private var shortcutStatusIsFailure: Bool {
        let failureStatuses: Set<String> = [
            "Accessibility required",
            "No valid shortcut",
            "Shortcut unavailable",
            "Transcribe shortcut not set",
            AppModel.secureInputStatusMessage,
        ]
        return failureStatuses.contains(appModel.globalShortcutStatus)
    }

    private func shortcutRow(_ shortcut: ShortcutBinding, width: CGFloat) -> some View {
        MurmurSettingRow(shortcut.name, description: shortcut.description) {
            HStack(spacing: 8) {
                ShortcutBindingField(
                    placeholder: shortcut.defaultBinding,
                    currentBinding: shortcut.currentBinding,
                    width: width,
                    reservedBindings: reservedShortcutBindings(excluding: shortcut.id)
                ) {
                    appModel.updateShortcutBinding(id: shortcut.id, currentBinding: $0)
                }
                Button("Reset") {
                    resetShortcut(shortcut.id)
                }
                .buttonStyle(MurmurButtonStyle(variant: .secondary))
                .disabled(shortcut.currentBinding == shortcut.defaultBinding)
            }
            .fixedSize(horizontal: true, vertical: false)
        }
    }

    private func binding<Value>(_ keyPath: WritableKeyPath<AppSettings, Value>) -> Binding<Value> {
        Binding(
            get: { appModel.settings[keyPath: keyPath] },
            set: { value in
                appModel.updateSettings { $0[keyPath: keyPath] = value }
            }
        )
    }

    private func resetShortcut(_ id: String) {
        guard let defaultBinding = ShortcutBinding.defaults[id]?.defaultBinding else {
            return
        }
        appModel.updateShortcutBinding(id: id, currentBinding: defaultBinding)
    }

    private func reservedShortcutBindings(excluding id: String) -> [String] {
        appModel.settings.shortcutBindings.values
            .filter { $0.id != id }
            .map(\.currentBinding)
    }
}

private struct GeneralInitialFocusSink: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = GeneralFocusSinkView()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            view.window?.makeFirstResponder(view)
        }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}
}

private final class GeneralFocusSinkView: NSView {
    override var acceptsFirstResponder: Bool {
        true
    }
}
