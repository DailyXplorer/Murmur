import AppKit
import Carbon
import Foundation

struct GlobalShortcutDescriptor: Equatable {
    let keyCode: CGKeyCode
    let requiredFlags: CGEventFlags

    static let optionSpace = GlobalShortcutDescriptor(
        keyCode: 49,
        requiredFlags: .maskAlternate
    )

    static func parse(_ binding: String) -> GlobalShortcutDescriptor? {
        var requiredFlags = CGEventFlags()
        var keyCode: CGKeyCode?

        for part in ShortcutBinding.normalizedParts(binding) {
            switch part {
            case "cmd", "command", "meta":
                requiredFlags.insert(.maskCommand)
            case "ctrl", "control":
                requiredFlags.insert(.maskControl)
            case "option", "alt":
                requiredFlags.insert(.maskAlternate)
            case "shift":
                requiredFlags.insert(.maskShift)
            default:
                guard keyCode == nil,
                      let parsedKeyCode = Self.keyCode(for: part)
                else {
                    return nil
                }
                keyCode = parsedKeyCode
            }
        }

        guard let keyCode else {
            return nil
        }
        return GlobalShortcutDescriptor(keyCode: keyCode, requiredFlags: requiredFlags)
    }

    static func bindingString(keyCode: CGKeyCode, modifierFlags: NSEvent.ModifierFlags) -> String? {
        guard let key = bindingKey(for: keyCode) else {
            return nil
        }

        let modifiers = modifierParts(from: modifierFlags)
        guard modifiers.isEmpty == false || key == "escape" else {
            return nil
        }

        return (modifiers + [key]).joined(separator: "+")
    }

    static func bindingsConflict(_ lhs: String, _ rhs: String) -> Bool {
        guard let left = parse(lhs), let right = parse(rhs) else {
            return false
        }

        return left == right
    }

    private static func modifierParts(from flags: NSEvent.ModifierFlags) -> [String] {
        let normalized = flags.intersection(.deviceIndependentFlagsMask)
        var parts: [String] = []

        if normalized.contains(.command) {
            parts.append("cmd")
        }
        if normalized.contains(.control) {
            parts.append("control")
        }
        if normalized.contains(.option) {
            parts.append("option")
        }
        if normalized.contains(.shift) {
            parts.append("shift")
        }

        return parts
    }

    private static func bindingKey(for keyCode: CGKeyCode) -> String? {
        switch Int(keyCode) {
        case kVK_Space:
            return "space"
        case kVK_Escape:
            return "escape"
        case kVK_Return:
            return "return"
        case kVK_Tab:
            return "tab"
        case kVK_Delete:
            return "delete"
        case kVK_ANSI_A: return "a"
        case kVK_ANSI_B: return "b"
        case kVK_ANSI_C: return "c"
        case kVK_ANSI_D: return "d"
        case kVK_ANSI_E: return "e"
        case kVK_ANSI_F: return "f"
        case kVK_ANSI_G: return "g"
        case kVK_ANSI_H: return "h"
        case kVK_ANSI_I: return "i"
        case kVK_ANSI_J: return "j"
        case kVK_ANSI_K: return "k"
        case kVK_ANSI_L: return "l"
        case kVK_ANSI_M: return "m"
        case kVK_ANSI_N: return "n"
        case kVK_ANSI_O: return "o"
        case kVK_ANSI_P: return "p"
        case kVK_ANSI_Q: return "q"
        case kVK_ANSI_R: return "r"
        case kVK_ANSI_S: return "s"
        case kVK_ANSI_T: return "t"
        case kVK_ANSI_U: return "u"
        case kVK_ANSI_V: return "v"
        case kVK_ANSI_W: return "w"
        case kVK_ANSI_X: return "x"
        case kVK_ANSI_Y: return "y"
        case kVK_ANSI_Z: return "z"
        case kVK_ANSI_0: return "0"
        case kVK_ANSI_1: return "1"
        case kVK_ANSI_2: return "2"
        case kVK_ANSI_3: return "3"
        case kVK_ANSI_4: return "4"
        case kVK_ANSI_5: return "5"
        case kVK_ANSI_6: return "6"
        case kVK_ANSI_7: return "7"
        case kVK_ANSI_8: return "8"
        case kVK_ANSI_9: return "9"
        default:
            return nil
        }
    }

