import AppKit
import Foundation

enum NativeExternalPasteTargetSmokeRunner {
    @MainActor
    static func runSynchronouslyAndExit(_ request: NativeExternalPasteTargetSmokeRequest) -> Never {
        let application = NSApplication.shared
        application.setActivationPolicy(.regular)
        application.finishLaunching()

        let target = NativeExternalPasteTarget(request: request)
        Task { @MainActor in
            await target.run()
        }

        application.run()
        exit(1)
    }
}

@MainActor
private final class NativeExternalPasteTarget {
    private let request: NativeExternalPasteTargetSmokeRequest
    private let window: NSWindow
    private let textView: NSTextView

    init(request: NativeExternalPasteTargetSmokeRequest) {
        self.request = request

        Self.installEditMenuIfNeeded()

        textView = NSTextView(frame: NSRect(x: 0, y: 0, width: 520, height: 180))
        textView.isEditable = true
        textView.isSelectable = true
        textView.string = ""
        textView.font = NSFont.systemFont(ofSize: 16)

        let scrollView = NSScrollView(frame: NSRect(x: 0, y: 0, width: 520, height: 180))
        scrollView.hasVerticalScroller = true
        scrollView.documentView = textView

        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 540, height: 240),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        window.title = "Handy External Paste Target"
        window.contentView = scrollView
        window.initialFirstResponder = textView
        window.center()
    }

    func run() async {
        do {
            show()
            try writeReadyIfNeeded()
            let deadline = Date().addingTimeInterval(TimeInterval(request.durationMilliseconds) / 1_000)

            while Date() < deadline {
                show()
                let output = currentOutput()
                try write(output)
                if output.matchedExpectedText == true {
                    finish(output, status: 0)
                }
                try await Task.sleep(for: .milliseconds(100))
            }

            let output = currentOutput()
            try write(output)
            finish(output, status: output.expectedText == nil ? 0 : 1)
        } catch {
            FileHandle.standardError.writeLine(error.localizedDescription)
            exit(1)
        }
    }

    private func show() {
        NSApp.setActivationPolicy(.regular)
        NSApp.unhide(nil)
        window.makeKeyAndOrderFront(nil)
        window.orderFrontRegardless()
        window.makeKey()
        NSRunningApplication.current.activate(options: [.activateAllWindows])
        NSApp.activate(ignoringOtherApps: true)
        window.makeFirstResponder(textView)
    }

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

    private func currentOutput() -> NativeExternalPasteTargetSmokeOutput {
        let currentText = textView.string
        return NativeExternalPasteTargetSmokeOutput(
            success: request.expectedText.map { currentText == $0 } ?? true,
            outputPath: request.outputPath,
            readyPath: request.readyPath,
            durationMilliseconds: request.durationMilliseconds,
            text: currentText,
            expectedText: request.expectedText,
            matchedExpectedText: request.expectedText.map { currentText == $0 },
            applicationActive: NSApp.isActive,
            windowKey: window.isKeyWindow,
            firstResponderClass: window.firstResponder.map { String(describing: type(of: $0)) }
        )
    }

    private func writeReadyIfNeeded() throws {
        guard let readyPath = request.readyPath else {
            return
        }
        let readyURL = URL(fileURLWithPath: readyPath)
        try FileManager.default.createDirectory(
            at: readyURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try "ready".write(to: readyURL, atomically: true, encoding: .utf8)
    }

    private func write(_ output: NativeExternalPasteTargetSmokeOutput) throws {
        let outputURL = URL(fileURLWithPath: request.outputPath)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try outputString(output).write(to: outputURL, atomically: true, encoding: .utf8)
    }

    private func finish(_ output: NativeExternalPasteTargetSmokeOutput, status: Int32) -> Never {
        let outputText = outputString(output)
        FileHandle.standardOutput.writeLine(outputText)
        exit(status)
    }

    private func outputString(_ output: NativeExternalPasteTargetSmokeOutput) -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        guard let data = try? encoder.encode(output),
              let string = String(data: data, encoding: .utf8)
        else {
            return "{}"
        }
        return string
    }
}

struct NativeExternalPasteTargetSmokeOutput: Codable {
    var success: Bool
    var outputPath: String
    var readyPath: String?
    var durationMilliseconds: Int
    var text: String
    var expectedText: String?
    var matchedExpectedText: Bool?
    var applicationActive: Bool
    var windowKey: Bool
    var firstResponderClass: String?
}
