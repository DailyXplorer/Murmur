import SwiftUI

struct DebugView: View {
    @Environment(\.colorScheme) private var colorScheme
    @EnvironmentObject private var appModel: AppModel
    @Environment(\.murmurTheme) private var murmurTheme

    var body: some View {
        VStack(alignment: .leading, spacing: 24) {
            MurmurSettingsGroup("DEBUG") {
                MurmurSettingRow("Log Level", description: "Set the verbosity of logging") {
                    HStack(spacing: 8) {
                        Menu(appModel.settings.logLevel.title) {
                            ForEach(NativeLogLevel.allCases, id: \.self) { level in
                                Button(level.title) {
                                    appModel.updateSettings {
                                        $0.logLevel = level
                                    }
                                }
                            }
                        }
                        .buttonStyle(MurmurButtonStyle(variant: .secondary))

                        Button {
                            appModel.openLogsFolder()
                        } label: {
                            MurmurHugeIcon(kind: .folderOpen, color: MurmurDesign.text, size: 18)
                        }
                        .buttonStyle(MurmurButtonStyle(variant: .secondary))
                        .help("Open logs folder")
                    }
                }
                MurmurSettingRow("Sound Theme", description: "Choose a sound theme for recording start and stop feedback") {
                    HStack(spacing: 8) {
                        Menu(appModel.settings.soundTheme.title) {
                            ForEach(AudioFeedbackTheme.allCases, id: \.self) { theme in
                                Button(theme.title) {
                                    appModel.updateSettings {
                                        $0.soundTheme = theme
                                    }
                                }
                            }
                        }
                        .buttonStyle(MurmurButtonStyle(variant: .secondary))

                        Button {
                            appModel.previewFeedbackSounds()
                        } label: {
                            MurmurHugeIcon(kind: .play, color: MurmurDesign.text, size: 18)
                        }
                        .buttonStyle(MurmurButtonStyle(variant: .secondary))
                        .help("Preview sound theme")
                    }
                }
                MurmurDivider()
                MurmurSettingRow("Word Correction Threshold", description: "Sensitivity for custom word corrections") {
                    HStack(spacing: 8) {
                        Slider(value: wordCorrectionThresholdBinding, in: 0...1)
                            .tint(murmurTheme.logoPrimary(for: colorScheme))
                            .frame(width: 160)
                        MurmurPill(text: String(format: "%.2f", appModel.settings.wordCorrectionThreshold))
                    }
                }
                MurmurDivider()
                MurmurSettingRow("Paste Delay", description: "Delay before sending paste keystroke (in milliseconds). Increase if wrong text is being pasted.") {
                    HStack(spacing: 8) {
                        Slider(value: pasteDelayBinding, in: 10...200, step: 10)
                            .tint(murmurTheme.logoPrimary(for: colorScheme))
                            .frame(width: 160)
                        MurmurPill(text: "\(appModel.settings.pasteDelayMilliseconds)ms")
                    }
                }
                MurmurDivider()
                MurmurSettingRow("Extra Recording Buffer", description: "Extra time (in milliseconds) to keep recording after you release the key, to capture trailing audio. 0 = no extra buffer.") {
                    HStack(spacing: 8) {
                        Slider(value: extraRecordingBufferBinding, in: 0...1_500, step: 50)
                            .tint(murmurTheme.logoPrimary(for: colorScheme))
                            .frame(width: 160)
                        MurmurPill(text: "\(appModel.settings.extraRecordingBufferMilliseconds)ms")
                    }
                }
                MurmurDivider()
                MurmurSettingRow("Always-On Microphone", description: "Keep microphone active for faster response") {
                    MurmurToggle(isOn: binding(\.alwaysOnMicrophone))
                }
                if appModel.isLaptop {
                    MurmurDivider()
                    MurmurSettingRow("Clamshell Microphone", description: "Microphone to use when laptop lid is closed") {
                        HStack(spacing: 8) {
                            Menu(appModel.selectedClamshellMicrophoneDisplayName) {
                                ForEach(appModel.inputDevices) { device in
                                    Button(device.name) {
                                        appModel.selectClamshellMicrophone(device)
                                    }
                                }
                                Divider()
                                Button("Refresh") {
                                    appModel.refreshAudioDevices()
                                }
                            }
                            .buttonStyle(MurmurButtonStyle(variant: .secondary))

                            Button {
                                appModel.updateSettings {
                                    $0.clamshellMicrophoneName = nil
                                }
                            } label: {
                                MurmurHugeIcon(kind: .refresh, color: MurmurDesign.text, size: 18)
                            }
                            .buttonStyle(MurmurButtonStyle(variant: .secondary))
                            .disabled(appModel.settings.clamshellMicrophoneName == nil)
                            .help("Reset")
                        }
                    }
                }
            }
        }
        .frame(maxWidth: .infinity)
    }

    private func binding<Value>(_ keyPath: WritableKeyPath<AppSettings, Value>) -> Binding<Value> {
        Binding(
            get: { appModel.settings[keyPath: keyPath] },
            set: { value in
                appModel.updateSettings { $0[keyPath: keyPath] = value }
            }
        )
    }

    private var wordCorrectionThresholdBinding: Binding<Double> {
        Binding(
            get: { appModel.settings.wordCorrectionThreshold },
            set: { value in
                appModel.updateSettings {
                    $0.wordCorrectionThreshold = AppSettings.clampedWordCorrectionThreshold(value)
                }
            }
        )
    }

    private var pasteDelayBinding: Binding<Double> {
        Binding(
            get: { Double(min(200, max(10, appModel.settings.pasteDelayMilliseconds))) },
            set: { value in
                appModel.updateSettings {
                    $0.pasteDelayMilliseconds = Int(value.rounded())
                }
            }
        )
    }

    private var extraRecordingBufferBinding: Binding<Double> {
        Binding(
            get: { Double(appModel.settings.extraRecordingBufferMilliseconds) },
            set: { value in
                appModel.updateSettings {
                    $0.extraRecordingBufferMilliseconds = AppSettings.clampedExtraRecordingBufferMilliseconds(Int(value.rounded()))
                }
            }
        )
    }
}
