import AppKit
import XCTest
@testable import MurmurNative

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
            keyboardEventPoster: keyboardEventPoster,
            isProcessTrusted: { true },
            isSecureInputActive: { false }
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
            keyboardEventPoster: keyboardEventPoster,
            isProcessTrusted: { true },
            isSecureInputActive: { false }
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
            keyboardEventPoster: keyboardEventPoster,
            isProcessTrusted: { true },
            isSecureInputActive: { false }
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
            directTextInserter: directTextInserter,
            isProcessTrusted: { true },
            isSecureInputActive: { false }
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

    @MainActor
    func testPasteThrowsWhenAccessibilityNotTrusted() async {
        var settings = AppSettings.defaults
        settings.pasteMethod = .commandV
        settings.pasteDelayMilliseconds = 0
        let pasteboard = Self.makePasteboard()
        pasteboard.clearContents()
        pasteboard.setString("previous", forType: .string)
        let keyboardEventPoster = RecordingKeyboardEventPoster()
        let service = PasteService(
            pasteboard: pasteboard,
            keyboardEventPoster: keyboardEventPoster,
            isProcessTrusted: { false },
            isSecureInputActive: { false }
        )

        do {
            try await service.paste("hello", options: PasteOutputOptions(settings: settings))
            XCTFail("Expected accessibilityNotTrusted to be thrown")
        } catch PasteServiceError.accessibilityNotTrusted {
            // Expected.
        } catch {
            XCTFail("Unexpected error: \(error)")
        }

        XCTAssertEqual(pasteboard.string(forType: .string), "hello")
        XCTAssertEqual(keyboardEventPoster.shortcuts, [])
        XCTAssertEqual(keyboardEventPoster.typedTexts, [])
    }

    @MainActor
    func testPasteThrowsWhenSecureInputActive() async {
        var settings = AppSettings.defaults
        settings.pasteMethod = .commandV
        settings.pasteDelayMilliseconds = 0
        let pasteboard = Self.makePasteboard()
        pasteboard.clearContents()
        pasteboard.setString("previous", forType: .string)
        let keyboardEventPoster = RecordingKeyboardEventPoster()
        let service = PasteService(
            pasteboard: pasteboard,
            keyboardEventPoster: keyboardEventPoster,
            isProcessTrusted: { true },
            isSecureInputActive: { true }
        )

        do {
            try await service.paste("hello", options: PasteOutputOptions(settings: settings))
            XCTFail("Expected secureInputActive to be thrown")
        } catch PasteServiceError.secureInputActive {
            // Expected.
        } catch {
            XCTFail("Unexpected error: \(error)")
        }

        XCTAssertEqual(pasteboard.string(forType: .string), "hello")
        XCTAssertEqual(keyboardEventPoster.shortcuts, [])
        XCTAssertEqual(keyboardEventPoster.typedTexts, [])
    }

    @MainActor
    func testDontModifyRestoresPreviousStringWhenClipboardUntouched() async throws {
        var settings = AppSettings.defaults
        settings.pasteMethod = .commandV
        settings.clipboardHandling = .dontModify
        settings.pasteDelayMilliseconds = 0
        let pasteboard = Self.makePasteboard()
        pasteboard.clearContents()
        pasteboard.setString("before", forType: .string)
        let service = PasteService(
            pasteboard: pasteboard,
            keyboardEventPoster: RecordingKeyboardEventPoster(),
            isProcessTrusted: { true },
            isSecureInputActive: { false }
        )

        try await service.paste("hello", options: PasteOutputOptions(settings: settings))

        XCTAssertEqual(pasteboard.string(forType: .string), "before")
    }

    @MainActor
    func testDontModifyKeepsTranscriptWhenPreviousClipboardWasEmpty() async throws {
        var settings = AppSettings.defaults
        settings.pasteMethod = .commandV
        settings.clipboardHandling = .dontModify
        settings.pasteDelayMilliseconds = 0
        let pasteboard = Self.makePasteboard()
        pasteboard.clearContents()
        let service = PasteService(
            pasteboard: pasteboard,
            keyboardEventPoster: RecordingKeyboardEventPoster(),
            isProcessTrusted: { true },
            isSecureInputActive: { false }
        )

        try await service.paste("hello", options: PasteOutputOptions(settings: settings))

        XCTAssertEqual(pasteboard.string(forType: .string), "hello")
    }

    @MainActor
    func testDontModifySkipsRestoreWhenClipboardChangedExternally() async throws {
        var settings = AppSettings.defaults
        settings.pasteMethod = .commandV
        settings.clipboardHandling = .dontModify
        settings.pasteDelayMilliseconds = 0
        let pasteboard = Self.makePasteboard()
        pasteboard.clearContents()
        pasteboard.setString("before", forType: .string)
        let keyboardEventPoster = ClipboardWritingKeyboardEventPoster(
            pasteboard: pasteboard,
            externalText: "external"
        )
        let service = PasteService(
            pasteboard: pasteboard,
            keyboardEventPoster: keyboardEventPoster,
            isProcessTrusted: { true },
            isSecureInputActive: { false }
        )

        try await service.paste("hello", options: PasteOutputOptions(settings: settings))

        XCTAssertEqual(pasteboard.string(forType: .string), "external")
    }

    private static func makePasteboard() -> NSPasteboard {
        NSPasteboard(name: NSPasteboard.Name("MurmurNativeTests-\(UUID().uuidString)"))
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

/// Simulates another app writing to the clipboard while the synthetic Cmd+V
/// is "in flight": the write happens when the paste shortcut is posted.
@MainActor
private final class ClipboardWritingKeyboardEventPoster: KeyboardEventPosting {
    private let pasteboard: NSPasteboard
    private let externalText: String

    init(pasteboard: NSPasteboard, externalText: String) {
        self.pasteboard = pasteboard
        self.externalText = externalText
    }

    func postShortcut(virtualKey: CGKeyCode, flags: CGEventFlags) throws {
        pasteboard.clearContents()
        pasteboard.setString(externalText, forType: .string)
    }

    func postText(_ text: String) throws {}
}

@MainActor
private final class RecordingDirectTextInserter: DirectTextInserting {
    private(set) var insertedTexts: [String] = []

    func insertText(_ text: String) throws {
        insertedTexts.append(text)
    }
}
