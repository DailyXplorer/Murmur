import Foundation
import SwiftUI

enum SidebarHugeIconKind: CaseIterable {
    case hand
    case cpu
    case cog
    case history
    case sparkles
    case flaskConical

    init(section: AppSection) {
        switch section {
        case .general:
            self = .hand
        case .models:
            self = .cpu
        case .advanced:
            self = .cog
        case .history:
            self = .history
        case .postProcessing:
            self = .sparkles
        case .debug:
            self = .flaskConical
        }
    }

    var pathData: [String] {
        switch self {
        case .hand:
            [
                "M18.5 11.5V8.75C18.5 7.7835 19.2835 7 20.25 7C21.2165 7 22 7.7835 22 8.75V14.8C22 18.7765 18.7764 22 14.8 22H12.0588M8 14L5.61507 11.2176C5.22468 10.7621 4.65476 10.5 4.05489 10.5H3.91898C2.85916 10.5 2 11.3592 2 12.419C2 12.7978 2.11214 13.1682 2.32229 13.4834L5.7812 18.6718C7.16738 20.7511 9.50102 22 12 22",
                "M18.5 8.5V5.75C18.5 4.7835 17.7165 4 16.75 4C15.7835 4 15 4.7835 15 5.75V10.5",
                "M15 10.5V3.75C15 2.7835 14.2165 2 13.25 2C12.2835 2 11.5 2.7835 11.5 3.75V10",
                "M11.5 10V5.75C11.5 4.7835 10.7165 4 9.75 4C8.7835 4 8 4.7835 8 5.75V14",
            ]
        case .cpu:
            [
                "M4 12C4 8.22876 4 6.34315 5.17157 5.17157C6.34315 4 8.22876 4 12 4C15.7712 4 17.6569 4 18.8284 5.17157C20 6.34315 20 8.22876 20 12C20 15.7712 20 17.6569 18.8284 18.8284C17.6569 20 15.7712 20 12 20C8.22876 20 6.34315 20 5.17157 18.8284C4 17.6569 4 15.7712 4 12Z",
                "M9.5 2V4",
                "M14.5 2V4",
                "M9.5 20V22",
                "M14.5 20V22",
                "M13 9L9 13",
                "M15 13L13 15",
                "M22 14.5L20 14.5",
                "M4 9.5L2 9.5",
                "M4 14.5L2 14.5",
                "M22 9.5L20 9.5",
            ]
        case .cog:
            [
                "M19.995 12C19.995 7.58172 16.4132 4 11.995 4C7.5767 4 3.99498 7.58172 3.99498 12C3.99498 16.4182 7.5767 20 11.995 20C16.4132 20 19.995 16.4182 19.995 12Z",
                "M13.995 12C13.995 10.8954 13.0995 10 11.995 10C10.8904 10 9.99498 10.8954 9.99498 12C9.99498 13.1046 10.8904 14 11.995 14C13.0995 14 13.995 13.1046 13.995 12Z",
                "M11.995 4V2M11.995 20V22M15.9957 5.0693L16.9957 3.33725M6.99566 20.6578L10.9967 13.7248M18.9257 7.99558L20.6577 6.99558M5.06929 15.9956L3.33724 16.9956M22 11.9949L13.995 11.9949M4.00003 11.9949H2.00003M18.9307 15.9955L20.6628 16.9955M5.07431 7.99554L3.34226 6.99554M16.0044 18.9256L17.0044 20.6577M7.00437 3.33718L10.9278 10.1435",
            ]
        case .history:
            [
                "M4.43186 14.9656C5.65759 18.4791 9.00032 21 12.9318 21C17.9024 21 21.9318 16.9706 21.9318 12C21.9318 7.02944 17.9024 3 12.9318 3C9.23111 3 5.83124 5.6756 4.62227 8.5",
                "M12.9319 7V12L15.9319 14",
                "M8.43054 8.74363C8.43054 8.74363 4.74691 9.3026 4.1879 8.7436C3.62888 8.1846 4.18791 4.50098 4.18791 4.50098",
            ]
        case .sparkles:
            [
                "M15 2L15.5387 4.39157C15.9957 6.42015 17.5798 8.00431 19.6084 8.46127L22 9L19.6084 9.53873C17.5798 9.99569 15.9957 11.5798 15.5387 13.6084L15 16L14.4613 13.6084C14.0043 11.5798 12.4202 9.99569 10.3916 9.53873L8 9L10.3916 8.46127C12.4201 8.00431 14.0043 6.42015 14.4613 4.39158L15 2Z",
                "M7 12L7.38481 13.7083C7.71121 15.1572 8.84275 16.2888 10.2917 16.6152L12 17L10.2917 17.3848C8.84275 17.7112 7.71121 18.8427 7.38481 20.2917L7 22L6.61519 20.2917C6.28879 18.8427 5.15725 17.7112 3.70827 17.3848L2 17L3.70827 16.6152C5.15725 16.2888 6.28879 15.1573 6.61519 13.7083L7 12Z",
            ]
        case .flaskConical:
            [
                "M14.4725 2V7.86783C14.4725 8.43028 14.4725 8.71151 14.5473 8.9786C14.6221 9.2457 14.7679 9.48521 15.0594 9.96422L18.6743 15.9036C20.3166 18.602 21.1377 19.9512 20.5716 20.9756C20.0054 22 18.4386 22 15.305 22H8.69496C5.56136 22 3.99455 22 3.42841 20.9756C2.86227 19.9512 3.68343 18.602 5.32575 15.9036L8.94061 9.96422C9.23215 9.48521 9.37792 9.2457 9.45269 8.9786C9.52746 8.71151 9.52746 8.43028 9.52746 7.86783V2",
                "M8 2H16",
                "M19 16.7232C18.4694 16.8939 17.8143 17 17 17C14 17 11 14 8 14C7.65244 14 7.31998 14.0181 7 14.0512",
            ]
        }
    }

