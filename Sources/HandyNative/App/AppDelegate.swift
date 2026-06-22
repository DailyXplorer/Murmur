import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        FontLoader.registerBundledFonts()
        let launchConfiguration = AppLaunchConfiguration(
            settings: SettingsStore().load(),
            launchArguments: .current
        )
        NSApp.setActivationPolicy(launchConfiguration.activationPolicy)

        if launchConfiguration.shouldActivateOnLaunch {
            NSApp.activate(ignoringOtherApps: true)
            ensureMainWindowVisible()
        } else if launchConfiguration.shouldHideOnLaunch {
            NSApp.hide(nil)
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if flag {
            sender.activate(ignoringOtherApps: true)
            return true
        }

        ensureMainWindowVisible()
        return false
    }

    private func ensureMainWindowVisible(attempt: Int = 0) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
            if let window = NSApp.windows.first(where: Self.isMainWindowCandidate) {
                window.makeKeyAndOrderFront(nil)
                NSApp.activate(ignoringOtherApps: true)
                return
            }

            NSApp.sendAction(Selector(("newWindow:")), to: nil, from: nil)
            NSApp.activate(ignoringOtherApps: true)

            if attempt < 8 {
                self.ensureMainWindowVisible(attempt: attempt + 1)
            }
        }
    }

    private static func isMainWindowCandidate(_ window: NSWindow) -> Bool {
        window.isVisible && !window.isMiniaturized && window.level == .normal
    }
}
