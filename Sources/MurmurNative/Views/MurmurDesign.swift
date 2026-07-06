import AppKit
import SwiftUI

enum MurmurDesign {
    static let windowWidth: CGFloat = 860
    static let windowHeight: CGFloat = 640
    static let sidebarWidth: CGFloat = 160
    static let contentWidth: CGFloat = 768
    static let cornerRadius: CGFloat = 8

    static let text = Color(hex: 0x0F0F0F)
    static let background = Color(hex: 0xFBFBFB)
    static let backgroundUI = AppTheme.pink.palette.backgroundUI
    static let logoPrimary = AppTheme.pink.palette.swatchColor
    static let logoStroke = Color(hex: 0x382731)
    static let textStroke = AppTheme.pink.palette.textStroke
    static let overlayBar = AppTheme.pink.palette.overlayBar
    static let midGray = Color(hex: 0x808080)

    static func font(size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .custom("DM Sans", size: size).weight(weight)
    }
}

private struct MurmurThemePaletteKey: EnvironmentKey {
    static let defaultValue = AppTheme.pink.palette
}

private struct MurmurAppThemeKey: EnvironmentKey {
    static let defaultValue = AppTheme.pink
}

extension EnvironmentValues {
    var murmurAppTheme: AppTheme {
        get { self[MurmurAppThemeKey.self] }
        set { self[MurmurAppThemeKey.self] = newValue }
    }

    var murmurTheme: MurmurThemePalette {
        get { self[MurmurThemePaletteKey.self] }
        set { self[MurmurThemePaletteKey.self] = newValue }
    }
}

extension Color {
    init(hex: UInt32, opacity: Double = 1) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: opacity
        )
    }
}

struct MurmurSettingsGroup<Content: View>: View {
    let title: String?
    @ViewBuilder let content: Content

    init(_ title: String? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let title {
                Text(title)
                    .font(MurmurDesign.font(size: 12, weight: .medium))
                    .foregroundStyle(MurmurDesign.midGray)
                    .textCase(.uppercase)
                    .tracking(0.6)
                    .padding(.horizontal, 16)
            }

            VStack(spacing: 0) {
                content
            }
            .background(MurmurDesign.background)
            .clipShape(RoundedRectangle(cornerRadius: MurmurDesign.cornerRadius, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: MurmurDesign.cornerRadius, style: .continuous)
                    .stroke(MurmurDesign.midGray.opacity(0.2), lineWidth: 1)
            }
        }
    }
}

struct MurmurDivider: View {
    var body: some View {
        Rectangle()
            .fill(MurmurDesign.midGray.opacity(0.2))
            .frame(height: 1)
            .padding(.leading, 16)
    }
}

struct MurmurTooltipIcon: View {
    let text: String
    var size: CGFloat = 16

    @State private var isPresented = false

    var body: some View {
        Button {
            isPresented.toggle()
        } label: {
            MurmurHugeIcon(kind: .informationCircle, color: MurmurDesign.midGray, size: size)
                .frame(width: size + 6, height: size + 6)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            isPresented = hovering
        }
        .popover(isPresented: $isPresented, arrowEdge: .top) {
            Text(text)
                .font(MurmurDesign.font(size: 13, weight: .medium))
                .foregroundStyle(MurmurDesign.text)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 280, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
        }
        .accessibilityLabel(text)
        .help(text)
    }
}

struct MurmurSettingRow<Accessory: View>: View {
    let title: String
    let description: String?
    let descriptionDisplay: DescriptionDisplay
    @ViewBuilder let accessory: Accessory

    enum DescriptionDisplay {
        case tooltip
        case inline
    }

    init(
        _ title: String,
        description: String? = nil,
        descriptionDisplay: DescriptionDisplay = .tooltip,
        @ViewBuilder accessory: () -> Accessory
    ) {
        self.title = title
        self.description = description
        self.descriptionDisplay = descriptionDisplay
        self.accessory = accessory()
    }

    var body: some View {
        HStack(alignment: .center, spacing: 16) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(title)
                        .font(MurmurDesign.font(size: 14, weight: .medium))
                        .foregroundStyle(MurmurDesign.text)
                        .lineLimit(1)

                    if let description {
                        MurmurTooltipIcon(text: description)
                    }
                }

                if descriptionDisplay == .inline, let description {
                    Text(description)
                        .font(MurmurDesign.font(size: 13))
                        .foregroundStyle(MurmurDesign.text.opacity(0.6))
                        .lineLimit(2)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            accessory
                .layoutPriority(1)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .frame(minHeight: 42)
    }
}

struct MurmurToggle: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme

    @Binding var isOn: Bool

    var body: some View {
        Toggle("", isOn: $isOn)
            .toggleStyle(.switch)
            .labelsHidden()
            .tint(murmurTheme.logoPrimary(for: colorScheme))
    }
}

struct MurmurPill: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme

    let text: String
    var emphasized = false

    var body: some View {
        Text(text)
            .font(MurmurDesign.font(size: 14, weight: .semibold))
            .foregroundStyle(MurmurDesign.text)
            .lineLimit(1)
            .fixedSize(horizontal: true, vertical: false)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(emphasized ? murmurTheme.logoPrimary(for: colorScheme).opacity(0.2) : MurmurDesign.midGray.opacity(0.1))
            .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .stroke(emphasized ? murmurTheme.logoPrimary(for: colorScheme) : MurmurDesign.midGray.opacity(0.8), lineWidth: 1)
            }
    }
}