    var paths: [Path] {
        HugeIconPathParser.paths(from: pathData)
    }
}

struct SidebarHugeIcon: View {
    let kind: SidebarHugeIconKind
    let color: Color

    var body: some View {
        Canvas { context, size in
            let scale = min(size.width, size.height) / 24
            let side = 24 * scale
            let transform = CGAffineTransform(
                translationX: (size.width - side) / 2,
                y: (size.height - side) / 2
            )
            .scaledBy(x: scale, y: scale)
            let style = StrokeStyle(lineWidth: 1.8 * scale, lineCap: .round, lineJoin: .round)

            for path in kind.paths {
                context.stroke(path.applying(transform), with: .color(color), style: style)
            }
        }
        .frame(width: 24, height: 24)
        .accessibilityHidden(true)
    }
}

enum HugeIconPathParser {
    private enum Token: Equatable {
        case command(Character)
        case number(CGFloat)
    }

    static func paths(from pathData: [String]) -> [Path] {
        pathData.map(parse)
    }

    static func parse(_ data: String) -> Path {
        let tokens = tokenize(data)
        var index = 0
        var path = Path()
        var command: Character?
        var currentPoint = CGPoint.zero
        var subpathStart = CGPoint.zero
        var firstMoveInCommand = false

        func nextNumber() -> CGFloat? {
            guard index < tokens.count, case let .number(value) = tokens[index] else {
                return nil
            }
            index += 1
            return value
        }

        func hasNumberAhead() -> Bool {
            index < tokens.count && {
                if case .number = tokens[index] {
                    return true
                }
                return false
            }()
        }

        func point(x: CGFloat, y: CGFloat, relative: Bool) -> CGPoint {
            relative ? CGPoint(x: currentPoint.x + x, y: currentPoint.y + y) : CGPoint(x: x, y: y)
        }

        while index < tokens.count {
            if case let .command(newCommand) = tokens[index] {
                command = newCommand
                firstMoveInCommand = newCommand == "M" || newCommand == "m"
                index += 1
            }

            guard let currentCommand = command else {
                break
            }

            let relative = currentCommand.isLowercase
            switch currentCommand.uppercased() {
            case "M":
                while let x = nextNumber(), let y = nextNumber() {
                    let nextPoint = point(x: x, y: y, relative: relative)
                    if firstMoveInCommand {
                        path.move(to: nextPoint)
                        subpathStart = nextPoint
                        firstMoveInCommand = false
                    } else {
                        path.addLine(to: nextPoint)
                    }
                    currentPoint = nextPoint
                    if !hasNumberAhead() { break }
                }
            case "L":
                while let x = nextNumber(), let y = nextNumber() {
                    let nextPoint = point(x: x, y: y, relative: relative)
                    path.addLine(to: nextPoint)
                    currentPoint = nextPoint
                    if !hasNumberAhead() { break }
                }
            case "H":
                while let x = nextNumber() {
                    let nextX = relative ? currentPoint.x + x : x
                    let nextPoint = CGPoint(x: nextX, y: currentPoint.y)
                    path.addLine(to: nextPoint)
                    currentPoint = nextPoint
                    if !hasNumberAhead() { break }
                }
            case "V":
                while let y = nextNumber() {
                    let nextY = relative ? currentPoint.y + y : y
                    let nextPoint = CGPoint(x: currentPoint.x, y: nextY)
                    path.addLine(to: nextPoint)
                    currentPoint = nextPoint
                    if !hasNumberAhead() { break }
                }
            case "C":
                while let x1 = nextNumber(), let y1 = nextNumber(),
                      let x2 = nextNumber(), let y2 = nextNumber(),
                      let x = nextNumber(), let y = nextNumber() {
                    let control1 = point(x: x1, y: y1, relative: relative)
                    let control2 = point(x: x2, y: y2, relative: relative)
                    let nextPoint = point(x: x, y: y, relative: relative)
                    path.addCurve(to: nextPoint, control1: control1, control2: control2)
                    currentPoint = nextPoint
                    if !hasNumberAhead() { break }
                }
            case "Z":
                path.closeSubpath()
                currentPoint = subpathStart
                command = nil
            default:
                command = nil
            }
        }

        return path
    }

    private static func tokenize(_ data: String) -> [Token] {
        var tokens: [Token] = []
        var numberBuffer = ""

        func flushNumber() {
            guard numberBuffer.isEmpty == false else {
                return
            }
            if let value = Double(numberBuffer) {
                tokens.append(.number(CGFloat(value)))
            }
            numberBuffer = ""
        }

        for character in data {
            if character.isLetter {
                flushNumber()
                tokens.append(.command(character))
            } else if character == "-" || character == "+" {
                if numberBuffer.isEmpty || numberBuffer.last == "e" || numberBuffer.last == "E" {
                    numberBuffer.append(character)
                } else {
                    flushNumber()
                    numberBuffer.append(character)
                }
            } else if character.isNumber || character == "." {
                numberBuffer.append(character)
            } else if character == "e" || character == "E" {
                numberBuffer.append(character)
            } else {
                flushNumber()
            }
        }

        flushNumber()
        return tokens
    }
}
