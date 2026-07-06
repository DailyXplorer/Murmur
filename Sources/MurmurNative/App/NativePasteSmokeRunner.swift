import AppKit
import ApplicationServices
import Foundation

enum NativePasteSmokeRunner {
    @MainActor
    static func runSynchronouslyAndExit(_ request: NativePasteSmokeRequest) -> Never {
        let application = NSApplication.shared
        application.setActivationPolicy(request.activationProcessIdentifier == nil ? .regular : .accessory)
        application.finishLaunching()

        Task { @MainActor in
            do {
                let output = try await run(request)
                if let outputPath = request.outputPath {
                    try write(output, to: outputPath)
                } else {
                    FileHandle.standardOutput.writeLine(output)
                }
                exit(0)
            } catch {
                if let outputPath = request.outputPath,
                   let output = try? failureOutput(for: error) {
                    try? write(output, to: outputPath)
                }
                FileHandle.standardError.writeLine(error.localizedDescription)
                exit(1)
            }
        }

        application.run()
        exit(1)
    }

    @MainActor
    static func run(_ request: NativePasteSmokeRequest) async throws -> String {
        let output = try await output(for: request)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }

    @MainActor
    static func output(for request: NativePasteSmokeRequest) async throws -> NativePasteSmokeOutput {
        let settings = settings(for: request)
        let options = PasteOutputOptions(settings: settings)
        let preparedText = PasteService.preparedText(request.text, options: options)
        guard !preparedText.isEmpty else {
            throw PasteServiceError.emptyText
        }

        let permissionSnapshot = PermissionService().snapshot()
        if options.pasteMethod.macOSCompatible != .none,
           permissionSnapshot.accessibilityTrusted == false {
            throw NativePasteSmokeError.accessibilityPermission
        }

        let pasteboard = NSPasteboard.general
        let previousString = pasteboard.string(forType: .string)

        let targetHarness = request.targetWindow ? NativePasteTargetHarness() : nil
        targetHarness?.show()
        if let targetHarness {
            await targetHarness.waitUntilFocused(
                timeoutMilliseconds: max(500, request.startDelayMilliseconds)
            )
        } else {
            let activation = activateTargetApplication(
                processIdentifier: request.activationProcessIdentifier
            )
            try await Task.sleep(for: .milliseconds(request.startDelayMilliseconds))
            activation.observeFrontmostApplication()
            if let requestedProcessIdentifier = request.activationProcessIdentifier,
               activation.frontmostProcessIdentifier != requestedProcessIdentifier {
                throw NativePasteSmokeError.activatedTargetNotFrontmost(
                    requestedProcessIdentifier: requestedProcessIdentifier,
                    frontmostProcessIdentifier: activation.frontmostProcessIdentifier,
                    frontmostBundleIdentifier: activation.frontmostBundleIdentifier,
                    frontmostLocalizedName: activation.frontmostLocalizedName
                )
            }
            return try await pasteAndBuildOutput(
                request,
                preparedText: preparedText,
                options: options,
                pasteboard: pasteboard,
                previousString: previousString,
                accessibilityTrusted: permissionSnapshot.accessibilityTrusted,
                targetHarness: targetHarness,
                activation: activation
            )
        }

        let activation = NativePasteSmokeActivationDiagnostics(
            requestedProcessIdentifier: nil,
            activationSucceeded: nil,
            activatedApplicationBundleIdentifier: nil,
            activatedApplicationLocalizedName: nil,
            accessibilityWindowFocusSucceeded: nil
        )
        return try await pasteAndBuildOutput(
            request,
            preparedText: preparedText,
            options: options,
            pasteboard: pasteboard,
            previousString: previousString,
            accessibilityTrusted: permissionSnapshot.accessibilityTrusted,
            targetHarness: targetHarness,
            activation: activation
        )
    }

