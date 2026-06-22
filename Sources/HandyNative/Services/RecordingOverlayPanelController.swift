import AppKit
import SwiftUI

@MainActor
final class RecordingOverlayViewModel: ObservableObject {
    @Published var state: RecordingOverlayState = .recording
    @Published var levels: [Float] = Array(repeating: 0, count: 9)
}

enum RecordingOverlayLevelMapper {
    static let barCount = 9

    static func levels(from level: Float, count: Int = barCount) -> [Float] {
        let clampedLevel = max(0, min(1, level))
        return (0..<count).map { index in
            let phase = Float(index) / Float(max(1, count - 1))
            let shape = 0.55 + 0.45 * sin((phase * .pi) + 0.35)
            return max(0, min(1, clampedLevel * shape))
        }
    }
}

@MainActor
final class RecordingOverlayPanelController {
    var onCancel: (() -> Void)?

    private let viewModel = RecordingOverlayViewModel()
    private var panel: RecordingOverlayPanel?
    private var hideTask: Task<Void, Never>?

    func show(state: RecordingOverlayState, palette: HandyThemePalette, position: OverlayPosition = .bottom) {
        viewModel.state = state
        hideTask?.cancel()

        let panel = ensurePanel(palette: palette)
        panel.setFrameOrigin(Self.overlayOrigin(for: panel.frame.size, position: position))
        panel.alphaValue = 0
        panel.orderFrontRegardless()

        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.3
            panel.animator().alphaValue = 1
        }
    }

    func updateLevels(_ levels: [Float]) {
        viewModel.levels = levels
    }

    func hide(animated: Bool = true) {
        guard let panel, panel.isVisible else {
            return
        }

        hideTask?.cancel()
        if animated {
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.3
                panel.animator().alphaValue = 0
            }

            hideTask = Task { [weak panel] in
                try? await Task.sleep(for: .milliseconds(300))
                await MainActor.run {
                    panel?.orderOut(nil)
                }
            }
        } else {
            panel.alphaValue = 0
            panel.orderOut(nil)
        }
    }

    private func ensurePanel(palette: HandyThemePalette) -> RecordingOverlayPanel {
        if let panel {
            panel.contentView = makeContentView(palette: palette)
            return panel
        }

        let contentSize = CGSize(width: 172, height: 36)
        let panel = RecordingOverlayPanel(
            contentRect: CGRect(origin: Self.overlayOrigin(for: contentSize), size: contentSize),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.isFloatingPanel = true
        panel.hidesOnDeactivate = false
        panel.level = .statusBar
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.ignoresMouseEvents = false
        panel.contentView = makeContentView(palette: palette)

        self.panel = panel
        return panel
    }

    private func makeContentView(palette: HandyThemePalette) -> NSView {
        NSHostingView(
            rootView: RecordingOverlayView(viewModel: viewModel) { [weak self] in
                self?.onCancel?()
            }
            .environment(\.handyTheme, palette)
        )
    }

    static func overlayOrigin(
        for size: CGSize,
        position: OverlayPosition = .bottom,
        mouseLocation: CGPoint = NSEvent.mouseLocation,
        screens: [NSScreen] = NSScreen.screens
    ) -> CGPoint {
        let screen = screens.first { $0.frame.contains(mouseLocation) } ?? NSScreen.main ?? screens.first
        guard let screen else {
            return .zero
        }
        let frame = screen.visibleFrame

        return CGPoint(
            x: frame.minX + (frame.width - size.width) / 2,
            y: position == .top ? frame.maxY - size.height - 15 : frame.minY + 15
        )
    }
}

final class RecordingOverlayPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}