    private static func keyCode(for part: String) -> CGKeyCode? {
        switch part {
        case "space":
            return CGKeyCode(kVK_Space)
        case "escape", "esc":
            return CGKeyCode(kVK_Escape)
        case "return", "enter":
            return CGKeyCode(kVK_Return)
        case "tab":
            return CGKeyCode(kVK_Tab)
        case "delete", "backspace":
            return CGKeyCode(kVK_Delete)
        case "a": return CGKeyCode(kVK_ANSI_A)
        case "b": return CGKeyCode(kVK_ANSI_B)
        case "c": return CGKeyCode(kVK_ANSI_C)
        case "d": return CGKeyCode(kVK_ANSI_D)
        case "e": return CGKeyCode(kVK_ANSI_E)
        case "f": return CGKeyCode(kVK_ANSI_F)
        case "g": return CGKeyCode(kVK_ANSI_G)
        case "h": return CGKeyCode(kVK_ANSI_H)
        case "i": return CGKeyCode(kVK_ANSI_I)
        case "j": return CGKeyCode(kVK_ANSI_J)
        case "k": return CGKeyCode(kVK_ANSI_K)
        case "l": return CGKeyCode(kVK_ANSI_L)
        case "m": return CGKeyCode(kVK_ANSI_M)
        case "n": return CGKeyCode(kVK_ANSI_N)
        case "o": return CGKeyCode(kVK_ANSI_O)
        case "p": return CGKeyCode(kVK_ANSI_P)
        case "q": return CGKeyCode(kVK_ANSI_Q)
        case "r": return CGKeyCode(kVK_ANSI_R)
        case "s": return CGKeyCode(kVK_ANSI_S)
        case "t": return CGKeyCode(kVK_ANSI_T)
        case "u": return CGKeyCode(kVK_ANSI_U)
        case "v": return CGKeyCode(kVK_ANSI_V)
        case "w": return CGKeyCode(kVK_ANSI_W)
        case "x": return CGKeyCode(kVK_ANSI_X)
        case "y": return CGKeyCode(kVK_ANSI_Y)
        case "z": return CGKeyCode(kVK_ANSI_Z)
        case "0": return CGKeyCode(kVK_ANSI_0)
        case "1": return CGKeyCode(kVK_ANSI_1)
        case "2": return CGKeyCode(kVK_ANSI_2)
        case "3": return CGKeyCode(kVK_ANSI_3)
        case "4": return CGKeyCode(kVK_ANSI_4)
        case "5": return CGKeyCode(kVK_ANSI_5)
        case "6": return CGKeyCode(kVK_ANSI_6)
        case "7": return CGKeyCode(kVK_ANSI_7)
        case "8": return CGKeyCode(kVK_ANSI_8)
        case "9": return CGKeyCode(kVK_ANSI_9)
        default:
            return nil
        }
    }
}

struct GlobalShortcutRegistration: Equatable {
    var bindingID: String
    var descriptor: GlobalShortcutDescriptor
}

struct GlobalShortcutMatcher {
    private(set) var isPressed = false
    let descriptor: GlobalShortcutDescriptor

    mutating func handle(type: CGEventType, keyCode: CGKeyCode, flags: CGEventFlags) -> GlobalShortcutMatch {
        guard keyCode == descriptor.keyCode else {
            return .passThrough
        }

        switch type {
        case .keyDown:
            guard flags.matchesExactly(descriptor.requiredFlags) else {
                return .passThrough
            }

            if isPressed {
                return .consume
            }

            isPressed = true
            return .pressed
        case .keyUp:
            guard isPressed else {
                return .passThrough
            }

            isPressed = false
            return .released
        default:
            return .passThrough
        }
    }
}

enum GlobalShortcutMatch: Equatable {
    case passThrough
    case consume
    case pressed
    case released

    var shouldConsume: Bool {
        self != .passThrough
    }
}

enum GlobalShortcutServiceError: LocalizedError {
    case eventTapUnavailable

    var errorDescription: String? {
        switch self {
        case .eventTapUnavailable:
            "Unable to monitor the global shortcut. Accessibility permission may be required."
        }
    }
}

