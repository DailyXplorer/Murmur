import AppKit
import ApplicationServices
import Carbon.HIToolbox
import Foundation

struct PasteOutputOptions: Equatable {
    let pasteMethod: PasteMethod
    let pasteDelayMilliseconds: Int
    let appendTrailingSpace: Bool
    let clipboardHandling: ClipboardHandling
    let autoSubmitAfterPaste: Bool
    let autoSubmitKey: AutoSubmitKey

    init(settings: AppSettings) {
        pasteMethod = settings.pasteMethod
        pasteDelayMilliseconds = settings.pasteDelayMilliseconds
        appendTrailingSpace = settings.appendTrailingSpace
        clipboardHandling = settings.clipboardHandling
        autoSubmitAfterPaste = settings.autoSubmitAfterPaste
        autoSubmitKey = settings.autoSubmitKey
    }
}

enum PasteServiceError: LocalizedError {
    case emptyText
    case eventCreationFailed
    case unsupportedPasteMethod(PasteMethod)
    case accessibilityNotTrusted
    case secureInputActive

    var errorDescription: String? {
        switch self {
        case .emptyText: "There is no text to paste."
        case .eventCreationFailed: "Unable to create the keyboard event needed to paste."
        case let .unsupportedPasteMethod(method): "Paste method '\(method.title)' is not supported in the native macOS app."
        case .accessibilityNotTrusted:
            "Murmur needs Accessibility permission to paste. Enable it in System Settings > Privacy & Security > Accessibility. The text is on your clipboard — press Cmd+V to paste it."
        case .secureInputActive:
            "A password field is capturing keyboard input, so Murmur could not paste. The text is on your clipboard — press Cmd+V to paste it."
        }
    }
}

@MainActor
final class PasteService {
    private let pasteboard: NSPasteboard
    private let keyboardEventPoster: KeyboardEventPosting
    private let directTextInserter: DirectTextInserting
    private let isProcessTrusted: () -> Bool
    private let isSecureInputActive: () -> Bool

    init(
        pasteboard: NSPasteboard = .general,
        keyboardEventPoster: KeyboardEventPosting = CGKeyboardEventPoster(),
        directTextInserter: DirectTextInserting? = nil,
        isProcessTrusted: @escaping () -> Bool = { AXIsProcessTrusted() },
        isSecureInputActive: @escaping () -> Bool = { IsSecureEventInputEnabled() }
    ) {
        self.pasteboard = pasteboard
        self.keyboardEventPoster = keyboardEventPoster
        self.directTextInserter = directTextInserter ?? AccessibilityDirectTextInserter(fallbackPoster: keyboardEventPoster)
        self.isProcessTrusted = isProcessTrusted
        self.isSecureInputActive = isSecureInputActive
    }

    func paste(_ rawText: String, options: PasteOutputOptions) async throws {
        let text = Self.preparedText(rawText, options: options)
        guard !text.isEmpty else {
            throw PasteServiceError.emptyText
        }

        switch options.pasteMethod.macOSCompatible {
        case .commandV:
            try await pasteViaClipboard(text, virtualKey: KeyCode.v, flags: .maskCommand, options: options)
        case .commandShiftV:
            try await pasteViaClipboard(text, virtualKey: KeyCode.v, flags: [.maskCommand, .maskShift], options: options)
        case .direct:
            guard isProcessTrusted() else {
                leaveTranscriptOnClipboard(text)
                throw PasteServiceError.accessibilityNotTrusted
            }
            try typeTextDirectly(text)
            try await sendAutoSubmitIfNeeded(options: options)
            copyToClipboardIfRequested(text, options: options)
        case .none:
            copyToClipboardIfRequested(text, options: options)
        case .shiftInsert, .externalScript:
            throw PasteServiceError.unsupportedPasteMethod(options.pasteMethod)
        }
    }

