import AppKit
import XCTest
@testable import HandyNative

final class PasteServiceTests: XCTestCase {
    func testPreparedTextTrimsInput() {
        let options = PasteOutputOptions(
            settings: AppSettings.defaults
        )

        XCTAssertEqual(PasteService.preparedText("  hello\n", options: options), "hello")
    }

    func testPreparedTextCanAppendTrailingSpace() {
        var settings = AppSettings.defaults
        settings.appendTrailingSpace = true
        let options = PasteOutputOptions(settings: settings)

        XCTAssertEqual(PasteService.preparedText("hello", options: options), "hello ")
        XCTAssertEqual(PasteService.preparedText("hello ", options: options), "hello ")
        XCTAssertEqual(PasteService.preparedText("   ", options: options), "")
    }

    func testPasteOptionsMirrorSettings() {
        var settings = AppSettings.defaults
        settings.pasteMethod = .direct
        settings.clipboardHandling = .copyToClipboard
        settings.autoSubmitAfterPaste = true
        settings.autoSubmitKey = .commandEnter

        let options = PasteOutputOptions(settings: settings)

        XCTAssertEqual(options.pasteMethod, .direct)
        XCTAssertEqual(options.clipboardHandling, .copyToClipboard)
        XCTAssertTrue(options.autoSubmitAfterPaste)
        XCTAssertEqual(options.autoSubmitKey, .commandEnter)
    }

    func testAutoSubmitSkipsNonePasteMethod() {
        var settings = AppSettings.defaults
        settings.autoSubmitAfterPaste = true
        settings.pasteMethod = .none

        XCTAssertFalse(PasteService.shouldSendAutoSubmit(options: PasteOutputOptions(settings: settings)))

        settings.pasteMethod = .commandV
        XCTAssertTrue(PasteService.shouldSendAutoSubmit(options: PasteOutputOptions(settings: settings)))
    }

    @MainActor
    func testCommandVPasteRestoresClipboardAndPostsShortcut() async throws {
        var settings = AppSettings.defaults
        settings.pasteMethod = .commandV
        settings.clipboardHandling = .dontModify
        settings.pasteDelayMilliseconds = 0
        let pasteboard = Self.makePasteboard()
        pasteboard.clearContents()
        pasteboard.setString("previous", forType: .string)
        let keyboardEventPoster = RecordingKeyboardEventPoster()
        let service = PasteService(
            pasteboard: pasteboard,
            keyboardEventPoster: keyboardEventPoster
        )

        try await service.paste("  hello  ", options: PasteOutputOptions(settings: settings))

        XCTAssertEqual(pasteboard.string(forType: .string), "previous")
        XCTAssertEqual(
            keyboardEventPoster.shortcuts,
            [RecordedShortcut(virtualKey: 9, flagsRawValue: CGEventFlags.maskCommand.rawValue)]
        )
        XCTAssertEqual(keyboardEventPoster.typedTexts, [])
    }

    @MainActor
    func testCommandVPasteCanLeaveOutputOnClipboard() async throws {
        var settings = AppSettings.defaults
        settings.pasteMethod = .commandV
        settings.clipboardHandling = .copyToClipboard
        settings.pasteDelayMilliseconds = 0
        let pasteboard = Self.makePasteboard()
        pasteboard.clearContents()
        pasteboard.setString("previous", forType: .string)
        let keyboardEventPoster = RecordingKeyboardEventPoster()
        let service = PasteService(
            pasteboard: pasteboard,
            keyboardEventPoster: keyboardEventPoster
        )

        try await service.paste("hello", options: PasteOutputOptions(settings: settings))

        XCTAssertEqual(pasteboard.string(forType: .string), "hello")
        XCTAssertEqual(
            keyboardEventPoster.shortcuts,
            [RecordedShortcut(virtualKey: 9, flagsRawValue: CGEventFlags.maskCommand.rawValue)]
        )
    }

    @MainActor
    func testCommandVPasteCanAutoSubmitAfterShortcut() async throws {
        var settings = AppSettings.defaults
        settings.pasteMethod = .commandV
        settings.pasteDelayMilliseconds = 0
        settings.autoSubmitAfterPaste = true
        settings.autoSubmitKey = .controlEnter
        let keyboardEventPoster = RecordingKeyboardEventPoster()
        let service = PasteService(
            pasteboard: Self.makePasteboard(),
            keyboardEventPoster: keyboardEventPoster
        )

        try await service.paste("hello", options: PasteOutputOptions(settings: settings))

        XCTAssertEqual(
            keyboardEventPoster.shortcuts,
            [
                RecordedShortcut(virtualKey: 9, flagsRawValue: CGEventFlags.maskCommand.rawValue),
                RecordedShortcut(virtualKey: 36, flagsRawValue: CGEventFlags.maskControl.rawValue),
            ]
        )
    }

    @MainActor
    func testDirectPasteInsertsTextDirectlyAndAutoSubmits() async throws {
        var settings = AppSettings.defaults
        settings.pasteMethod = .direct
        settings.autoSubmitAfterPaste = true
        settings.autoSubmitKey = .commandEnter
        let keyboardEventPoster = RecordingKeyboardEventPoster()
        let directTextInserter = RecordingDirectTextInserter()
        let service = PasteService(
            pasteboard: Self.makePasteboard(),
            keyboardEventPoster: keyboardEventPoster,
            directTextInserter: directTextInserter
        )

        try await service.paste("hello", options: PasteOutputOptions(settings: settings))

        XCTAssertEqual(directTextInserter.insertedTexts, ["hello"])
        XCTAssertEqual(keyboardEventPoster.typedTexts, [])
        XCTAssertEqual(
            keyboardEventPoster.shortcuts,
            [RecordedShortcut(virtualKey: 36, flagsRawValue: CGEventFlags.maskCommand.rawValue)]
        )
    }

    @MainActor
    func testNonePasteMethodOnlyCopiesWhenRequested() async throws {
        var settings = AppSettings.defaults
        settings.pasteMethod = .none
        settings.clipboardHandling = .copyToClipboard
        let pasteboard = Self.makePasteboard()
        pasteboard.clearContents()
        let keyboardEventPoster = RecordingKeyboardEventPoster()
        let service = PasteService(
            pasteboard: pasteboard,
            keyboardEventPoster: keyboardEventPoster
        )

        try await service.paste("hello", options: PasteOutputOptions(settings: settings))

        XCTAssertEqual(pasteboard.string(forType: .string), "hello")
        XCTAssertEqual(keyboardEventPoster.shortcuts, [])
        XCTAssertEqual(keyboardEventPoster.typedTexts, [])
    }

    private static func makePasteboard() -> NSPasteboard {
        NSPasteboard(name: NSPasteboard.Name("HandyNativeTests-\(UUID().uuidString)"))
    }
}

private struct RecordedShortcut: Equatable {
    var virtualKey: CGKeyCode
    var flagsRawValue: UInt64
}

@MainActor
private final class RecordingKeyboardEventPoster: KeyboardEventPosting {
    private(set) var shortcuts: [RecordedShortcut] = []
    private(set) var typedTexts: [String] = []

    func postShortcut(virtualKey: CGKeyCode, flags: CGEventFlags) throws {
        shortcuts.append(
            RecordedShortcut(
                virtualKey: virtualKey,
                flagsRawValue: flags.rawValue
            )
        )
    }

    func postText(_ text: String) throws {
        typedTexts.append(text)
    }
}

@MainActor
private final class RecordingDirectTextInserter: DirectTextInserting {
    private(set) var insertedTexts: [String] = []

    func insertText(_ text: String) throws {
        insertedTexts.append(text)
    }
}
