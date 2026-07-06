import SwiftUI

struct MenuBarView: View {
    @Environment(\.openWindow) private var openWindow
    @EnvironmentObject private var appModel: AppModel

    var body: some View {
        Button("Murmur v\(appModel.appVersion)") {}
            .disabled(true)

        Divider()

        Button("Settings") {
            openWindow(id: "main")
            NSApp.activate(ignoringOtherApps: true)
        }
        .keyboardShortcut(",", modifiers: [.command])

        if appModel.recordingState == .idle {
            Button(appModel.recordingActionTitle) {
                appModel.toggleRecording()
            }
            .keyboardShortcut(.space, modifiers: [.option])
        } else {
            Button("Cancel") {
                appModel.cancelRecording()
            }
        }

        Divider()

        Button("Copy Last Transcript") {
            appModel.copyLatestTranscript()
        }
        .disabled(!appModel.canCopyLatestTranscript)

        Button("Open Recordings Folder") {
            appModel.openRecordingsFolder()
        }

        Divider()

        Menu("Model: \(appModel.selectedTranscriptionModelDisplayName)") {
            ForEach(appModel.menuBarModelOptions) { option in
                Button(option.isSelected ? "\(option.title) (Active)" : option.title) {
                    appModel.selectTranscriptionModel(id: option.id)
                }
                .disabled(option.isSelected || !option.isEnabled)
            }
        }

        Button("Unload Model") {
            appModel.unloadCurrentModelFromMenuBar()
        }
        .disabled(!appModel.canUnloadCurrentModelFromMenuBar)

        Divider()

        Button("Quit") {
            NSApplication.shared.terminate(nil)
        }
        .keyboardShortcut("q")
    }
}