    private func pasteViaClipboard(
        _ text: String,
        virtualKey: CGKeyCode,
        flags: CGEventFlags,
        options: PasteOutputOptions
    ) async throws {
        guard isProcessTrusted() else {
            leaveTranscriptOnClipboard(text)
            throw PasteServiceError.accessibilityNotTrusted
        }
        if isSecureInputActive() {
            leaveTranscriptOnClipboard(text)
            throw PasteServiceError.secureInputActive
        }

        let previousString = pasteboard.string(forType: .string)
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
        let changeCountAfterSet = pasteboard.changeCount
        try await Task.sleep(for: .milliseconds(max(0, options.pasteDelayMilliseconds)))
        try sendKeyboardShortcut(virtualKey: virtualKey, flags: flags)
        try await Task.sleep(for: .milliseconds(max(200, options.pasteDelayMilliseconds)))
        try await sendAutoSubmitIfNeeded(options: options)

        if options.clipboardHandling == .dontModify,
           pasteboard.changeCount == changeCountAfterSet,
           let previousString {
            pasteboard.clearContents()
            pasteboard.setString(previousString, forType: .string)
        } else if options.clipboardHandling == .copyToClipboard {
            pasteboard.clearContents()
            pasteboard.setString(text, forType: .string)
        }
    }

    /// When a paste cannot be delivered, leave the transcript on the clipboard
    /// so the user can press Cmd+V manually instead of losing the text.
    private func leaveTranscriptOnClipboard(_ text: String) {
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
    }

    nonisolated static func preparedText(_ rawText: String, options: PasteOutputOptions) -> String {
        var text = rawText.trimmingCharacters(in: .whitespacesAndNewlines)
        if options.appendTrailingSpace, !text.isEmpty, !text.hasSuffix(" ") {
            text += " "
        }
        return text
    }

    nonisolated static func shouldSendAutoSubmit(options: PasteOutputOptions) -> Bool {
        options.autoSubmitAfterPaste && options.pasteMethod.macOSCompatible != .none
    }

    private func sendAutoSubmitIfNeeded(options: PasteOutputOptions) async throws {
        guard Self.shouldSendAutoSubmit(options: options) else {
            return
        }

        try await Task.sleep(for: .milliseconds(50))
        switch options.autoSubmitKey {
        case .enter:
            try sendKeyboardShortcut(virtualKey: KeyCode.returnKey)
        case .controlEnter:
            try sendKeyboardShortcut(virtualKey: KeyCode.returnKey, flags: .maskControl)
        case .commandEnter:
            try sendKeyboardShortcut(virtualKey: KeyCode.returnKey, flags: .maskCommand)
        }
    }

    private func copyToClipboardIfRequested(_ text: String, options: PasteOutputOptions) {
        guard options.clipboardHandling == .copyToClipboard else {
            return
        }
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
    }

    private func typeTextDirectly(_ text: String) throws {
        try directTextInserter.insertText(text)
    }

    private func sendKeyboardShortcut(virtualKey: CGKeyCode, flags: CGEventFlags = []) throws {
        try keyboardEventPoster.postShortcut(virtualKey: virtualKey, flags: flags)
    }
}

@MainActor
protocol KeyboardEventPosting {
    func postShortcut(virtualKey: CGKeyCode, flags: CGEventFlags) throws
    func postText(_ text: String) throws
}

@MainActor
protocol DirectTextInserting {
    func insertText(_ text: String) throws
}

@MainActor
struct AccessibilityDirectTextInserter: DirectTextInserting {
    private let fallbackPoster: KeyboardEventPosting

    init(fallbackPoster: KeyboardEventPosting = CGKeyboardEventPoster()) {
        self.fallbackPoster = fallbackPoster
    }

    func insertText(_ text: String) throws {
        do {
            try insertIntoFocusedElement(text)
        } catch {
            try fallbackPoster.postText(text)
        }
    }

    private func insertIntoFocusedElement(_ text: String) throws {
        let systemWideElement = AXUIElementCreateSystemWide()
        var focusedValue: CFTypeRef?
        let focusedResult = AXUIElementCopyAttributeValue(
            systemWideElement,
            kAXFocusedUIElementAttribute as CFString,
            &focusedValue
        )
        guard focusedResult == .success,
              let focusedValue,
              CFGetTypeID(focusedValue) == AXUIElementGetTypeID()
        else {
            throw DirectTextInsertionError.focusedElementUnavailable
        }
        let focusedElement = unsafeDowncast(focusedValue, to: AXUIElement.self)

        if AXUIElementSetAttributeValue(
            focusedElement,
            kAXSelectedTextAttribute as CFString,
            text as CFString
        ) == .success {
            return
        }

        try replaceFocusedElementValue(text, focusedElement: focusedElement)
    }

