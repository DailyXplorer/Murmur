import AppKit
import Carbon
import XCTest
@testable import HandyNative

final class GlobalShortcutMatcherTests: XCTestCase {
    func testOptionSpacePressAndReleaseAreConsumed() {
        var matcher = GlobalShortcutMatcher(descriptor: .optionSpace)

        XCTAssertEqual(matcher.handle(type: .keyDown, keyCode: 49, flags: .maskAlternate), .pressed)
        XCTAssertEqual(matcher.handle(type: .keyDown, keyCode: 49, flags: .maskAlternate), .consume)
        XCTAssertEqual(matcher.handle(type: .keyUp, keyCode: 49, flags: []), .released)
    }

    func testWrongModifierPassesThrough() {
        var matcher = GlobalShortcutMatcher(descriptor: .optionSpace)

        XCTAssertEqual(matcher.handle(type: .keyDown, keyCode: 49, flags: .maskCommand), .passThrough)
        XCTAssertEqual(matcher.handle(type: .keyUp, keyCode: 49, flags: []), .passThrough)
    }

    func testExtraModifierPassesThrough() {
        var matcher = GlobalShortcutMatcher(descriptor: .optionSpace)
        let flags: CGEventFlags = [.maskAlternate, .maskShift]

        XCTAssertEqual(matcher.handle(type: .keyDown, keyCode: 49, flags: flags), .passThrough)
    }

    func testShortcutDescriptorParsesConfiguredBindings() throws {
        let descriptor = try XCTUnwrap(GlobalShortcutDescriptor.parse("option+shift+space"))

        XCTAssertEqual(descriptor.keyCode, 49)
        XCTAssertEqual(descriptor.requiredFlags.intersection([.maskAlternate, .maskShift]), [.maskAlternate, .maskShift])
    }

    func testShortcutDescriptorParsesEscapeWithoutModifiers() throws {
        let descriptor = try XCTUnwrap(GlobalShortcutDescriptor.parse("escape"))

        XCTAssertEqual(descriptor.keyCode, CGKeyCode(kVK_Escape))
        XCTAssertTrue(descriptor.requiredFlags.intersection([.maskAlternate, .maskCommand, .maskControl, .maskShift]).isEmpty)
    }

    func testShortcutDescriptorRejectsMissingKey() {
        XCTAssertNil(GlobalShortcutDescriptor.parse("option+shift"))
    }

    func testCapturedShortcutBuildsCanonicalBinding() {
        XCTAssertEqual(
            GlobalShortcutDescriptor.bindingString(
                keyCode: CGKeyCode(kVK_Space),
                modifierFlags: [.option]
            ),
            "option+space"
        )
        XCTAssertEqual(
            GlobalShortcutDescriptor.bindingString(
                keyCode: CGKeyCode(kVK_Return),
                modifierFlags: [.command, .shift]
            ),
            "cmd+shift+return"
        )
    }

    func testCapturedShortcutRejectsUnmodifiedTypingKeysButAllowsEscape() {
        XCTAssertNil(
            GlobalShortcutDescriptor.bindingString(
                keyCode: CGKeyCode(kVK_ANSI_A),
                modifierFlags: []
            )
        )
        XCTAssertEqual(
            GlobalShortcutDescriptor.bindingString(
                keyCode: CGKeyCode(kVK_Escape),
                modifierFlags: []
            ),
            "escape"
        )
    }

    func testShortcutConflictDetectionNormalizesAliases() {
        XCTAssertTrue(GlobalShortcutDescriptor.bindingsConflict("alt+space", "option+space"))
        XCTAssertFalse(GlobalShortcutDescriptor.bindingsConflict("option+space", "option+shift+space"))
    }

    func testConfiguredShortcutDisplayUsesMacSymbols() {
        XCTAssertEqual(ShortcutBinding.displayName(for: "option+shift+space"), "⌥ ⇧ Space")
        XCTAssertEqual(ShortcutBinding.displayName(for: "cmd+return"), "⌘ Return")
    }
}