final class GlobalShortcutService {
    private var eventTap: CFMachPort?
    private var runLoopSource: CFRunLoopSource?
    private var matchers: [String: GlobalShortcutMatcher] = [
        ShortcutBinding.transcribeID: GlobalShortcutMatcher(descriptor: .optionSpace)
    ]
    private var onPressed: (@Sendable (String) -> Void)?
    private var onReleased: (@Sendable (String) -> Void)?

    var isRunning: Bool {
        eventTap != nil
    }

    func start(
        descriptor: GlobalShortcutDescriptor = .optionSpace,
        onPressed: @escaping @Sendable () -> Void,
        onReleased: @escaping @Sendable () -> Void
    ) throws {
        try start(
            registrations: [
                GlobalShortcutRegistration(bindingID: ShortcutBinding.transcribeID, descriptor: descriptor)
            ],
            onPressed: { _ in onPressed() },
            onReleased: { _ in onReleased() }
        )
    }

    func start(
        registrations: [GlobalShortcutRegistration],
        onPressed: @escaping @Sendable (String) -> Void,
        onReleased: @escaping @Sendable (String) -> Void
    ) throws {
        stop()

        let activeRegistrations = registrations.isEmpty
            ? [GlobalShortcutRegistration(bindingID: ShortcutBinding.transcribeID, descriptor: .optionSpace)]
            : registrations
        matchers = Dictionary(uniqueKeysWithValues: activeRegistrations.map {
            ($0.bindingID, GlobalShortcutMatcher(descriptor: $0.descriptor))
        })
        self.onPressed = onPressed
        self.onReleased = onReleased

        let eventMask = (1 << CGEventType.keyDown.rawValue) | (1 << CGEventType.keyUp.rawValue)
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: .defaultTap,
            eventsOfInterest: CGEventMask(eventMask),
            callback: { proxy, type, event, refcon in
                guard let refcon else {
                    return Unmanaged.passUnretained(event)
                }

                let service = Unmanaged<GlobalShortcutService>.fromOpaque(refcon).takeUnretainedValue()
                return service.handle(proxy: proxy, type: type, event: event)
            },
            userInfo: Unmanaged.passUnretained(self).toOpaque()
        ) else {
            throw GlobalShortcutServiceError.eventTapUnavailable
        }

        guard let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0) else {
            CFMachPortInvalidate(tap)
            throw GlobalShortcutServiceError.eventTapUnavailable
        }

        eventTap = tap
        runLoopSource = source
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
    }

    func stop() {
        if let runLoopSource {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), runLoopSource, .commonModes)
        }
        if let eventTap {
            CFMachPortInvalidate(eventTap)
        }

        eventTap = nil
        runLoopSource = nil
        onPressed = nil
        onReleased = nil
        matchers = [
            ShortcutBinding.transcribeID: GlobalShortcutMatcher(descriptor: .optionSpace)
        ]
    }

    private func handle(proxy: CGEventTapProxy, type: CGEventType, event: CGEvent) -> Unmanaged<CGEvent>? {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            if let eventTap {
                CGEvent.tapEnable(tap: eventTap, enable: true)
            }
            return Unmanaged.passUnretained(event)
        }

        let keyCode = CGKeyCode(event.getIntegerValueField(.keyboardEventKeycode))
        var handled: (bindingID: String, match: GlobalShortcutMatch)?
        for bindingID in matchers.keys.sorted() {
            guard var matcher = matchers[bindingID] else {
                continue
            }
            let match = matcher.handle(type: type, keyCode: keyCode, flags: event.flags)
            matchers[bindingID] = matcher
            if match.shouldConsume {
                handled = (bindingID, match)
                break
            }
        }

        guard let handled else {
            return Unmanaged.passUnretained(event)
        }

        switch handled.match {
        case .pressed:
            let handler = onPressed
            let bindingID = handled.bindingID
            DispatchQueue.main.async {
                handler?(bindingID)
            }
        case .released:
            let handler = onReleased
            let bindingID = handled.bindingID
            DispatchQueue.main.async {
                handler?(bindingID)
            }
        case .consume, .passThrough:
            break
        }

        return nil
    }
}

private extension CGEventFlags {
    func matchesExactly(_ required: CGEventFlags) -> Bool {
        let relevant: CGEventFlags = [.maskAlternate, .maskCommand, .maskControl, .maskShift]
        return intersection(relevant) == required
    }
}
