import AppKit
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
            MurmurMenuBarIcon()
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

private struct MurmurMenuBarIcon: View {
    var body: some View {
        Image(nsImage: MurmurMenuBarIconImage.make())
            .renderingMode(.template)
            .resizable()
            .frame(width: MurmurMenuBarIconImage.size.width, height: MurmurMenuBarIconImage.size.height)
            .accessibilityLabel("Murmur")
    }
}

enum MurmurMenuBarIconImage {
    static let size = NSSize(width: 18, height: 18)

    static func make() -> NSImage {
        let image = NSImage(size: size, flipped: false) { rect in
            guard let context = NSGraphicsContext.current?.cgContext else {
                return false
            }

            let scale = min(rect.width, rect.height) / 24
            let side = 24 * scale
            let originX = rect.minX + (rect.width - side) / 2
            let originY = rect.minY + (rect.height - side) / 2

            context.saveGState()
            context.translateBy(x: originX, y: originY + side)
            context.scaleBy(x: scale, y: -scale)
            context.setStrokeColor(NSColor.black.cgColor)
            context.setLineWidth(1.5)
            context.setLineCap(.round)
            context.setLineJoin(.round)

            context.addPath(CGPath(roundedRect: CGRect(x: 7, y: 2, width: 10, height: 14), cornerWidth: 5, cornerHeight: 5, transform: nil))
            context.strokePath()

            context.beginPath()
            context.move(to: CGPoint(x: 17, y: 7))
            context.addLine(to: CGPoint(x: 14, y: 7))
            context.move(to: CGPoint(x: 17, y: 11))
            context.addLine(to: CGPoint(x: 14, y: 11))
            context.strokePath()

            context.beginPath()
            context.move(to: CGPoint(x: 20, y: 11))
            context.addCurve(
                to: CGPoint(x: 12, y: 19),
                control1: CGPoint(x: 20, y: 15.4183),
                control2: CGPoint(x: 16.4183, y: 19)
            )
            context.move(to: CGPoint(x: 12, y: 19))
            context.addCurve(
                to: CGPoint(x: 4, y: 11),
                control1: CGPoint(x: 7.58172, y: 19),
                control2: CGPoint(x: 4, y: 15.4183)
            )
            context.move(to: CGPoint(x: 12, y: 19))
            context.addLine(to: CGPoint(x: 12, y: 22))
            context.move(to: CGPoint(x: 12, y: 22))
            context.addLine(to: CGPoint(x: 15, y: 22))
            context.move(to: CGPoint(x: 12, y: 22))
            context.addLine(to: CGPoint(x: 9, y: 22))
            context.strokePath()

            context.restoreGState()
            return true
        }
        image.isTemplate = true
        return image
    }
}
