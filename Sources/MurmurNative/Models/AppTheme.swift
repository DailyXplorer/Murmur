import SwiftUI

enum AppTheme: String, CaseIterable, Codable, Equatable, Identifiable {
    case pink
    case blue
    case green
    case purple
    case orange
    case gray

    var id: String { rawValue }

    var title: String {
        switch self {
        case .pink: "Pink"
        case .blue: "Blue"
        case .green: "Green"
        case .purple: "Purple"
        case .orange: "Orange"
        case .gray: "Gray"
        }
    }

    var palette: MurmurThemePalette {
        switch self {
        case .pink:
            MurmurThemePalette(
                primaryHex: 0xFAA2CA,
                primaryDarkHex: 0xF28CBB,
                uiHex: 0xDA5893,
                strokeHex: 0x382731,
                strokeDarkHex: 0xFAD1ED,
                textStrokeHex: 0xF6F6F6,
                overlayBarHex: 0xFFE5EE
            )
        case .blue:
            MurmurThemePalette(
                primaryHex: 0x7CB7FF,
                primaryDarkHex: 0x75AEF0,
                uiHex: 0x327FD9,
                strokeHex: 0x17324D,
                strokeDarkHex: 0xD7E8FF,
                textStrokeHex: 0xF4F8FF,
                overlayBarHex: 0xE0EFFF
            )
        case .green:
            MurmurThemePalette(
                primaryHex: 0x65D6A5,
                primaryDarkHex: 0x6BD19F,
                uiHex: 0x239B6D,
                strokeHex: 0x123D2C,
                strokeDarkHex: 0xD2F6E4,
                textStrokeHex: 0xF1FFF8,
                overlayBarHex: 0xD9F7E8
            )
        case .purple:
            MurmurThemePalette(
                primaryHex: 0xB9A2FF,
                primaryDarkHex: 0xAA93F5,
                uiHex: 0x7658DA,
                strokeHex: 0x2F2754,
                strokeDarkHex: 0xE7DDFF,
                textStrokeHex: 0xFBF8FF,
                overlayBarHex: 0xEEE7FF
            )
        case .orange:
            MurmurThemePalette(
                primaryHex: 0xFFB06F,
                primaryDarkHex: 0xF6A765,
                uiHex: 0xD9792F,
                strokeHex: 0x4A2A16,
                strokeDarkHex: 0xFFE2C6,
                textStrokeHex: 0xFFF8F1,
                overlayBarHex: 0xFFE8D2
            )
        case .gray:
            MurmurThemePalette(
                primaryHex: 0xA9ADB7,
                primaryDarkHex: 0xB3B8C3,
                uiHex: 0x69707D,
                strokeHex: 0x272A31,
                strokeDarkHex: 0xE0E3E9,
                textStrokeHex: 0xF7F8FA,
                overlayBarHex: 0xECEFF3
            )
        }
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        let rawValue = try container.decode(String.self).lowercased()
        self = AppTheme(rawValue: rawValue) ?? .pink
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }
}

struct MurmurThemePalette: Equatable {
    let primaryHex: UInt32
    let primaryDarkHex: UInt32
    let uiHex: UInt32
    let strokeHex: UInt32
    let strokeDarkHex: UInt32
    let textStrokeHex: UInt32
    let overlayBarHex: UInt32

    var swatchColor: Color { Color(hex: primaryHex) }
    var backgroundUI: Color { Color(hex: uiHex) }
    var textStroke: Color { Color(hex: textStrokeHex) }
    var overlayBar: Color { Color(hex: overlayBarHex) }

    func logoPrimary(for colorScheme: ColorScheme) -> Color {
        Color(hex: colorScheme == .dark ? primaryDarkHex : primaryHex)
    }

    func logoStroke(for colorScheme: ColorScheme) -> Color {
        Color(hex: colorScheme == .dark ? strokeDarkHex : strokeHex)
    }
}
