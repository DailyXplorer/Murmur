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
    nonisolated(unsafe) private var activeSpaceObserver: NSObjectProtocol?
    private var lastPosition: OverlayPosition = .bottom

    private static let overlayCollectionBehavior: NSWindow.CollectionBehavior = [
        .moveToActiveSpace,
        .fullScreenAuxiliary,
        .transient,
        .ignoresCycle,
    ]

    init() {
        activeSpaceObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.activeSpaceDidChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor in
                self?.refreshVisiblePanelForActiveSpace()
            }
        }
    }

    deinit {
        if let activeSpaceObserver {
            NSWorkspace.shared.notificationCenter.removeObserver(activeSpaceObserver)
        }
    }

    func show(state: RecordingOverlayState, palette: HandyThemePalette, position: OverlayPosition = .bottom) {
        visibilityGeneration += 1
        viewModel.state = state
        lastPosition = position
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
        panel.collectionBehavior = Self.overlayCollectionBehavior
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
                    self.panel = nil
                    self.hideTask = nil
                }
            }
        } else {
            visibilityGeneration += 1
            hideTask?.cancel()
            hideTask = nil
            panel.alphaValue = 0
            panel.orderOut(nil)
            self.panel = nil
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
                isOnActiveSpace: false,
                hasStatusBarLevel: false,
                canBecomeKey: false,
                canBecomeMain: false,
                collectionBehaviorRawValue: 0,
                visualSnapshot: .failure(reason: "missing-panel"),
                matchesExpectedState: expectedState == nil
            )
        }

        let matchesExpectedState = expectedState.map { $0 == viewModel.state } ?? true
        let frameWithinVisibleFrame = Self.frameIsWithinVisibleFrame(panel.frame)
        let isOnActiveSpace = panel.isOnActiveSpace
        let hasStatusBarLevel = panel.level == .statusBar
        let alphaValue = Double(panel.alphaValue)
        let isVisible = panel.isVisible
        let visualSnapshot = Self.visualSnapshotDiagnostics(for: panel)

        return RecordingOverlayPanelDiagnostics(
            success: isVisible &&
                alphaValue >= 0.95 &&
                hasStatusBarLevel &&
                frameWithinVisibleFrame &&
                isOnActiveSpace &&
                visualSnapshot.success &&
                matchesExpectedState,
            state: viewModel.state.diagnosticName,
            expectedState: expectedState?.diagnosticName,
            isVisible: isVisible,
            alphaValue: alphaValue,
            level: Int(panel.level.rawValue),
            frame: RecordingOverlayPanelFrame(panel.frame),
            frameWithinVisibleFrame: frameWithinVisibleFrame,
            isOnActiveSpace: isOnActiveSpace,
            hasStatusBarLevel: hasStatusBarLevel,
            canBecomeKey: panel.canBecomeKey,
            canBecomeMain: panel.canBecomeMain,
            collectionBehaviorRawValue: panel.collectionBehavior.rawValue,
            visualSnapshot: visualSnapshot,
            matchesExpectedState: matchesExpectedState
        )
    }

    func refreshVisiblePanelForActiveSpace() {
        guard let panel, panel.isVisible else {
            return
        }

        if NSApp.isHidden {
            NSApp.unhideWithoutActivation()
        }
        panel.collectionBehavior = Self.overlayCollectionBehavior
        panel.alphaValue = 1
        panel.setFrameOrigin(Self.overlayOrigin(for: panel.frame.size, position: lastPosition))
        panel.orderFrontRegardless()
    }

    func visualSnapshotPNGData() throws -> Data {
        guard let panel else {
            throw RecordingOverlayVisualSnapshotError.missingPanel
        }
        let bitmap = try Self.visualSnapshotBitmap(for: panel)
        guard let data = bitmap.representation(using: .png, properties: [:]) else {
            throw RecordingOverlayVisualSnapshotError.pngEncodingFailed
        }
        return data
    }

    private func ensurePanel(palette: HandyThemePalette) -> RecordingOverlayPanel {
        if let panel {
            panel.contentView = makeContentView(palette: palette)
            panel.collectionBehavior = Self.overlayCollectionBehavior
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
        panel.collectionBehavior = Self.overlayCollectionBehavior
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

    private static func visualSnapshotDiagnostics(for panel: RecordingOverlayPanel) -> RecordingOverlayVisualSnapshotDiagnostics {
        do {
            let bitmap = try visualSnapshotBitmap(for: panel)
            return analyzeVisualSnapshot(bitmap)
        } catch {
            return .failure(reason: error.localizedDescription)
        }
    }

    private static func visualSnapshotBitmap(for panel: RecordingOverlayPanel) throws -> NSBitmapImageRep {
        guard let contentView = panel.contentView else {
            throw RecordingOverlayVisualSnapshotError.missingContentView
        }

        contentView.layoutSubtreeIfNeeded()
        let bounds = contentView.bounds
        guard bounds.width > 0, bounds.height > 0 else {
            throw RecordingOverlayVisualSnapshotError.emptyBounds
        }
        guard let bitmap = contentView.bitmapImageRepForCachingDisplay(in: bounds) else {
            throw RecordingOverlayVisualSnapshotError.bitmapCreationFailed
        }
        bitmap.size = bounds.size
        contentView.cacheDisplay(in: bounds, to: bitmap)
        return bitmap
    }

    private static func analyzeVisualSnapshot(_ bitmap: NSBitmapImageRep) -> RecordingOverlayVisualSnapshotDiagnostics {
        let width = bitmap.pixelsWide
        let height = bitmap.pixelsHigh
        guard width > 0, height > 0 else {
            return .failure(reason: "empty-bitmap")
        }

        let step = max(1, Int(sqrt(Double(width * height) / 4_000.0)))
        var sampledPixelCount = 0
        var nonTransparentPixelCount = 0
        var highlightedPixelCount = 0
        var alphaTotal = 0.0
        var luminanceTotal = 0.0

        for y in stride(from: 0, to: height, by: step) {
            for x in stride(from: 0, to: width, by: step) {
                guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else {
                    continue
                }
                let alpha = Double(color.alphaComponent)
                let luminance = (0.2126 * Double(color.redComponent)) +
                    (0.7152 * Double(color.greenComponent)) +
                    (0.0722 * Double(color.blueComponent))

                sampledPixelCount += 1
                alphaTotal += alpha
                luminanceTotal += luminance

                if alpha > 0.05 {
                    nonTransparentPixelCount += 1
                }
                if alpha > 0.05, luminance > 0.08 {
                    highlightedPixelCount += 1
                }
            }
        }

        guard sampledPixelCount > 0 else {
            return .failure(reason: "no-sampled-pixels")
        }

        let sampled = Double(sampledPixelCount)
        let nonTransparentPixelRatio = Double(nonTransparentPixelCount) / sampled
        let highlightedPixelRatio = Double(highlightedPixelCount) / sampled
        let averageAlpha = alphaTotal / sampled
        let averageLuminance = luminanceTotal / sampled

        return RecordingOverlayVisualSnapshotDiagnostics(
            success: nonTransparentPixelRatio >= 0.25 &&
                highlightedPixelRatio >= 0.005 &&
                averageAlpha >= 0.2,
            pixelWidth: width,
            pixelHeight: height,
            sampledPixelCount: sampledPixelCount,
            nonTransparentPixelRatio: nonTransparentPixelRatio,
            highlightedPixelRatio: highlightedPixelRatio,
            averageAlpha: averageAlpha,
            averageLuminance: averageLuminance,
            failureReason: nil
        )
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
    var isOnActiveSpace: Bool
    var hasStatusBarLevel: Bool
    var canBecomeKey: Bool
    var canBecomeMain: Bool
    var collectionBehaviorRawValue: NSWindow.CollectionBehavior.RawValue
    var visualSnapshot: RecordingOverlayVisualSnapshotDiagnostics
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

struct RecordingOverlayVisualSnapshotDiagnostics: Codable, Equatable {
    var success: Bool
    var pixelWidth: Int
    var pixelHeight: Int
    var sampledPixelCount: Int
    var nonTransparentPixelRatio: Double
    var highlightedPixelRatio: Double
    var averageAlpha: Double
    var averageLuminance: Double
    var failureReason: String?

    static func failure(reason: String) -> RecordingOverlayVisualSnapshotDiagnostics {
        RecordingOverlayVisualSnapshotDiagnostics(
            success: false,
            pixelWidth: 0,
            pixelHeight: 0,
            sampledPixelCount: 0,
            nonTransparentPixelRatio: 0,
            highlightedPixelRatio: 0,
            averageAlpha: 0,
            averageLuminance: 0,
            failureReason: reason
        )
    }
}

private enum RecordingOverlayVisualSnapshotError: LocalizedError {
    case missingPanel
    case missingContentView
    case emptyBounds
    case bitmapCreationFailed
    case pngEncodingFailed

    var errorDescription: String? {
        switch self {
        case .missingPanel:
            "missing-panel"
        case .missingContentView:
            "missing-content-view"
        case .emptyBounds:
            "empty-bounds"
        case .bitmapCreationFailed:
            "bitmap-creation-failed"
        case .pngEncodingFailed:
            "png-encoding-failed"
        }
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
