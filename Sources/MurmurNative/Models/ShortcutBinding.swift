import Foundation

struct ShortcutBinding: Codable, Equatable, Identifiable {
    var id: String
    var name: String
    var description: String
    var defaultBinding: String
    var currentBinding: String

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case description
        case defaultBinding = "default_binding"
        case currentBinding = "current_binding"
    }

    init(
        id: String,
        name: String,
        description: String,
        defaultBinding: String,
        currentBinding: String
    ) {
        self.id = id
        self.name = name
        self.description = description
        self.defaultBinding = defaultBinding
        self.currentBinding = currentBinding
    }

    var displayBinding: String {
        Self.displayName(for: currentBinding)
    }

    static let transcribeID = "transcribe"
    static let transcribeWithPostProcessID = "transcribe_with_post_process"
    static let cancelID = "cancel"

    static let defaults: [String: ShortcutBinding] = [
        transcribeID: ShortcutBinding(
            id: transcribeID,
            name: "Transcribe",
            description: "Converts your speech into text.",
            defaultBinding: "option+space",
            currentBinding: "option+space"
        ),
        transcribeWithPostProcessID: ShortcutBinding(
            id: transcribeWithPostProcessID,
            name: "Transcribe with Post-Processing",
            description: "Converts your speech into text and applies AI post-processing.",
            defaultBinding: "option+shift+space",
            currentBinding: "option+shift+space"
        ),
        cancelID: ShortcutBinding(
            id: cancelID,
            name: "Cancel",
            description: "Cancels the current recording.",
            defaultBinding: "escape",
            currentBinding: "escape"
        ),
    ]

    static func displayName(for binding: String) -> String {
        let parts = normalizedParts(binding)
        guard parts.isEmpty == false else {
            return "Not set"
        }

        return parts.map { part in
            switch part {
            case "cmd", "command", "meta":
                "⌘"
            case "ctrl", "control":
                "⌃"
            case "option", "alt":
                "⌥"
            case "shift":
                "⇧"
            case "space":
                "Space"
            case "escape", "esc":
                "Esc"
            case "return", "enter":
                "Return"
            case "tab":
                "Tab"
            case "delete", "backspace":
                "Delete"
            default:
                part.uppercased()
            }
        }.joined(separator: " ")
    }

    static func normalizedParts(_ binding: String) -> [String] {
        binding
            .lowercased()
            .split(separator: "+")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }
}

extension Dictionary where Key == String, Value == ShortcutBinding {
    var mergedWithShortcutDefaults: [String: ShortcutBinding] {
        var merged = ShortcutBinding.defaults
        for (id, binding) in self where binding.currentBinding.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false {
            merged[id] = binding
        }
        return merged
    }
}

extension AppSettings {
    var transcribeShortcutBinding: ShortcutBinding {
        shortcutBindings[ShortcutBinding.transcribeID] ?? ShortcutBinding(
            id: ShortcutBinding.transcribeID,
            name: "Transcribe",
            description: "Converts your speech into text.",
            defaultBinding: "option+space",
            currentBinding: "option+space"
        )
    }

    var transcribeWithPostProcessShortcutBinding: ShortcutBinding {
        shortcutBindings[ShortcutBinding.transcribeWithPostProcessID] ?? ShortcutBinding(
            id: ShortcutBinding.transcribeWithPostProcessID,
            name: "Transcribe with Post-Processing",
            description: "Converts your speech into text and applies AI post-processing.",
            defaultBinding: "option+shift+space",
            currentBinding: "option+shift+space"
        )
    }

    var cancelShortcutBinding: ShortcutBinding {
        shortcutBindings[ShortcutBinding.cancelID] ?? ShortcutBinding(
            id: ShortcutBinding.cancelID,
            name: "Cancel",
            description: "Cancels the current recording.",
            defaultBinding: "escape",
            currentBinding: "escape"
        )
    }

    mutating func updateShortcutBinding(id: String, currentBinding: String) {
        guard var binding = shortcutBindings[id] ?? ShortcutBinding.defaults[id] else {
            return
        }

        let trimmed = currentBinding.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.isEmpty == false else {
            return
        }

        binding.currentBinding = trimmed
        shortcutBindings[id] = binding
    }
}
