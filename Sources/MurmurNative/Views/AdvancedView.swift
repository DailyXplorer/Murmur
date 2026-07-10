import SwiftUI

struct AdvancedView: View {
    @Environment(\.colorScheme) private var colorScheme
    @EnvironmentObject private var appModel: AppModel
    @Environment(\.murmurTheme) private var murmurTheme

    @State private var draftCustomWord = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 24) {
            MurmurSettingsGroup("APP") {
                MurmurSettingRow("Start hidden", description: "Launch Murmur without opening the main window.") {
                    MurmurToggle(isOn: binding(\.startHidden))
                }
                MurmurDivider()
                MurmurSettingRow("Start at login", description: "Open Murmur automatically when macOS starts.") {
                    MurmurToggle(isOn: launchAtLoginBinding)
                }
                MurmurDivider()
                MurmurSettingRow("Show tray icon", description: "Keep Murmur available from the menu bar.") {
                    MurmurToggle(isOn: binding(\.showMenuBarIcon))
                }
                MurmurDivider()
                MurmurSettingRow("App theme", description: "Follow the current Murmur theme setting.") {
                    appThemeSelector
                }
                MurmurDivider()
                MurmurSettingRow("Overlay Position", description: "Display visual feedback overlay during recording and transcription.") {
                    overlayPositionMenu
                }
                MurmurDivider()
                MurmurSettingRow("Unload Model", description: "Automatically free GPU/CPU memory when the model hasn't been used for the specified time") {
                    modelUnloadMenu
                }
                MurmurDivider()
                MurmurSettingRow("Experimental Features", description: "Enable experimental features that are still in development.") {
                    MurmurToggle(isOn: binding(\.experimentalEnabled))
                }
            }

            MurmurSettingsGroup("OUTPUT") {
                MurmurSettingRow("Paste method", description: "How transcribed text is inserted into the active app.") {
                    Menu(appModel.settings.pasteMethod.macOSCompatible.title) {
                        ForEach(PasteMethod.macOSCases, id: \.self) { method in
                            Button(method.title) {
                                appModel.updateSettings {
                                    $0.pasteMethod = method
                                }
                            }
                        }
                    }
                    .buttonStyle(MurmurButtonStyle(variant: .secondary))
                }
                MurmurDivider()
                MurmurSettingRow("Clipboard handling", description: "Choose whether the transcript remains on the clipboard.") {
                    Menu(appModel.settings.clipboardHandling.title) {
                        ForEach(ClipboardHandling.allCases, id: \.self) { handling in
                            Button(handling.title) {
                                appModel.updateSettings {
                                    $0.clipboardHandling = handling
                                    $0.restoreClipboardAfterPaste = handling == .dontModify
                                }
                            }
                        }
                    }
                    .buttonStyle(MurmurButtonStyle(variant: .secondary))
                }
                MurmurDivider()
                MurmurSettingRow("Auto-submit", description: "Press a submit key after inserting the transcription.") {
                    Menu(autoSubmitTitle) {
                        Button("Off") {
                            appModel.updateSettings {
                                $0.autoSubmitAfterPaste = false
                            }
                        }
                        ForEach(AutoSubmitKey.allCases, id: \.self) { key in
                            Button(key.title) {
                                appModel.updateSettings {
                                    $0.autoSubmitKey = key
                                    $0.autoSubmitAfterPaste = true
                                }
                            }
                        }
                    }
                    .buttonStyle(MurmurButtonStyle(variant: .secondary))
                }
            }

            MurmurSettingsGroup("TRANSCRIPTION") {
                MurmurSettingRow("Custom words", description: "Words to bias or correct after transcription.") {
                    customWordsEditor
                }
                MurmurDivider()
                MurmurSettingRow("Append trailing space", description: "Add a space after pasted transcriptions.") {
                    MurmurToggle(isOn: binding(\.appendTrailingSpace))
                }
            }

            MurmurSettingsGroup("HISTORY") {
                MurmurSettingRow("History limit", description: "Maximum number of entries kept in local history.") {
                    Stepper("\(appModel.settings.historyLimit)", value: binding(\.historyLimit), in: 1...100, step: 1)
                        .font(MurmurDesign.font(size: 13))
                }
                MurmurDivider()
                MurmurSettingRow("Recording retention", description: "How long unsaved recordings are kept.") {
                    Menu(appModel.settings.recordingRetentionPeriod.title(historyLimit: appModel.settings.historyLimit)) {
                        ForEach(RecordingRetentionPeriod.allCases, id: \.self) { period in
                            Button(period.title(historyLimit: appModel.settings.historyLimit)) {
                                appModel.updateSettings {
                                    $0.recordingRetentionPeriod = period
                                }
                            }
                        }
                    }
                    .buttonStyle(MurmurButtonStyle(variant: .secondary))
                }
            }

            if appModel.settings.experimentalEnabled {
                MurmurSettingsGroup("EXPERIMENTAL") {
                    MurmurSettingRow("Post Processing", description: "Enable AI-powered text refinement after transcription.") {
                        MurmurToggle(isOn: binding(\.postProcessEnabled))
                    }
                    MurmurDivider()
                    MurmurSettingRow("Keyboard Implementation", description: "Choose the keyboard shortcut backend.") {
                        keyboardImplementationMenu
                    }
                    MurmurDivider()
                    MurmurSettingRow("Whisper Acceleration", description: "Hardware acceleration for Whisper models. Auto uses GPU if available (Metal on macOS).") {
                        whisperAccelerationMenu
                    }
                    MurmurDivider()
                    MurmurSettingRow("Keep Mic Open Between Transcriptions", description: "Keeps the microphone stream open for 30 seconds after recording stops, reducing latency for back-to-back transcriptions. May degrade Bluetooth audio quality while active.") {
                        MurmurToggle(isOn: binding(\.lazyStreamClose))
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

    private var launchAtLoginBinding: Binding<Bool> {
        Binding(
            get: { appModel.settings.autostartEnabled },
            set: { appModel.setLaunchAtLoginEnabled($0) }
        )
    }

    private var autoSubmitTitle: String {
        appModel.settings.autoSubmitAfterPaste ? appModel.settings.autoSubmitKey.title : "Off"
    }

    private var appThemeSelector: some View {
        HStack(spacing: 4) {
            ForEach(AppTheme.allCases) { theme in
                let isSelected = appModel.settings.appTheme == theme
                Button {
                    appModel.updateSettings {
                        $0.appTheme = theme
                    }
                } label: {
                    ZStack {
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .fill(theme.palette.swatchColor)

                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .stroke(
                                isSelected ? murmurTheme.logoStroke(for: colorScheme) : MurmurDesign.midGray.opacity(0.4),
                                lineWidth: isSelected ? 1.5 : 1
                            )

                        if isSelected {
                            MurmurHugeIcon(
                                kind: .check,
                                color: murmurTheme.logoStroke(for: colorScheme),
                                size: 16,
                                strokeWidth: 2.2
                            )
                        }
                    }
                    .frame(width: 32, height: 32)
                    .overlay {
                        if isSelected {
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .stroke(murmurTheme.logoPrimary(for: colorScheme).opacity(0.5), lineWidth: 3)
                                .padding(-3)
                        }
                    }
                }
                .buttonStyle(.plain)
                .help(theme.title)
                .accessibilityLabel(theme.title)
                .accessibilityAddTraits(isSelected ? .isSelected : [])
            }
        }
    }

    private var overlayPositionMenu: some View {
        Menu(appModel.settings.overlayPosition.title) {
            ForEach(OverlayPosition.allCases, id: \.self) { position in
                Button(position.title) {
                    appModel.updateSettings {
                        $0.overlayPosition = position
                        $0.showOverlay = position != .none
                    }
                }
            }
        }
        .buttonStyle(MurmurButtonStyle(variant: .secondary))
    }

    private var modelUnloadMenu: some View {
        Menu(appModel.settings.modelUnloadTimeout.title) {
            ForEach(modelUnloadOptions, id: \.self) { timeout in
                Button(timeout.title) {
                    appModel.updateSettings {
                        $0.modelUnloadTimeout = timeout
                    }
                }
            }
        }
        .buttonStyle(MurmurButtonStyle(variant: .secondary))
    }

    private var whisperAccelerationMenu: some View {
        Menu(appModel.settings.whisperAccelerator.title) {
            ForEach(WhisperAcceleratorSetting.allCases, id: \.self) { accelerator in
                Button(accelerator.title) {
                    appModel.updateSettings {
                        $0.whisperAccelerator = accelerator
                    }
                }
            }
        }
        .buttonStyle(MurmurButtonStyle(variant: .secondary))
    }

    private var keyboardImplementationMenu: some View {
        Menu(appModel.settings.keyboardImplementation.title) {
            ForEach(KeyboardImplementationSetting.allCases, id: \.self) { implementation in
                Button(implementation.title) {
                    appModel.updateSettings {
                        $0.keyboardImplementation = implementation
                    }
                }
            }
        }
        .buttonStyle(MurmurButtonStyle(variant: .secondary))
    }

    private var modelUnloadOptions: [ModelUnloadTimeout] {
        appModel.settings.debugMode ? ModelUnloadTimeout.allCases : ModelUnloadTimeout.standardCases
    }

    private var customWordsEditor: some View {
        VStack(alignment: .trailing, spacing: 8) {
            HStack(spacing: 8) {
                TextField("Add a word", text: $draftCustomWord)
                    .textFieldStyle(MurmurTextFieldStyle())
                    .frame(width: 200)
                    .onSubmit(addCustomWord)

                Button("Add", action: addCustomWord)
                    .buttonStyle(MurmurButtonStyle())
                    .frame(minWidth: 64)
                    .disabled(AppSettings.sanitizeCustomWord(draftCustomWord) == nil)
            }

            if appModel.settings.customWords.isEmpty == false {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(appModel.settings.customWords, id: \.self) { word in
                            Button {
                                appModel.removeCustomWord(word)
                            } label: {
                                HStack(spacing: 4) {
                                    Text(word)
                                    MurmurHugeIcon(
                                        kind: .cancelCircle,
                                        color: MurmurDesign.text.opacity(0.85),
                                        size: 12,
                                        strokeWidth: 2
                                    )
                                }
                            }
                            .buttonStyle(MurmurButtonStyle(variant: .secondary))
                        }
                    }
                    .padding(.vertical, 1)
                }
                .frame(width: 280)
            }
        }
    }

    private func addCustomWord() {
        if appModel.addCustomWord(draftCustomWord) {
            draftCustomWord = ""
        }
    }
}