struct MurmurTextFieldStyle: TextFieldStyle {
    func _body(configuration: TextField<Self._Label>) -> some View {
        configuration
            .font(MurmurDesign.font(size: 14, weight: .semibold))
            .textFieldStyle(.plain)
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(MurmurDesign.midGray.opacity(0.1))
            .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .stroke(MurmurDesign.midGray.opacity(0.8), lineWidth: 1)
            }
    }
}

struct MurmurEmptyState: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme

    let title: String
    let description: String
    let systemImage: String

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: systemImage)
                .font(.system(size: 28, weight: .medium))
                .foregroundStyle(murmurTheme.logoPrimary(for: colorScheme))
                .frame(width: 52, height: 52)
                .background(murmurTheme.logoPrimary(for: colorScheme).opacity(0.18))
                .clipShape(Circle())

            VStack(spacing: 4) {
                Text(title)
                    .font(MurmurDesign.font(size: 16, weight: .semibold))
                    .foregroundStyle(MurmurDesign.text)
                Text(description)
                    .font(MurmurDesign.font(size: 14))
                    .foregroundStyle(MurmurDesign.text.opacity(0.6))
                    .multilineTextAlignment(.center)
                    .lineLimit(3)
            }
        }
        .frame(maxWidth: .infinity, minHeight: 220)
        .padding(24)
        .background(MurmurDesign.background)
        .clipShape(RoundedRectangle(cornerRadius: MurmurDesign.cornerRadius, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: MurmurDesign.cornerRadius, style: .continuous)
                .stroke(MurmurDesign.midGray.opacity(0.2), lineWidth: 1)
        }
    }
}

struct MurmurButtonStyle: ButtonStyle {
    @Environment(\.murmurTheme) private var murmurTheme

    var variant: Variant = .primary

    enum Variant {
        case primary
        case secondary
        case soft
    }

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(MurmurDesign.font(size: 14, weight: .medium))
            .foregroundStyle(foreground)
            .lineLimit(1)
            .fixedSize(horizontal: true, vertical: false)
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
            .background(background.opacity(configuration.isPressed ? 0.75 : 1))
            .clipShape(RoundedRectangle(cornerRadius: MurmurDesign.cornerRadius, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: MurmurDesign.cornerRadius, style: .continuous)
                    .stroke(border, lineWidth: 1)
            }
    }

    private var foreground: Color {
        switch variant {
        case .primary: .white
        case .secondary, .soft: MurmurDesign.text
        }
    }

    private var background: Color {
        switch variant {
        case .primary: murmurTheme.backgroundUI
        case .secondary: MurmurDesign.midGray.opacity(0.1)
        case .soft: murmurTheme.swatchColor.opacity(0.2)
        }
    }

    private var border: Color {
        switch variant {
        case .primary: murmurTheme.backgroundUI
        case .secondary: MurmurDesign.midGray.opacity(0.2)
        case .soft: .clear
        }
    }
}

struct MurmurLogoView: View {
    @Environment(\.murmurAppTheme) private var murmurAppTheme
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme

    var width: CGFloat = 132
    var height: CGFloat = 61

    var body: some View {
        logoMark
        .accessibilityLabel("Murmur for Mac")
    }

    @ViewBuilder
    private var logoMark: some View {
        if let logoImage = Self.logoImage(for: murmurAppTheme, colorScheme: colorScheme) {
            Image(nsImage: logoImage)
                .interpolation(.high)
                .antialiased(true)
                .resizable()
                .scaledToFit()
                .frame(width: width, height: height)
        } else {
            fallbackLogo
        }
    }

    private var fallbackLogo: some View {
        ZStack {
            ForEach(outlineOffsets.indices, id: \.self) { index in
                let offset = outlineOffsets[index]
                Text("Murmur")
                    .font(MurmurDesign.font(size: width * 0.225, weight: .black))
                    .foregroundStyle(murmurTheme.logoStroke(for: colorScheme))
                    .offset(x: offset.width, y: offset.height)
            }

            Text("Murmur")
                .font(MurmurDesign.font(size: width * 0.225, weight: .black))
                .foregroundStyle(murmurTheme.logoPrimary(for: colorScheme))
        }
        .frame(width: width, height: height)
    }

    private static func logoImage(for theme: AppTheme, colorScheme: ColorScheme) -> NSImage? {
        let variant = colorScheme == .dark ? "dark" : "light"
        let themedResource = "MurmurTextLogo-\(theme.rawValue)-\(variant)"
        if let themedURL = Bundle.main.url(forResource: themedResource, withExtension: "png") {
            return NSImage(contentsOf: themedURL)
        }

        if let fallbackURL = Bundle.main.url(forResource: "MurmurTextLogo", withExtension: "png") {
            return NSImage(contentsOf: fallbackURL)
        }

        return nil
    }

    private var outlineOffsets: [CGSize] {
        [
            CGSize(width: -1, height: 0),
            CGSize(width: 1, height: 0),
            CGSize(width: 0, height: -1),
            CGSize(width: 0, height: 1),
            CGSize(width: -1, height: -1),
            CGSize(width: 1, height: 1),
        ]
    }
}
