import AppKit
import XCTest
@testable import HandyNative

final class DebugModeShortcutTests: XCTestCase {
    func testCommandShiftDTogglesDebugModeOnMac() {
        XCTAssertTrue(
            DebugModeShortcut.matches(
                charactersIgnoringModifiers: "d",
                modifierFlags: [.command, .shift]
            )
        )
        XCTAssertTrue(
            DebugModeShortcut.matches(
                charactersIgnoringModifiers: "D",
                modifierFlags: [.command, .shift]
            )
        )
    }

    func testControlShiftDAlsoMatchesSourceAppShortcutContract() {
        XCTAssertTrue(
            DebugModeShortcut.matches(
                charactersIgnoringModifiers: "d",
                modifierFlags: [.control, .shift]
            )
        )
    }

    func testDebugShortcutRequiresShiftAndCommandOrControl() {
        XCTAssertFalse(
            DebugModeShortcut.matches(
                charactersIgnoringModifiers: "d",
                modifierFlags: [.command]
            )
        )
        XCTAssertFalse(
            DebugModeShortcut.matches(
                charactersIgnoringModifiers: "d",
                modifierFlags: [.shift]
            )
        )
        XCTAssertFalse(
            DebugModeShortcut.matches(
                charactersIgnoringModifiers: "d",
                modifierFlags: [.option, .shift]
            )
        )
    }

    func testDebugShortcutRequiresDKey() {
        XCTAssertFalse(
            DebugModeShortcut.matches(
                charactersIgnoringModifiers: "f",
                modifierFlags: [.command, .shift]
            )
        )
        XCTAssertFalse(
            DebugModeShortcut.matches(
                charactersIgnoringModifiers: nil,
                modifierFlags: [.command, .shift]
            )
        )
    }
}
