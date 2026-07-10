import AppKit
import XCTest
@testable import MurmurNative

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
    func testShowTransientOutcomeShowsFailureStateThenAutoHides() async throws {
        try XCTSkipIf(NSScreen.main == nil, "Recording overlay panel tests require a visible screen.")
        let controller = RecordingOverlayPanelController()
        defer {
            controller.hide(animated: false)
        }

        controller.showTransientOutcome(
            state: .failure(message: "Transcription failed"),
            palette: AppTheme.pink.palette,
            position: .bottom,
            dismissAfterMilliseconds: 50
        )

        let shownDiagnostics = controller.diagnostics(expectedState: .failure(message: "Transcription failed"))
        XCTAssertTrue(shownDiagnostics.isVisible)
        XCTAssertTrue(shownDiagnostics.matchesExpectedState)
        XCTAssertEqual(shownDiagnostics.state, "failure")

        let hidden = try await Self.poll(timeoutMilliseconds: 2_000) {
            controller.diagnostics().isVisible == false
        }
        XCTAssertTrue(hidden, "Panel should auto-hide after the transient dismiss interval plus fade.")
    }

    @MainActor
    func testShowDuringTransientWindowCancelsAutoHide() async throws {
        try XCTSkipIf(NSScreen.main == nil, "Recording overlay panel tests require a visible screen.")
        let controller = RecordingOverlayPanelController()
        defer {
            controller.hide(animated: false)
        }

        controller.showTransientOutcome(
            state: .notice(message: "No speech detected"),
            palette: AppTheme.pink.palette,
            position: .bottom,
            dismissAfterMilliseconds: 50
        )
        controller.show(state: .recording, palette: AppTheme.pink.palette)

        try await Task.sleep(for: .milliseconds(500))
        let diagnostics = controller.diagnostics(expectedState: .recording)

        XCTAssertTrue(diagnostics.isVisible)
        XCTAssertTrue(diagnostics.matchesExpectedState)
        XCTAssertGreaterThanOrEqual(diagnostics.alphaValue, 0.95)
    }

    @MainActor
    private static func poll(
        timeoutMilliseconds: Int,
        intervalMilliseconds: Int = 50,
        condition: () -> Bool
    ) async throws -> Bool {
        let deadline = ContinuousClock.now.advanced(by: .milliseconds(timeoutMilliseconds))
        while ContinuousClock.now < deadline {
            if condition() {
                return true
            }
            try await Task.sleep(for: .milliseconds(intervalMilliseconds))
        }
        return condition()
    }

    @MainActor
    func testActiveSpaceRefreshKeepsPanelVisibleWithoutResettingState() async throws {
        try XCTSkipIf(NSScreen.main == nil, "Recording overlay panel tests require a visible screen.")
        let controller = RecordingOverlayPanelController()
        defer {
            controller.hide(animated: false)
        }

        controller.show(state: .recording, palette: AppTheme.pink.palette)
        controller.refreshVisiblePanelForActiveSpace()
        try await Task.sleep(for: .milliseconds(50))
        let diagnostics = controller.diagnostics(expectedState: .recording)

        XCTAssertTrue(diagnostics.success)
        XCTAssertTrue(diagnostics.isVisible)
        XCTAssertTrue(diagnostics.isOnActiveSpace)
        XCTAssertGreaterThanOrEqual(diagnostics.alphaValue, 0.95)
    }

    @MainActor
    func testShowTwiceReusesPanelAndContentView() async throws {
        try XCTSkipIf(NSScreen.main == nil, "Recording overlay panel tests require a visible screen.")
        let controller = RecordingOverlayPanelController()
        defer {
            controller.hide(animated: false)
        }

        controller.show(state: .recording, palette: AppTheme.pink.palette)
        let firstDiagnostics = controller.diagnostics(expectedState: .recording)
        let firstContentView = controller.panel?.contentView

        controller.show(state: .transcribing, palette: AppTheme.pink.palette)
        let secondDiagnostics = controller.diagnostics(expectedState: .transcribing)
        let secondContentView = controller.panel?.contentView

        XCTAssertNotNil(firstDiagnostics.panelInstanceID)
        XCTAssertEqual(firstDiagnostics.panelInstanceID, secondDiagnostics.panelInstanceID)
        XCTAssertNotNil(firstContentView)
        XCTAssertTrue(firstContentView === secondContentView)
        XCTAssertEqual(controller.viewModel.state, .transcribing)
        XCTAssertTrue(secondDiagnostics.matchesExpectedState)
    }

    @MainActor
    func testHideKeepsPanelForReuse() async throws {
        try XCTSkipIf(NSScreen.main == nil, "Recording overlay panel tests require a visible screen.")
        let controller = RecordingOverlayPanelController()
        defer {
            controller.hide(animated: false)
        }

        controller.show(state: .recording, palette: AppTheme.pink.palette)
        let shownID = controller.diagnostics().panelInstanceID
        XCTAssertNotNil(shownID)

        controller.hide(animated: false)
        let hiddenDiagnostics = controller.diagnostics()
        XCTAssertFalse(hiddenDiagnostics.isVisible)
        XCTAssertEqual(hiddenDiagnostics.panelInstanceID, shownID)

        controller.show(state: .recording, palette: AppTheme.pink.palette)
        let reshownDiagnostics = controller.diagnostics(expectedState: .recording)
        XCTAssertTrue(reshownDiagnostics.isVisible)
        XCTAssertEqual(reshownDiagnostics.panelInstanceID, shownID)
        XCTAssertTrue(reshownDiagnostics.matchesExpectedState)
    }

    @MainActor
    func testPaletteChangeFlowsThroughViewModel() async throws {
        try XCTSkipIf(NSScreen.main == nil, "Recording overlay panel tests require a visible screen.")
        let controller = RecordingOverlayPanelController()
        defer {
            controller.hide(animated: false)
        }

        controller.show(state: .recording, palette: AppTheme.pink.palette)
        XCTAssertEqual(controller.viewModel.palette, AppTheme.pink.palette)

        controller.show(state: .recording, palette: AppTheme.blue.palette)
        XCTAssertEqual(controller.viewModel.palette, AppTheme.blue.palette)

        let diagnostics = controller.diagnostics(expectedState: .recording)
        XCTAssertTrue(diagnostics.isVisible)
        XCTAssertTrue(diagnostics.matchesExpectedState)
    }
}
