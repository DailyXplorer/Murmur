import XCTest
@testable import MurmurNative

@MainActor
final class GlobalShortcutRegistrationFallbackTests: XCTestCase {
    private func makeBinding(currentBinding: String, defaultBinding: String = "option+space") -> ShortcutBinding {
        ShortcutBinding(
            id: ShortcutBinding.transcribeID,
            name: "Transcribe",
            description: "Converts your speech into text.",
            defaultBinding: defaultBinding,
            currentBinding: currentBinding
        )
    }

    func testValidCurrentBindingParsesWithoutFallback() throws {
        let binding = makeBinding(currentBinding: "option+space")

        let resolution = try XCTUnwrap(AppModel.descriptorWithFallback(for: binding))

        XCTAssertEqual(resolution.descriptor, GlobalShortcutDescriptor.parse("option+space"))
        XCTAssertEqual(resolution.usedFallback, false)
    }

    func testUnparseableCurrentBindingFallsBackToDefault() throws {
        let binding = makeBinding(currentBinding: "option+f19")

        let resolution = try XCTUnwrap(AppModel.descriptorWithFallback(for: binding))

        XCTAssertEqual(resolution.descriptor, GlobalShortcutDescriptor.parse("option+space"))
        XCTAssertEqual(resolution.usedFallback, true)
    }

    func testUnparseableCurrentAndDefaultReturnsNil() {
        let binding = makeBinding(currentBinding: "option+f19", defaultBinding: "option+f18")

        XCTAssertNil(AppModel.descriptorWithFallback(for: binding))
    }

    func testEmptyCurrentBindingFallsBackToDefault() throws {
        let binding = makeBinding(currentBinding: "")

        let resolution = try XCTUnwrap(AppModel.descriptorWithFallback(for: binding))

        XCTAssertEqual(resolution.descriptor, GlobalShortcutDescriptor.parse("option+space"))
        XCTAssertEqual(resolution.usedFallback, true)
    }
}
