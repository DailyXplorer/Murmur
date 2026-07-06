import SwiftUI

@main
struct MurmurNativeApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var appModel: AppModel
    @State private var handledLaunchRemoteCommand = false
    private let launchArguments: NativeLaunchArguments

    init() {
        let launchArguments = NativeLaunchArguments.current
        if let smokeModelCacheRequest = launchArguments.smokeModelCacheRequest {
            NativeModelCacheSmokeRunner.runSynchronouslyAndExit(smokeModelCacheRequest)
        }

        if let smokeModelRuntimeRequest = launchArguments.smokeModelRuntimeRequest {
            NativeModelRuntimeSmokeRunner.runSynchronouslyAndExit(smokeModelRuntimeRequest)
        }

        if let smokePermissionStatusRequest = launchArguments.smokePermissionStatusRequest {
            NativePermissionStatusSmokeRunner.runSynchronouslyAndExit(smokePermissionStatusRequest)
        }

        if let smokeReplacementReadinessRequest = launchArguments.smokeReplacementReadinessRequest {
            NativeReplacementReadinessSmokeRunner.runSynchronouslyAndExit(smokeReplacementReadinessRequest)
        }

        if let smokeRemoteControlListenerRequest = launchArguments.smokeRemoteControlListenerRequest {
            NativeRemoteControlSmokeRunner.runListenerSynchronouslyAndExit(smokeRemoteControlListenerRequest)
        }

        if let smokeRemoteControlSendRequest = launchArguments.smokeRemoteControlSendRequest {
            NativeRemoteControlSmokeRunner.runSenderSynchronouslyAndExit(smokeRemoteControlSendRequest)
        }

        if let smokeExternalPasteTargetRequest = launchArguments.smokeExternalPasteTargetRequest {
            NativeExternalPasteTargetSmokeRunner.runSynchronouslyAndExit(smokeExternalPasteTargetRequest)
        }

        if let smokeExternalPasteRoundTripRequest = launchArguments.smokeExternalPasteRoundTripRequest {
            NativeExternalPasteRoundTripSmokeRunner.runSynchronouslyAndExit(smokeExternalPasteRoundTripRequest)
        }

        if let smokeGlobalShortcutRequest = launchArguments.smokeGlobalShortcutRequest {
            NativeGlobalShortcutSmokeRunner.runSynchronouslyAndExit(smokeGlobalShortcutRequest)
        }

        if let smokeGlobalShortcutRecordingRequest = launchArguments.smokeGlobalShortcutRecordingRequest {
            NativeGlobalShortcutRecordingSmokeRunner.runSynchronouslyAndExit(smokeGlobalShortcutRecordingRequest)
        }

        if let smokeAudioRecordingRequest = launchArguments.smokeAudioRecordingRequest {
            NativeAudioRecordingSmokeRunner.runSynchronouslyAndExit(smokeAudioRecordingRequest)
        }

        if let smokeRecordTranscriptionRequest = launchArguments.smokeRecordTranscriptionRequest {
            NativeRecordTranscriptionSmokeRunner.runSynchronouslyAndExit(smokeRecordTranscriptionRequest)
        }

        if let smokePasteRequest = launchArguments.smokePasteRequest {
            NativePasteSmokeRunner.runSynchronouslyAndExit(smokePasteRequest)
        }

        if let smokeTranscriptionRequest = launchArguments.smokeTranscriptionRequest {
            NativeTranscriptionSmokeRunner.runSynchronouslyAndExit(smokeTranscriptionRequest)
        }

        self.launchArguments = launchArguments
        _appModel = StateObject(wrappedValue: AppModel(launchArguments: launchArguments))

        if let command = launchArguments.remoteCommand,
           NativeRemoteControlService.sendToRunningInstance(command) {
            DispatchQueue.main.async {
                NSApp.terminate(nil)
            }
        }
    }

    var body: some Scene {
        Window("Murmur", id: "main") {
            ContentView()
                .environmentObject(appModel)
                .environment(\.murmurAppTheme, appModel.settings.appTheme)
                .environment(\.murmurTheme, appModel.settings.appTheme.palette)
                .frame(minWidth: MurmurDesign.windowWidth, minHeight: MurmurDesign.windowHeight)
                .task {
                    await appModel.refreshPermissions()
                    appModel.applyIdleMicrophonePreference()
                    handleLaunchRemoteCommandIfNeeded()
                }
        }
        .defaultSize(width: MurmurDesign.windowWidth, height: MurmurDesign.windowHeight)
        .windowResizability(.contentMinSize)
        .commands {
            CommandMenu("Transcription") {
                Button(appModel.recordingActionTitle) {
                    appModel.toggleRecording()
                }
                .keyboardShortcut(.space, modifiers: [.option])

                Button("Cancel Recording") {
                    appModel.cancelRecording()
                }
                .keyboardShortcut(.escape, modifiers: [])
                .disabled(!appModel.recordingState.isActive)
            }
        }

        Settings {
            ContentView()
                .environmentObject(appModel)
                .environment(\.murmurAppTheme, appModel.settings.appTheme)
                .environment(\.murmurTheme, appModel.settings.appTheme.palette)
                .frame(width: MurmurDesign.windowWidth, height: MurmurDesign.windowHeight)
        }

        MenuBarExtra(isInserted: menuBarInsertedBinding) {
            MenuBarView()
                .environmentObject(appModel)
                .environment(\.murmurAppTheme, appModel.settings.appTheme)
                .environment(\.murmurTheme, appModel.settings.appTheme.palette)
        } label: {
            Label("Murmur", systemImage: appModel.menuBarSystemImage)
        }
    }

    private var menuBarInsertedBinding: Binding<Bool> {
        Binding(
            get: { appModel.settings.showMenuBarIcon },
            set: { _ in }
        )
    }

    private func handleLaunchRemoteCommandIfNeeded() {
        guard handledLaunchRemoteCommand == false,
              NativeRemoteControlService.hasRunningPeer() == false,
              let command = launchArguments.remoteCommand
        else {
            return
        }

        handledLaunchRemoteCommand = true
        appModel.handleRemoteControlCommand(command)
    }
}
