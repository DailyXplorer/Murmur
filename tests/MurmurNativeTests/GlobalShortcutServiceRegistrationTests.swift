import AppKit
import Carbon
import XCTest
@testable import MurmurNative

final class GlobalShortcutServiceRegistrationTests: XCTestCase {
    private let cancelDescriptor = GlobalShortcutDescriptor(
        keyCode: CGKeyCode(kVK_Escape),
        requiredFlags: []
    )

    private func pressedTranscribeMatchers() -> [String: GlobalShortcutMatcher] {
        var transcribe = GlobalShortcutMatcher(descriptor: .optionSpace)
        XCTAssertEqual(transcribe.handle(type: .keyDown, keyCode: 49, flags: .maskAlternate), .pressed)
        return [ShortcutBinding.transcribeID: transcribe]
    }

    func testCarryOverPreservesPressedStateForUnchangedDescriptor() {
        let carried = GlobalShortcutService.carryingOverPressState(
            from: pressedTranscribeMatchers(),
            registrations: [
                GlobalShortcutRegistration(bindingID: ShortcutBinding.transcribeID, descriptor: .optionSpace),
                GlobalShortcutRegistration(bindingID: ShortcutBinding.cancelID, descriptor: cancelDescriptor)
            ]
        )

        XCTAssertEqual(carried[ShortcutBinding.transcribeID]?.isPressed, true)
        XCTAssertEqual(carried[ShortcutBinding.cancelID]?.isPressed, false)
    }

    func testCarryOverDropsPressedStateWhenDescriptorChanged() {
        let changedDescriptor = GlobalShortcutDescriptor(
            keyCode: CGKeyCode(kVK_Return),
            requiredFlags: .maskAlternate
        )

        let carried = GlobalShortcutService.carryingOverPressState(
            from: pressedTranscribeMatchers(),
            registrations: [
                GlobalShortcutRegistration(bindingID: ShortcutBinding.transcribeID, descriptor: changedDescriptor)
            ]
        )

        XCTAssertEqual(carried[ShortcutBinding.transcribeID]?.isPressed, false)
    }

    func testCarryOverRemovesUnregisteredBindings() {
        let previous: [String: GlobalShortcutMatcher] = [
            ShortcutBinding.transcribeID: GlobalShortcutMatcher(descriptor: .optionSpace),
            ShortcutBinding.cancelID: GlobalShortcutMatcher(descriptor: cancelDescriptor)
        ]

        let carried = GlobalShortcutService.carryingOverPressState(
            from: previous,
            registrations: [
                GlobalShortcutRegistration(bindingID: ShortcutBinding.transcribeID, descriptor: .optionSpace)
            ]
        )

        XCTAssertNil(carried[ShortcutBinding.cancelID])
        XCTAssertEqual(Array(carried.keys), [ShortcutBinding.transcribeID])
    }

    func testCarriedOverMatcherReleasesOnKeyUp() throws {
        // Push-to-talk regression: after registrations are swapped mid-hold,
        // the key-up for the still-held key must still emit .released.
        let carried = GlobalShortcutService.carryingOverPressState(
            from: pressedTranscribeMatchers(),
            registrations: [
                GlobalShortcutRegistration(bindingID: ShortcutBinding.transcribeID, descriptor: .optionSpace),
                GlobalShortcutRegistration(bindingID: ShortcutBinding.cancelID, descriptor: cancelDescriptor)
            ]
        )

        var transcribe = try XCTUnwrap(carried[ShortcutBinding.transcribeID])
        XCTAssertEqual(transcribe.handle(type: .keyUp, keyCode: 49, flags: []), .released)
        XCTAssertFalse(transcribe.isPressed)
    }
}
