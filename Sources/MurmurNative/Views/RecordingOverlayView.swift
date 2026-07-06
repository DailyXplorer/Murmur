import SwiftUI

struct RecordingOverlayView: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme

    @ObservedObject var viewModel: RecordingOverlayViewModel
    let onCancel: () -> Void
    @State private var cancelHovering = false

    var body: some View {
        HStack(spacing: 0) {
            overlayIcon
                .frame(width: 24, height: 24)

            Spacer(minLength: 0)

            middleContent
                .frame(maxWidth: .infinity)

            Spacer(minLength: 0)

            cancelButton
                .frame(width: 24, height: 24)
        }
        .padding(6)
        .frame(width: 172, height: 36)
        .background(Color.black.opacity(0.8))
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .font(MurmurDesign.font(size: 12))
    }

    private var overlayIcon: some View {
        RecordingOverlayHugeIcon(
            kind: viewModel.state == .recording ? .microphone : .speechToText,
            color: murmurTheme.logoPrimary(for: colorScheme)
        )
    }

    @ViewBuilder
    private var middleContent: some View {
        switch viewModel.state {
        case .recording:
            HStack(alignment: .bottom, spacing: 3) {
                ForEach(viewModel.levels.indices, id: \.self) { index in
                    let value = max(0, min(1, viewModel.levels[index]))
                    RoundedRectangle(cornerRadius: 2, style: .continuous)
                        .fill(murmurTheme.overlayBar)
                        .frame(width: 6, height: CGFloat(min(20, 4 + pow(Double(value), 0.7) * 16)))
                        .opacity(max(0.2, Double(value) * 1.7))
                        .animation(.easeOut(duration: 0.06), value: value)
                }
            }
            .frame(height: 24, alignment: .bottom)
        case .transcribing, .processing:
            PulsingOverlayText(viewModel.state.title)
        }
    }

    @ViewBuilder
    private var cancelButton: some View {
        if viewModel.state == .recording {
            Button(action: onCancel) {
                RecordingOverlayHugeIcon(
                    kind: .cancelCircle,
                    color: murmurTheme.logoPrimary(for: colorScheme)
                )
                    .frame(width: 24, height: 24)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .background(
                Circle()
                    .fill(cancelHovering ? murmurTheme.logoPrimary(for: colorScheme).opacity(0.2) : .clear)
            )
            .clipShape(Circle())
            .scaleEffect(cancelHovering ? 1.05 : 1)
            .animation(.easeOut(duration: 0.15), value: cancelHovering)
            .onHover { cancelHovering = $0 }
        } else {
            Color.clear
        }
    }
}

private struct PulsingOverlayText: View {
    let text: String
    @State private var emphasized = false

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text)
            .font(MurmurDesign.font(size: 12))
            .foregroundStyle(.white)
            .opacity(emphasized ? 1 : 0.6)
            .onAppear {
                emphasized = false
                withAnimation(.easeInOut(duration: 1.5).repeatForever(autoreverses: true)) {
                    emphasized = true
                }
            }
    }
}

private struct RecordingOverlayHugeIcon: View {
    enum Kind {
        case microphone
        case speechToText
        case cancelCircle
    }

    let kind: Kind
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