    private func replaceFocusedElementValue(_ text: String, focusedElement: AXUIElement) throws {
        var valueRef: CFTypeRef?
        let valueResult = AXUIElementCopyAttributeValue(
            focusedElement,
            kAXValueAttribute as CFString,
            &valueRef
        )
        guard valueResult == .success,
              let currentValue = valueRef as? String
        else {
            throw DirectTextInsertionError.valueUnavailable
        }

        let currentNSString = currentValue as NSString
        let selectedRange = focusedSelectedRange(
            for: focusedElement,
            fallbackLocation: currentNSString.length
        )
        let replacementRange = boundedRange(selectedRange, valueLength: currentNSString.length)
        let updatedValue = currentNSString.replacingCharacters(
            in: replacementRange,
            with: text
        )

        let setResult = AXUIElementSetAttributeValue(
            focusedElement,
            kAXValueAttribute as CFString,
            updatedValue as CFString
        )
        guard setResult == .success else {
            throw DirectTextInsertionError.valueNotWritable
        }

        var caretRange = CFRange(
            location: replacementRange.location + (text as NSString).length,
            length: 0
        )
        if let caretValue = AXValueCreate(.cfRange, &caretRange) {
            AXUIElementSetAttributeValue(
                focusedElement,
                kAXSelectedTextRangeAttribute as CFString,
                caretValue
            )
        }
    }

    private func focusedSelectedRange(
        for focusedElement: AXUIElement,
        fallbackLocation: Int
    ) -> CFRange {
        var selectedRangeRef: CFTypeRef?
        let rangeResult = AXUIElementCopyAttributeValue(
            focusedElement,
            kAXSelectedTextRangeAttribute as CFString,
            &selectedRangeRef
        )
        guard rangeResult == .success,
              let selectedRangeRef,
              CFGetTypeID(selectedRangeRef) == AXValueGetTypeID()
        else {
            return CFRange(location: fallbackLocation, length: 0)
        }
        let selectedRangeValue = unsafeDowncast(selectedRangeRef, to: AXValue.self)
        guard AXValueGetType(selectedRangeValue) == .cfRange else {
            return CFRange(location: fallbackLocation, length: 0)
        }

        var selectedRange = CFRange(location: fallbackLocation, length: 0)
        if AXValueGetValue(selectedRangeValue, .cfRange, &selectedRange) {
            return selectedRange
        }
        return CFRange(location: fallbackLocation, length: 0)
    }

    private func boundedRange(_ range: CFRange, valueLength: Int) -> NSRange {
        let location = min(max(0, range.location), valueLength)
        let length = min(max(0, range.length), valueLength - location)
        return NSRange(location: location, length: length)
    }
}

private enum DirectTextInsertionError: LocalizedError {
    case focusedElementUnavailable
    case valueUnavailable
    case valueNotWritable

    var errorDescription: String? {
        switch self {
        case .focusedElementUnavailable:
            "No focused text element is available for direct text insertion."
        case .valueUnavailable:
            "The focused element does not expose editable text."
        case .valueNotWritable:
            "The focused element does not allow direct text replacement."
        }
    }
}

@MainActor
struct CGKeyboardEventPoster: KeyboardEventPosting {
    func postShortcut(virtualKey: CGKeyCode, flags: CGEventFlags = []) throws {
        guard
            let source = CGEventSource(stateID: .hidSystemState),
            let keyDown = CGEvent(keyboardEventSource: source, virtualKey: virtualKey, keyDown: true),
            let keyUp = CGEvent(keyboardEventSource: source, virtualKey: virtualKey, keyDown: false)
        else {
            throw PasteServiceError.eventCreationFailed
        }

        keyDown.flags = flags
        keyUp.flags = flags
        post(keyDown)
        post(keyUp)
    }

    func postText(_ text: String) throws {
        guard let source = CGEventSource(stateID: .hidSystemState) else {
            throw PasteServiceError.eventCreationFailed
        }

        for character in text {
            var utf16 = Array(String(character).utf16)
            guard
                let keyDown = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: true),
                let keyUp = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: false)
            else {
                throw PasteServiceError.eventCreationFailed
            }

            keyDown.keyboardSetUnicodeString(stringLength: utf16.count, unicodeString: &utf16)
            keyUp.keyboardSetUnicodeString(stringLength: utf16.count, unicodeString: &utf16)
            keyDown.post(tap: .cgSessionEventTap)
            keyUp.post(tap: .cgSessionEventTap)
        }
    }

    private func post(_ event: CGEvent) {
        event.post(tap: .cgSessionEventTap)
    }
}

private enum KeyCode {
    static let v: CGKeyCode = 9
    static let returnKey: CGKeyCode = 36
}
