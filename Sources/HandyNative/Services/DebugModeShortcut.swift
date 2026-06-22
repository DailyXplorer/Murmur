import AppKit

enum DebugModeShortcut {
    static func matches(_ event: NSEvent) -> Bool {
        matches(
            charactersIgnoringModifiers: event.charactersIgnoringModifiers,
            modifierFlags: event.modifierFlags
        )
    }

    static func matches(
        charactersIgnoringModifiers: String?,
        modifierFlags: NSEvent.ModifierFlags
    ) -> Bool {
        let flags = modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard flags.contains(.shift),
              flags.contains(.command) || flags.contains(.control),
              charactersIgnoringModifiers?.lowercased() == "d"
        else {
            return false
        }

        return true
    }
}
