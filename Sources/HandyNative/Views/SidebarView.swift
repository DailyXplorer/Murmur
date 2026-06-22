import SwiftUI

struct SidebarView: View {
    @Environment(\.colorScheme) private var colorScheme
    @EnvironmentObject private var appModel: AppModel
    @Environment(\.handyTheme) private var handyTheme

    @Binding var selection: AppSection

    var body: some View {
        VStack(spacing: 0) {
            HandyLogoView()
                .padding(.vertical, 16)

            VStack(spacing: 4) {
                ForEach(AppSection.visibleSections(settings: appModel.settings)) { section in
                    sidebarButton(for: section)
                }
            }
            .padding(.top, 8)
            .overlay(alignment: .top) {
                Rectangle()
                    .fill(HandyDesign.midGray.opacity(0.2))
                    .frame(height: 1)
            }

            Spacer(minLength: 0)
        }
        .padding(.horizontal, 8)
        .frame(maxHeight: .infinity)
        .overlay(alignment: .trailing) {
            Rectangle()
                .fill(HandyDesign.midGray.opacity(0.2))
                .frame(width: 1)
        }
    }

    private func sidebarButton(for section: AppSection) -> some View {
        let isActive = selection == section
        let itemColor = HandyDesign.text.opacity(isActive ? 1 : 0.85)

        return Button {
            selection = section
        } label: {
            HStack(spacing: 8) {
                SidebarHugeIcon(kind: SidebarHugeIconKind(section: section), color: itemColor)
                    .frame(width: 24, height: 24)
                Text(section.title)
                    .font(HandyDesign.font(size: 14, weight: .medium))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 0)
            }
            .foregroundStyle(itemColor)
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(isActive ? handyTheme.logoPrimary(for: colorScheme).opacity(0.8) : .clear)
            .clipShape(RoundedRectangle(cornerRadius: HandyDesign.cornerRadius, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: HandyDesign.cornerRadius, style: .continuous))
        }
        .buttonStyle(.plain)
    }
}
