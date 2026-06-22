import SwiftUI

struct DebugView: View {
    @Environment(\.colorScheme) private var colorScheme
    @EnvironmentObject private var appModel: AppModel
    @Environment(\.handyTheme) private var handyTheme

    var body: some View {
        VStack(alignment: .leading, spacing: 24) {
            HandySettingsGroup("DEBUG") {
                HandySettingRow("Log Level", description: "Set the verbosity of logging") {
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
                        .buttonStyle(HandyButtonStyle(variant: .secondary))

                        Button {
                            appModel.openLogsFolder()
                        } label: {
                            HandyHugeIcon(kind: .folderOpen, color: HandyDesign.text, size: 18)
                        }
                        .buttonStyle(HandyButtonStyle(variant: .secondary))
                        .help("Open logs folder")
                    }
                }
                HandySettingRow("Sound Theme", description: "Choose a sound theme for recording start and stop feedback") {
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
                        .buttonStyle(HandyButtonStyle(variant: .secondary))

                        Button {
                            appModel.previewFeedbackSounds()
                        } label: {
                            HandyHugeIcon(kind: .play, color: HandyDesign.text, size: 18)
                        }
                        .buttonStyle(HandyButtonStyle(variant: .secondary))
                        .help("Preview sound theme")
                    }
                }
                HandyDivider()
                HandySettingRow("Word Correction Threshold", description: "Sensitivity for custom word corrections") {
                    HStack(spacing: 8) {
                        Slider(value: wordCorrectionThresholdBinding, in: 0...1)
                            .tint(handyTheme.logoPrimary(for: colorScheme))
                            .frame(width: 160)
                        HandyPill(text: String(format: "%.2f", appModel.settings.wordCorrectionThreshold))
                    }
                }
                HandyDivider()
                HandySettingRow("Paste Delay", description: "Delay before sending paste keystroke (in milliseconds). Increase if wrong text is being pasted.") {
                    HStack(spacing: 8) {
                        Slider(value: pasteDelayBinding, in: 10...200, step: 10)
                            .tint(handyTheme.logoPrimary(for: colorScheme))
                            .frame(width: 160)
                        HandyPill(text: "\(appModel.settings.pasteDelayMilliseconds)ms")
                    }
                }
                HandyDivider()
                HandySettingRow("Extra Recording Buffer", description: "Extra time (in milliseconds) to keep recording after you release the key, to capture trailing audio. 0 = no extra buffer.") {
                    HStack(spacing: 8) {
                        Slider(value: extraRecordingBufferBinding, in: 0...1_500, step: 50)
                            .tint(handyTheme.logoPrimary(for: colorScheme))
                            .frame(width: 160)
                        HandyPill(text: "\(appModel.settings.extraRecordingBufferMilliseconds)ms")
                    }
                }
                HandyDivider()
                HandySettingRow("Always-On Microphone", description: "Keep microphone active for faster response") {
                    HandyToggle(isOn: binding(\.alwaysOnMicrophone))
                }
                if appModel.isLaptop {
                    HandyDivider()
                    HandySettingRow("Clamshell Microphone", description: "Microphone to use when laptop lid is closed") {
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
                            .buttonStyle(HandyButtonStyle(variant: .secondary))

                            Button {
                                appModel.updateSettings {
                                    $0.clamshellMicrophoneName = nil
                                }
                            } label: {
                                HandyHugeIcon(kind: .refresh, color: HandyDesign.text, size: 18)
                            }
                            .buttonStyle(HandyButtonStyle(variant: .secondary))
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