            for path in paths {
                context.stroke(path.applying(transform), with: .color(color), style: style)
            }
        }
        .frame(width: 24, height: 24)
        .accessibilityHidden(true)
    }

    private var paths: [Path] {
        switch kind {
        case .microphone:
            Self.microphonePaths
        case .speechToText:
            Self.speechToTextPaths
        case .cancelCircle:
            Self.cancelCirclePaths
        }
    }

    private static let microphonePaths: [Path] = {
        var capsule = Path(
            roundedRect: CGRect(x: 7, y: 2, width: 10, height: 14),
            cornerSize: CGSize(width: 5, height: 5)
        )

        var sideLines = Path()
        sideLines.move(to: CGPoint(x: 17, y: 7))
        sideLines.addLine(to: CGPoint(x: 14, y: 7))
        sideLines.move(to: CGPoint(x: 17, y: 11))
        sideLines.addLine(to: CGPoint(x: 14, y: 11))

        var base = Path()
        base.move(to: CGPoint(x: 20, y: 11))
        base.addCurve(
            to: CGPoint(x: 12, y: 19),
            control1: CGPoint(x: 20, y: 15.4183),
            control2: CGPoint(x: 16.4183, y: 19)
        )
        base.move(to: CGPoint(x: 12, y: 19))
        base.addCurve(
            to: CGPoint(x: 4, y: 11),
            control1: CGPoint(x: 7.58172, y: 19),
            control2: CGPoint(x: 4, y: 15.4183)
        )
        base.move(to: CGPoint(x: 12, y: 19))
        base.addLine(to: CGPoint(x: 12, y: 22))
        base.move(to: CGPoint(x: 12, y: 22))
        base.addLine(to: CGPoint(x: 15, y: 22))
        base.move(to: CGPoint(x: 12, y: 22))
        base.addLine(to: CGPoint(x: 9, y: 22))

        return [capsule, sideLines, base]
    }()

    private static let speechToTextPaths: [Path] = {
        var textLine = Path()
        textLine.move(to: CGPoint(x: 16, y: 17))
        textLine.addLine(to: CGPoint(x: 10, y: 17))

        var waveform = Path()
        waveform.move(to: CGPoint(x: 8, y: 4))
        waveform.addLine(to: CGPoint(x: 8, y: 8))
        waveform.move(to: CGPoint(x: 5, y: 2))
        waveform.addLine(to: CGPoint(x: 5, y: 10))
        waveform.move(to: CGPoint(x: 2, y: 5))
        waveform.addLine(to: CGPoint(x: 2, y: 7))
        waveform.move(to: CGPoint(x: 11, y: 5))
        waveform.addLine(to: CGPoint(x: 11, y: 7))

        var bubble = Path()
        bubble.move(to: CGPoint(x: 4.00006, y: 13))
        bubble.addCurve(
            to: CGPoint(x: 5.31802, y: 20.6124),
            control1: CGPoint(x: 4.00172, y: 17.1517),
            control2: CGPoint(x: 4.04756, y: 19.2749)
        )
        bubble.addCurve(
            to: CGPoint(x: 13, y: 22),
            control1: CGPoint(x: 6.63604, y: 22),
            control2: CGPoint(x: 8.75736, y: 22)
        )
        bubble.addLine(to: CGPoint(x: 13.45, y: 22))
        bubble.addCurve(
            to: CGPoint(x: 20.4225, y: 20.8649),
            control1: CGPoint(x: 17.2568, y: 22),
            control2: CGPoint(x: 19.1601, y: 22)
        )
        bubble.addCurve(
            to: CGPoint(x: 20.9218, y: 20.3393),
            control1: CGPoint(x: 20.6018, y: 20.7038),
            control2: CGPoint(x: 20.7687, y: 20.528)
        )
        bubble.addCurve(
            to: CGPoint(x: 22, y: 12.9989),
            control1: CGPoint(x: 22, y: 19.0103),
            control2: CGPoint(x: 22, y: 17.0065)
        )
        bubble.addCurve(
            to: CGPoint(x: 20.9218, y: 5.65845),
            control1: CGPoint(x: 22, y: 8.99125),
            control2: CGPoint(x: 22, y: 6.98744)
        )
        bubble.addCurve(
            to: CGPoint(x: 20.4225, y: 5.1328),
            control1: CGPoint(x: 20.7687, y: 5.46974),
            control2: CGPoint(x: 20.6018, y: 5.29398)
        )
        bubble.addCurve(
            to: CGPoint(x: 14.8, y: 4),
            control1: CGPoint(x: 19.3191, y: 4.14066),
            control2: CGPoint(x: 17.7259, y: 4.01573)
        )
        bubble.addLine(to: CGPoint(x: 14, y: 4))

        return [textLine, waveform, bubble]
    }()

    private static let cancelCirclePaths: [Path] = {
        var circle = Path(ellipseIn: CGRect(x: 2, y: 2, width: 20, height: 20))

        var cross = Path()
        cross.move(to: CGPoint(x: 14.9994, y: 15))
        cross.addLine(to: CGPoint(x: 9, y: 9))
        cross.move(to: CGPoint(x: 9.00064, y: 15))
        cross.addLine(to: CGPoint(x: 15, y: 9))

        return [circle, cross]
    }()
}
