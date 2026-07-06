import AppKit

struct AppLaunchConfiguration: Equatable {
    var startHidden: Bool
    var showMenuBarIcon: Bool

    init(settings: AppSettings, launchArguments: NativeLaunchArguments = .none) {
        startHidden = settings.startHidden || launchArguments.startHidden
        showMenuBarIcon = settings.showMenuBarIcon && !launchArguments.noTray
    }

    init(startHidden: Bool, showMenuBarIcon: Bool) {
        self.startHidden = startHidden
        self.showMenuBarIcon = showMenuBarIcon
    }

    var activationPolicy: NSApplication.ActivationPolicy {
        startHidden && showMenuBarIcon ? .accessory : .regular
    }

    var shouldActivateOnLaunch: Bool {
        startHidden == false
    }

    var shouldHideOnLaunch: Bool {
        startHidden
    }
}
