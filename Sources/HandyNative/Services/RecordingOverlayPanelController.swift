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
    private var visibilityGeneration = 0

    func show(state: RecordingOverlayState, palette: HandyThemePalette, position: OverlayPosition = .bottom) {
        visibilityGeneration += 1
        viewModel.state = state
        hideTask?.cancel()
        hideTask = nil

        let panel = ensurePanel(palette: palette)
        panel.setFrameOrigin(Self.overlayOrigin(for: panel.frame.size, position: position))
        let shouldFadeIn = panel.isVisible == false
        if shouldFadeIn {
            panel.alphaValue = 0
        } else {
            panel.alphaValue = 1
        }
        if NSApp.isHidden {
            NSApp.unhideWithoutActivation()
        }
        panel.orderFrontRegardless()

        if shouldFadeIn {
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.3
                panel.animator().alphaValue = 1
            }
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
            visibilityGeneration += 1
            let generation = visibilityGeneration
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.3
                panel.animator().alphaValue = 0
            }

            hideTask = Task { [weak self, weak panel] in
                do {
                    try await Task.sleep(for: .milliseconds(300))
                } catch {
                    return
                }
                await MainActor.run {
                    guard let self,
                          self.visibilityGeneration == generation,
                          self.panel === panel
                    else {
                        return
                    }
                    panel?.orderOut(nil)
                    self.hideTask = nil
                }
            }
        } else {
            visibilityGeneration += 1
            hideTask?.cancel()
            hideTask = nil
            panel.alphaValue = 0
            panel.orderOut(nil)
        }
    }

    func diagnostics(expectedState: RecordingOverlayState? = nil) -> RecordingOverlayPanelDiagnostics {
        guard let panel else {
            return RecordingOverlayPanelDiagnostics(
                success: false,
                state: viewModel.state.diagnosticName,
                expectedState: expectedState?.diagnosticName,
                isVisible: false,
                alphaValue: 0,
                level: 0,
                frame: .zero,
                frameWithinVisibleFrame: false,
                hasStatusBarLevel: false,
                canBecomeKey: false,
                canBecomeMain: false,
                collectionBehaviorRawValue: 0,
                matchesExpectedState: expectedState == nil
            )
        }

        let matchesExpectedState = expectedState.map { $0 == viewModel.state } ?? true
        let frameWithinVisibleFrame = Self.frameIsWithinVisibleFrame(panel.frame)
        let hasStatusBarLevel = panel.level == .statusBar
        let alphaValue = Double(panel.alphaValue)
        let isVisible = panel.isVisible

        return RecordingOverlayPanelDiagnostics(
            success: isVisible &&
                alphaValue >= 0.95 &&
                hasStatusBarLevel &&
                frameWithinVisibleFrame &&
                matchesExpectedState,
            state: viewModel.state.diagnosticName,
            expectedState: expectedState?.diagnosticName,
            isVisible: isVisible,
            alphaValue: alphaValue,
            level: Int(panel.level.rawValue),
            frame: RecordingOverlayPanelFrame(panel.frame),
            frameWithinVisibleFrame: frameWithinVisibleFrame,
            hasStatusBarLevel: hasStatusBarLevel,
            canBecomeKey: panel.canBecomeKey,
            canBecomeMain: panel.canBecomeMain,
            collectionBehaviorRawValue: panel.collectionBehavior.rawValue,
            matchesExpectedState: matchesExpectedState
        )
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
        panel.canHide = false
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

    private static func frameIsWithinVisibleFrame(_ frame: CGRect, screens: [NSScreen] = NSScreen.screens) -> Bool {
        screens.contains { screen in
            screen.visibleFrame.contains(frame)
        }
    }
}

final class RecordingOverlayPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

struct RecordingOverlayPanelDiagnostics: Codable, Equatable {
    var success: Bool
    var state: String
    var expectedState: String?
    var isVisible: Bool
    var alphaValue: Double
    var level: Int
    var frame: RecordingOverlayPanelFrame
    var frameWithinVisibleFrame: Bool
    var hasStatusBarLevel: Bool
    var canBecomeKey: Bool
    var canBecomeMain: Bool
    var collectionBehaviorRawValue: NSWindow.CollectionBehavior.RawValue
    var matchesExpectedState: Bool
}

struct RecordingOverlayPanelFrame: Codable, Equatable {
    var x: Double
    var y: Double
    var width: Double
    var height: Double

    static let zero = RecordingOverlayPanelFrame(CGRect.zero)

    init(_ frame: CGRect) {
        x = Double(frame.origin.x)
        y = Double(frame.origin.y)
        width = Double(frame.width)
        height = Double(frame.height)
    }
}

private extension RecordingOverlayState {
    var diagnosticName: String {
        switch self {
        case .recording:
            "recording"
        case .transcribing:
            "transcribing"
        case .processing:
            "processing"
        }
    }
}
