import AppKit
import XCTest
@testable import HandyNative

final class RecordingOverlayTests: XCTestCase {
    func testLevelMapperProducesNineClampedBars() {
        let levels = RecordingOverlayLevelMapper.levels(from: 2)

        XCTAssertEqual(levels.count, 9)
        XCTAssertTrue(levels.allSatisfy { $0 >= 0 && $0 <= 1 })
        XCTAssertGreaterThan(levels.max() ?? 0, 0)
    }

    func testLevelMapperKeepsSilenceAtZero() {
        XCTAssertEqual(RecordingOverlayLevelMapper.levels(from: 0), Array(repeating: 0, count: 9))
    }

    @MainActor
    func testCancelledAnimatedHideDoesNotOrderOutNewlyShownPanel() async throws {
        try XCTSkipIf(NSScreen.main == nil, "Recording overlay panel tests require a visible screen.")
        let controller = RecordingOverlayPanelController()
        defer {
            controller.hide(animated: false)
        }

        controller.show(state: .recording, palette: AppTheme.pink.palette)
        controller.hide(animated: true)
        controller.show(state: .recording, palette: AppTheme.pink.palette)

        try await Task.sleep(for: .milliseconds(400))
        let diagnostics = controller.diagnostics(expectedState: .recording)

        XCTAssertTrue(diagnostics.isVisible)
        XCTAssertGreaterThanOrEqual(diagnostics.alphaValue, 0.95)
        XCTAssertTrue(diagnostics.isOnActiveSpace)
        XCTAssertTrue(diagnostics.matchesExpectedState)
    }

    @MainActor
    func testVisibleStateTransitionDoesNotResetPanelAlphaToZero() async throws {
        try XCTSkipIf(NSScreen.main == nil, "Recording overlay panel tests require a visible screen.")
        let controller = RecordingOverlayPanelController()
        defer {
            controller.hide(animated: false)
        }

        controller.show(state: .recording, palette: AppTheme.pink.palette)
        try await Task.sleep(for: .milliseconds(350))
        controller.show(state: .transcribing, palette: AppTheme.pink.palette)
        let diagnostics = controller.diagnostics(expectedState: .transcribing)

        XCTAssertTrue(diagnostics.isVisible)
        XCTAssertGreaterThanOrEqual(diagnostics.alphaValue, 0.95)
        XCTAssertTrue(diagnostics.isOnActiveSpace)
        XCTAssertTrue(diagnostics.matchesExpectedState)
    }

    @MainActor
    func testPanelDiagnosticsCaptureVisibilityContract() async throws {
        try XCTSkipIf(NSScreen.main == nil, "Recording overlay panel tests require a visible screen.")
        let controller = RecordingOverlayPanelController()
        defer {
            controller.hide(animated: false)
        }

        controller.show(state: .recording, palette: AppTheme.pink.palette)
        try await Task.sleep(for: .milliseconds(350))
        let diagnostics = controller.diagnostics(expectedState: .recording)
        let collectionBehavior = NSWindow.CollectionBehavior(rawValue: diagnostics.collectionBehaviorRawValue)

        XCTAssertTrue(diagnostics.success)
        XCTAssertTrue(diagnostics.hasStatusBarLevel)
        XCTAssertEqual(diagnostics.level, Int(NSWindow.Level.statusBar.rawValue))
        XCTAssertTrue(collectionBehavior.contains(.moveToActiveSpace))
        XCTAssertTrue(collectionBehavior.contains(.fullScreenAuxiliary))
        XCTAssertTrue(collectionBehavior.contains(.transient))
        XCTAssertFalse(collectionBehavior.contains(.canJoinAllSpaces))
        XCTAssertFalse(collectionBehavior.contains(.stationary))
        XCTAssertTrue(diagnostics.isOnActiveSpace)
        XCTAssertFalse(diagnostics.canBecomeKey)
        XCTAssertFalse(diagnostics.canBecomeMain)
        XCTAssertTrue(diagnostics.frameWithinVisibleFrame)
        XCTAssertTrue(diagnostics.visualSnapshot.success)
        XCTAssertGreaterThanOrEqual(diagnostics.visualSnapshot.pixelWidth, 172)
        XCTAssertGreaterThanOrEqual(diagnostics.visualSnapshot.pixelHeight, 36)
        XCTAssertGreaterThan(diagnostics.visualSnapshot.nonTransparentPixelRatio, 0.25)
        XCTAssertGreaterThan(diagnostics.visualSnapshot.highlightedPixelRatio, 0.005)
    }

    @MainActor
    func testActiveSpaceRefreshKeepsPanelVisibleWithoutResettingState() async throws {
        try XCTSkipIf(NSScreen.main == nil, "Recording overlay panel tests require a visible screen.")
        let controller = RecordingOverlayPanelController()
        defer {
            controller.hide(animated: false)
        }

        controller.show(state: .recording, palette: AppTheme.pink.palette)
        try await Task.sleep(for: .milliseconds(350))
        controller.refreshVisiblePanelForActiveSpace()
        let diagnostics = controller.diagnostics(expectedState: .recording)

        XCTAssertTrue(diagnostics.success)
        XCTAssertTrue(diagnostics.isVisible)
        XCTAssertTrue(diagnostics.isOnActiveSpace)
        XCTAssertGreaterThanOrEqual(diagnostics.alphaValue, 0.95)
    }
}
