import AppKit
import SwiftUI

struct MurmurMenuBarIcon: View {
    var showsPermissionWarning = false

    var body: some View {
        Image(nsImage: MurmurMenuBarIconImage.make())
            .renderingMode(.template)
            .resizable()
            .frame(width: MurmurMenuBarIconImage.size.width, height: MurmurMenuBarIconImage.size.height)
            .overlay(alignment: .bottomTrailing) {
                if showsPermissionWarning {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .resizable()
                        .frame(width: 8, height: 8)
                }
            }
            .accessibilityLabel(showsPermissionWarning ? "Murmur — Accessibility permission required" : "Murmur")
    }
}

enum MurmurMenuBarIconImage {
    static let size = NSSize(width: 18, height: 18)

    static func make() -> NSImage {
        let image = NSImage(size: size, flipped: false) { rect in
            guard let context = NSGraphicsContext.current?.cgContext else {
                return false
            }

            let scale = min(rect.width, rect.height) / 24
            let side = 24 * scale
            let originX = rect.minX + (rect.width - side) / 2
            let originY = rect.minY + (rect.height - side) / 2

            context.saveGState()
            context.translateBy(x: originX, y: originY + side)
            context.scaleBy(x: scale, y: -scale)
            context.setStrokeColor(NSColor.black.cgColor)
            context.setLineWidth(1.5)
            context.setLineCap(.round)
            context.setLineJoin(.round)

            context.addPath(CGPath(roundedRect: CGRect(x: 7, y: 2, width: 10, height: 14), cornerWidth: 5, cornerHeight: 5, transform: nil))
            context.strokePath()

            context.beginPath()
            context.move(to: CGPoint(x: 17, y: 7))
            context.addLine(to: CGPoint(x: 14, y: 7))
            context.move(to: CGPoint(x: 17, y: 11))
            context.addLine(to: CGPoint(x: 14, y: 11))
            context.strokePath()

            context.beginPath()
            context.move(to: CGPoint(x: 20, y: 11))
            context.addCurve(
                to: CGPoint(x: 12, y: 19),
                control1: CGPoint(x: 20, y: 15.4183),
                control2: CGPoint(x: 16.4183, y: 19)
            )
            context.move(to: CGPoint(x: 12, y: 19))
            context.addCurve(
                to: CGPoint(x: 4, y: 11),
                control1: CGPoint(x: 7.58172, y: 19),
                control2: CGPoint(x: 4, y: 15.4183)
            )
            context.move(to: CGPoint(x: 12, y: 19))
            context.addLine(to: CGPoint(x: 12, y: 22))
            context.move(to: CGPoint(x: 12, y: 22))
            context.addLine(to: CGPoint(x: 15, y: 22))
            context.move(to: CGPoint(x: 12, y: 22))
            context.addLine(to: CGPoint(x: 9, y: 22))
            context.strokePath()

            context.restoreGState()
            return true
        }
        image.isTemplate = true
        return image
    }
}
