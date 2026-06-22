import AppKit
import SwiftUI

struct ShortcutBindingField: View {
    var placeholder: String
    var currentBinding: String
    var width: CGFloat = 170
    var reservedBindings: [String] = []
    var onCommit: (String) -> Void

    @State private var isCapturing = false
    @State private var validationMessage: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Button {
                validationMessage = nil
                isCapturing.toggle()
            } label: {
                Text(buttonTitle)
                    .frame(width: max(44, width - 28), alignment: .center)
            }
            .buttonStyle(HandyButtonStyle(variant: isCapturing ? .soft : .secondary))
            .help(helpText)
            .overlay {
                if isCapturing {
                    ShortcutKeyboardCaptureView(
                        isCapturing: $isCapturing,
                        onCapture: handleCapturedShortcut,
                        onCancel: {
                            isCapturing = false
                        }
                    )
                }
            }

            if let validationMessage {
                Text(validationMessage)
                    .font(HandyDesign.font(size: 11, weight: .medium))
                    .foregroundStyle(.red)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(width: width, alignment: .leading)
        .onChange(of: currentBinding) {
            validationMessage = nil
            isCapturing = false
        }
    }

    private var buttonTitle: String {
        if isCapturing {
            return "Press shortcut..."
        }

        if currentBinding.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return ShortcutBinding.displayName(for: placeholder)
        }

        return ShortcutBinding.displayName(for: currentBinding)
    }

    private var helpText: String {
        isCapturing
            ? "Press a shortcut. Use at least one modifier for letter, number, Space, Return, Tab, or Delete shortcuts."
            : "Click to change this keyboard shortcut."
    }

    private func handleCapturedShortcut(_ event: NSEvent) {
        guard let binding = GlobalShortcutDescriptor.bindingString(
            keyCode: CGKeyCode(event.keyCode),
            modifierFlags: event.modifierFlags
        ) else {
            validationMessage = "Unsupported shortcut"
            return
        }

        guard reservedBindings.contains(where: { GlobalShortcutDescriptor.bindingsConflict($0, binding) }) == false else {
            validationMessage = "Already used"
            return
        }

        validationMessage = nil
        isCapturing = false

        if GlobalShortcutDescriptor.bindingsConflict(currentBinding, binding) == false {
            onCommit(binding)
        }
    }
}

private struct ShortcutKeyboardCaptureView: NSViewRepresentable {
    @Binding var isCapturing: Bool
    var onCapture: (NSEvent) -> Void
    var onCancel: () -> Void

    func makeNSView(context: Context) -> ShortcutKeyboardCaptureNSView {
        let view = ShortcutKeyboardCaptureNSView()
        view.onCapture = onCapture
        view.onCancel = onCancel
        return view
    }

    func updateNSView(_ nsView: ShortcutKeyboardCaptureNSView, context: Context) {
        nsView.onCapture = onCapture
        nsView.onCancel = onCancel

        guard isCapturing else {
            if nsView.window?.firstResponder === nsView {
                nsView.window?.makeFirstResponder(nil)
            }
            return
        }

        DispatchQueue.main.async {
            nsView.window?.makeFirstResponder(nsView)
        }
    }
}

private final class ShortcutKeyboardCaptureNSView: NSView {
    var onCapture: ((NSEvent) -> Void)?
    var onCancel: (() -> Void)?

    override var acceptsFirstResponder: Bool {
        true
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        DispatchQueue.main.async {
            self.window?.makeFirstResponder(self)
        }
    }

    override func keyDown(with event: NSEvent) {
        onCapture?(event)
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard event.type == .keyDown else {
            return false
        }

        onCapture?(event)
        return true
    }

    override func mouseDown(with event: NSEvent) {
        onCancel?()
    }

    override func resignFirstResponder() -> Bool {
        onCancel?()
        return true
    }
}