    @MainActor
    private static func pasteAndBuildOutput(
        _ request: NativePasteSmokeRequest,
        preparedText: String,
        options: PasteOutputOptions,
        pasteboard: NSPasteboard,
        previousString: String?,
        accessibilityTrusted: Bool,
        targetHarness: NativePasteTargetHarness?,
        activation: NativePasteSmokeActivationDiagnostics
    ) async throws -> NativePasteSmokeOutput {
        let pasteService: PasteService
        if let targetHarness {
            pasteService = PasteService(
                pasteboard: pasteboard,
                keyboardEventPoster: targetHarness,
                directTextInserter: targetHarness
            )
        } else {
            pasteService = PasteService(pasteboard: pasteboard)
        }
        try await pasteService.paste(preparedText, options: options)
        try await Task.sleep(
            for: .milliseconds(postPasteObservationDelayMilliseconds(for: request, activation: activation))
        )

        let clipboardAfter = pasteboard.string(forType: .string)
        let externalTargetSnapshot = targetHarness == nil
            ? focusedTextSnapshot(processIdentifier: activation.requestedProcessIdentifier)
            : nil
        let targetText = targetHarness?.text ?? externalTargetSnapshot?.text
        let targetDiagnostics = targetHarness?.diagnostics()
        let targetMatchesPreparedText = targetText == preparedText
        targetHarness?.close()
        let output = NativePasteSmokeOutput(
            success: true,
            requestedText: request.text,
            preparedText: preparedText,
            pasteMethod: request.pasteMethod.rawValue,
            effectivePasteMethod: options.pasteMethod.macOSCompatible.rawValue,
            clipboardHandling: request.clipboardHandling.rawValue,
            pasteDelayMilliseconds: request.pasteDelayMilliseconds,
            startDelayMilliseconds: request.startDelayMilliseconds,
            appendTrailingSpace: request.appendTrailingSpace,
            autoSubmitKey: request.autoSubmitKey?.rawValue,
            accessibilityTrusted: accessibilityTrusted,
            eventDispatchRequired: options.pasteMethod.macOSCompatible != .none,
            hadClipboardBefore: previousString != nil,
            clipboardRestored: clipboardAfter == previousString,
            clipboardAfterEqualsPreparedText: clipboardAfter == preparedText,
            clipboardAfterLength: clipboardAfter?.count,
            targetWindow: request.targetWindow,
            targetText: targetText,
            targetMatchesPreparedText: targetMatchesPreparedText,
            targetApplicationActive: targetDiagnostics?.applicationActive,
            targetWindowKey: targetDiagnostics?.windowKey,
            targetFirstResponderClass: targetDiagnostics?.firstResponderClass ?? externalTargetSnapshot?.role,
            targetInsertionDriver: targetHarness?.insertionDriver ?? externalTargetSnapshot?.insertionDriver,
            activationRequestedProcessIdentifier: activation.requestedProcessIdentifier,
            activationSucceeded: activation.activationSucceeded,
            activatedApplicationBundleIdentifier: activation.activatedApplicationBundleIdentifier,
            activatedApplicationLocalizedName: activation.activatedApplicationLocalizedName,
            accessibilityWindowFocusSucceeded: activation.accessibilityWindowFocusSucceeded,
            frontmostApplicationProcessIdentifier: activation.frontmostProcessIdentifier,
            frontmostApplicationBundleIdentifier: activation.frontmostBundleIdentifier,
            frontmostApplicationLocalizedName: activation.frontmostLocalizedName
        )

        if request.targetWindow, targetMatchesPreparedText == false {
            throw NativePasteSmokeError.targetWindowInsertionFailed(
                applicationActive: targetDiagnostics?.applicationActive,
                windowKey: targetDiagnostics?.windowKey,
                firstResponderClass: targetDiagnostics?.firstResponderClass
            )
        }

        return output
    }

    private static func postPasteObservationDelayMilliseconds(
        for request: NativePasteSmokeRequest,
        activation: NativePasteSmokeActivationDiagnostics
    ) -> Int {
        if request.targetWindow {
            return 1_000
        }
        if activation.requestedProcessIdentifier != nil {
            return max(300, request.pasteDelayMilliseconds)
        }
        return 0
    }

