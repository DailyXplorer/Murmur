import AppKit
import SwiftUI

struct GeneralView: View {
    @Environment(\.colorScheme) private var colorScheme
    @EnvironmentObject private var appModel: AppModel
    @Environment(\.handyTheme) private var handyTheme

    var body: some View {
        VStack(alignment: .leading, spacing: 24) {
            HandySettingsGroup("GENERAL") {
                shortcutRow(appModel.settings.transcribeShortcutBinding, width: 170)

                HandyDivider()
                HandySettingRow("Push To Talk", description: "Hold to record, release to stop") {
                    HandyToggle(isOn: binding(\.pushToTalk))
                }

                if appModel.settings.pushToTalk == false {
                    HandyDivider()
                    shortcutRow(appModel.settings.cancelShortcutBinding, width: 140)
                }
            }

            if appModel.settings.selectedTranscriptionHasModelSettings {
                modelSettingsGroup
            }

            HandySettingsGroup("SOUND") {
                HandySettingRow("Microphone", description: "Select your preferred microphone device") {
                    microphoneControls
                }
                HandyDivider()
                HandySettingRow("Mute While Recording", description: "Mute system audio during recording") {
                    HandyToggle(isOn: binding(\.muteWhileRecording))
                }
                HandyDivider()
                HandySettingRow("Audio Feedback", description: "Play sound when recording starts and stops") {
                    HandyToggle(isOn: binding(\.audioFeedback))
                }
                HandyDivider()
                HandySettingRow("Output Device", description: "Select your preferred audio output device for feedback sounds") {
                    outputDeviceControls
                }
                HandyDivider()
                HandySettingRow("Volume", description: "Adjust the volume of audio feedback sounds") {
                    HStack(spacing: 8) {
                        Slider(value: binding(\.audioFeedbackVolume), in: 0...1)
                            .tint(handyTheme.logoPrimary(for: colorScheme))
                            .frame(width: 180)
                            .disabled(!appModel.settings.audioFeedback)
                        HandyPill(text: "\(Int((appModel.settings.audioFeedbackVolume * 100).rounded()))%")
                    }
                }
            }
        }
        .frame(maxWidth: .infinity)
        .background(GeneralInitialFocusSink())
    }

    private var modelSettingsGroup: some View {
        HandySettingsGroup("MODEL SETTINGS") {
            if appModel.settings.selectedTranscriptionSupportsLanguageSelection {
                HandySettingRow("Language", description: "Select the language for speech recognition. Auto will automatically determine the language, while selecting a specific language can improve accuracy for that language.") {
                    languageControls
                }
            }

            if appModel.settings.selectedTranscriptionSupportsTranslation {
                if appModel.settings.selectedTranscriptionSupportsLanguageSelection {
                    HandyDivider()
                }
                HandySettingRow("Translate to English", description: "Translate supported transcriptions into English.") {
                    HandyToggle(isOn: binding(\.translateToEnglish))
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
            .buttonStyle(HandyButtonStyle(variant: .secondary))

            Button("Reset") {
                appModel.updateSettings {
                    $0.selectedLanguage = AppSettings.defaults.selectedLanguage
                }
            }
            .buttonStyle(HandyButtonStyle(variant: .secondary))
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
            .buttonStyle(HandyButtonStyle(variant: .secondary))

            Button("Reset") {
                appModel.selectMicrophone(.defaultDevice(direction: .input))
            }
            .buttonStyle(HandyButtonStyle(variant: .secondary))
            .disabled(appModel.settings.selectedMicrophoneName == nil)
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
            .buttonStyle(HandyButtonStyle(variant: .secondary))
            .disabled(!appModel.settings.audioFeedback)

            Button("Reset") {
                appModel.selectOutputDevice(.defaultDevice(direction: .output))
            }
            .buttonStyle(HandyButtonStyle(variant: .secondary))
            .disabled(!appModel.settings.audioFeedback || appModel.settings.selectedOutputDeviceName == nil)
        }
    }

    private func shortcutRow(_ shortcut: ShortcutBinding, width: CGFloat) -> some View {
        HandySettingRow(shortcut.name, description: shortcut.description) {
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
                .buttonStyle(HandyButtonStyle(variant: .secondary))
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