    @MainActor
    private static func activateTargetApplication(
        processIdentifier: Int32?
    ) -> NativePasteSmokeActivationDiagnostics {
        guard let processIdentifier else {
            return NativePasteSmokeActivationDiagnostics(
                requestedProcessIdentifier: nil,
                activationSucceeded: nil,
                activatedApplicationBundleIdentifier: nil,
                activatedApplicationLocalizedName: nil,
                accessibilityWindowFocusSucceeded: nil
            )
        }

        let application = NSRunningApplication(processIdentifier: processIdentifier)
        var activationOptions: NSApplication.ActivationOptions = [.activateAllWindows]
        if #unavailable(macOS 14) {
            activationOptions.insert(.activateIgnoringOtherApps)
        }
        let activationSucceeded = application?.activate(options: activationOptions) ?? false

        return NativePasteSmokeActivationDiagnostics(
            requestedProcessIdentifier: processIdentifier,
            activationSucceeded: activationSucceeded,
            activatedApplicationBundleIdentifier: application?.bundleIdentifier,
            activatedApplicationLocalizedName: application?.localizedName,
            accessibilityWindowFocusSucceeded: focusApplicationWindow(processIdentifier: processIdentifier)
        )
    }

    private static func focusedTextSnapshot(processIdentifier: Int32?) -> NativePasteAXFocusedTextSnapshot? {
        guard let processIdentifier else {
            return nil
        }

        let applicationElement = AXUIElementCreateApplication(processIdentifier)
        var focusedValue: CFTypeRef?
        guard AXUIElementCopyAttributeValue(
            applicationElement,
            kAXFocusedUIElementAttribute as CFString,
            &focusedValue
        ) == .success,
              let focusedValue,
              CFGetTypeID(focusedValue) == AXUIElementGetTypeID()
        else {
            return nil
        }

        let focusedElement = unsafeDowncast(focusedValue, to: AXUIElement.self)
        return NativePasteAXFocusedTextSnapshot(
            role: accessibilityStringAttribute(kAXRoleAttribute, from: focusedElement),
            text: accessibilityTextValue(from: focusedElement)
        )
    }

    private static func accessibilityStringAttribute(
        _ attribute: String,
        from element: AXUIElement
    ) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success else {
            return nil
        }
        return value as? String
    }

    private static func accessibilityTextValue(from element: AXUIElement) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &value) == .success,
              let value
        else {
            return nil
        }

        if let text = value as? String {
            return text
        }
        if let attributedText = value as? NSAttributedString {
            return attributedText.string
        }
        return nil
    }

    private static func focusApplicationWindow(processIdentifier: Int32) -> Bool {
        let applicationElement = AXUIElementCreateApplication(processIdentifier)
        var windowsValue: CFTypeRef?
        guard AXUIElementCopyAttributeValue(
            applicationElement,
            kAXWindowsAttribute as CFString,
            &windowsValue
        ) == .success,
              let windows = windowsValue as? [AXUIElement],
              let window = windows.first
        else {
            return false
        }

        let raiseResult = AXUIElementPerformAction(window, kAXRaiseAction as CFString)
        let focusResult = AXUIElementSetAttributeValue(
            applicationElement,
            kAXFocusedWindowAttribute as CFString,
            window
        )
        return raiseResult == .success || focusResult == .success
    }

    private static func settings(for request: NativePasteSmokeRequest) -> AppSettings {
        var settings = AppSettings.defaults
        settings.pasteMethod = request.pasteMethod
        settings.clipboardHandling = request.clipboardHandling
        settings.pasteDelayMilliseconds = request.pasteDelayMilliseconds
        settings.appendTrailingSpace = request.appendTrailingSpace
        if let autoSubmitKey = request.autoSubmitKey {
            settings.autoSubmitAfterPaste = true
            settings.autoSubmitKey = autoSubmitKey
        } else {
            settings.autoSubmitAfterPaste = false
        }
        return settings
    }

    private static func write(_ output: String, to path: String) throws {
        let outputURL = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try output.write(to: outputURL, atomically: true, encoding: .utf8)
    }

    private static func failureOutput(for error: Error) throws -> String {
        let output = NativePasteSmokeFailureOutput(
            success: false,
            errorDescription: error.localizedDescription,
            accessibilityTrusted: PermissionService().snapshot().accessibilityTrusted
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

struct NativePasteSmokeOutput: Encodable {
    var success: Bool
    var requestedText: String
    var preparedText: String
    var pasteMethod: String
    var effectivePasteMethod: String
    var clipboardHandling: String
    var pasteDelayMilliseconds: Int
    var startDelayMilliseconds: Int
    var appendTrailingSpace: Bool
    var autoSubmitKey: String?
    var accessibilityTrusted: Bool
    var eventDispatchRequired: Bool
    var hadClipboardBefore: Bool
    var clipboardRestored: Bool
    var clipboardAfterEqualsPreparedText: Bool
    var clipboardAfterLength: Int?
    var targetWindow: Bool
    var targetText: String?
    var targetMatchesPreparedText: Bool
    var targetApplicationActive: Bool?
    var targetWindowKey: Bool?
    var targetFirstResponderClass: String?
    var targetInsertionDriver: String?
    var activationRequestedProcessIdentifier: Int32?
    var activationSucceeded: Bool?
    var activatedApplicationBundleIdentifier: String?
    var activatedApplicationLocalizedName: String?
    var accessibilityWindowFocusSucceeded: Bool?
    var frontmostApplicationProcessIdentifier: Int32?
    var frontmostApplicationBundleIdentifier: String?
    var frontmostApplicationLocalizedName: String?
}

private struct NativePasteSmokeFailureOutput: Encodable {
    var success: Bool
    var errorDescription: String
    var accessibilityTrusted: Bool
}

private final class NativePasteSmokeActivationDiagnostics {
    let requestedProcessIdentifier: Int32?
    let activationSucceeded: Bool?
    let activatedApplicationBundleIdentifier: String?
    let activatedApplicationLocalizedName: String?
    let accessibilityWindowFocusSucceeded: Bool?
    private(set) var frontmostProcessIdentifier: Int32?
    private(set) var frontmostBundleIdentifier: String?
    private(set) var frontmostLocalizedName: String?

    init(
        requestedProcessIdentifier: Int32?,
        activationSucceeded: Bool?,
        activatedApplicationBundleIdentifier: String?,
        activatedApplicationLocalizedName: String?,
        accessibilityWindowFocusSucceeded: Bool?
    ) {
        self.requestedProcessIdentifier = requestedProcessIdentifier
        self.activationSucceeded = activationSucceeded
        self.activatedApplicationBundleIdentifier = activatedApplicationBundleIdentifier
        self.activatedApplicationLocalizedName = activatedApplicationLocalizedName
        self.accessibilityWindowFocusSucceeded = accessibilityWindowFocusSucceeded
    }

    @MainActor
    func observeFrontmostApplication() {
        frontmostProcessIdentifier = NSWorkspace.shared.frontmostApplication?.processIdentifier
        frontmostBundleIdentifier = NSWorkspace.shared.frontmostApplication?.bundleIdentifier
        frontmostLocalizedName = NSWorkspace.shared.frontmostApplication?.localizedName
    }
}

@MainActor
private final class NativePasteTargetHarness: KeyboardEventPosting, DirectTextInserting {
    private let window: NSWindow
    private let textView: NSTextView
    let insertionDriver = "appkit_harness"

    init() {
        Self.installEditMenuIfNeeded()

        textView = NSTextView(frame: NSRect(x: 0, y: 0, width: 480, height: 160))
        textView.isEditable = true
        textView.isSelectable = true
        textView.string = ""
        textView.font = NSFont.systemFont(ofSize: 16)

        let scrollView = NSScrollView(frame: NSRect(x: 0, y: 0, width: 480, height: 160))
        scrollView.hasVerticalScroller = true
        scrollView.documentView = textView

        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 500, height: 220),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        window.title = "Murmur Paste Smoke"
        window.contentView = scrollView
        window.initialFirstResponder = textView
        window.center()
    }

    var text: String {
        textView.string
    }

    func diagnostics() -> NativePasteTargetDiagnostics {
        NativePasteTargetDiagnostics(
            applicationActive: NSApp.isActive,
            windowKey: window.isKeyWindow,
            firstResponderClass: window.firstResponder.map { String(describing: type(of: $0)) }
        )
    }

    func show() {
        NSApp.setActivationPolicy(.regular)
        NSApp.unhide(nil)
        window.makeKeyAndOrderFront(nil)
        window.orderFrontRegardless()
        window.makeKey()
        NSRunningApplication.current.activate(options: [.activateAllWindows])
        NSApp.activate(ignoringOtherApps: true)
        window.makeFirstResponder(textView)
    }

    func waitUntilFocused(timeoutMilliseconds: Int) async {
        let deadline = Date().addingTimeInterval(TimeInterval(timeoutMilliseconds) / 1_000)
        while Date() < deadline {
            show()
            if NSApp.isActive, window.isKeyWindow {
                return
            }
            try? await Task.sleep(for: .milliseconds(100))
        }
        show()
    }

    func close() {
        window.close()
    }

    func insertText(_ text: String) throws {
        textView.insertText(text, replacementRange: textView.selectedRange())
    }

    func postShortcut(virtualKey: CGKeyCode, flags: CGEventFlags) throws {
        if virtualKey == Self.vKeyCode,
           flags.contains(.maskCommand) {
            textView.paste(nil)
            return
        }

        if virtualKey == Self.returnKeyCode {
            return
        }

        throw PasteServiceError.eventCreationFailed
    }

    func postText(_ text: String) throws {
        try insertText(text)
    }

    private static let vKeyCode: CGKeyCode = 9
    private static let returnKeyCode: CGKeyCode = 36

    private static func installEditMenuIfNeeded() {
        let mainMenu = NSApp.mainMenu ?? NSMenu()
        if NSApp.mainMenu == nil {
            NSApp.mainMenu = mainMenu
        }

        if mainMenu.items.contains(where: { $0.submenu?.title == "Edit" }) {
            return
        }

        let editMenu = NSMenu(title: "Edit")
        editMenu.addItem(
            NSMenuItem(
                title: "Paste",
                action: #selector(NSText.paste(_:)),
                keyEquivalent: "v"
            )
        )

        let editMenuItem = NSMenuItem(title: "Edit", action: nil, keyEquivalent: "")
        editMenuItem.submenu = editMenu
        mainMenu.addItem(editMenuItem)
    }
}

private struct NativePasteAXFocusedTextSnapshot {
    var role: String?
    var text: String?

    var insertionDriver: String? {
        text == nil ? nil : "accessibility_focused_element"
    }
}

private struct NativePasteTargetDiagnostics {
    var applicationActive: Bool
    var windowKey: Bool
    var firstResponderClass: String?
}

private enum NativePasteSmokeError: LocalizedError {
    case accessibilityPermission
    case activatedTargetNotFrontmost(
        requestedProcessIdentifier: Int32,
        frontmostProcessIdentifier: Int32?,
        frontmostBundleIdentifier: String?,
        frontmostLocalizedName: String?
    )
    case targetWindowInsertionFailed(
        applicationActive: Bool?,
        windowKey: Bool?,
        firstResponderClass: String?
    )

    var errorDescription: String? {
        switch self {
        case .accessibilityPermission:
            "Accessibility permission is not granted; grant it before running --smoke-paste-text with a keyboard-event paste method."
        case let .activatedTargetNotFrontmost(
            requestedProcessIdentifier,
            frontmostProcessIdentifier,
            frontmostBundleIdentifier,
            frontmostLocalizedName
        ):
            "Requested paste target pid \(requestedProcessIdentifier) is not frontmost after activation (frontmostPid: \(frontmostProcessIdentifier.map(String.init) ?? "unknown"), frontmostBundle: \(frontmostBundleIdentifier ?? "unknown"), frontmostName: \(frontmostLocalizedName ?? "unknown"))."
        case let .targetWindowInsertionFailed(applicationActive, windowKey, firstResponderClass):
            "Paste target window did not receive the prepared text (applicationActive: \(applicationActive.map(String.init) ?? "unknown"), windowKey: \(windowKey.map(String.init) ?? "unknown"), firstResponder: \(firstResponderClass ?? "unknown"))."
        }
    }
}
