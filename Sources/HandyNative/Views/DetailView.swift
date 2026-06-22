import SwiftUI

struct DetailView: View {
    let section: AppSection

    var body: some View {
        ScrollView {
            VStack(alignment: .center, spacing: 16) {
                PermissionBannerView()
                LastErrorBannerView()

                switch section {
                case .general:
                    GeneralView()
                case .models:
                    ModelsView()
                case .advanced:
                    AdvancedView()
                case .history:
                    HistoryView()
                case .postProcessing:
                    PostProcessingView()
                case .debug:
                    DebugView()
                }
            }
            .padding(16)
            .frame(maxWidth: HandyDesign.contentWidth)
            .frame(maxWidth: .infinity)
        }
        .background(HandyDesign.background)
    }
}

private struct PermissionBannerView: View {
    @EnvironmentObject private var appModel: AppModel

    var body: some View {
        if let banner = SettingsPermissionBannerModel.make(snapshot: appModel.permissionSnapshot) {
            HStack(alignment: .center, spacing: 12) {
                Text(banner.message)
                    .font(HandyDesign.font(size: 14, weight: .medium))
                    .foregroundStyle(HandyDesign.text)
                    .frame(maxWidth: .infinity, alignment: .leading)

                Button(banner.buttonTitle, action: appModel.requestAccessibility)
                    .buttonStyle(HandyButtonStyle(variant: .secondary))
                    .frame(minHeight: 40)
            }
            .padding(16)
            .background(HandyDesign.background)
            .clipShape(RoundedRectangle(cornerRadius: HandyDesign.cornerRadius, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: HandyDesign.cornerRadius, style: .continuous)
                    .stroke(HandyDesign.midGray, lineWidth: 1)
            }
        }
    }
}

private struct LastErrorBannerView: View {
    @EnvironmentObject private var appModel: AppModel

    var body: some View {
        if let banner = LastErrorBannerModel.make(message: appModel.lastErrorMessage) {
            HStack(alignment: .top, spacing: 12) {
                HandyHugeIcon(kind: .alertCircle, color: Color.red.opacity(0.85), size: 20)
                    .padding(.top, 1)

                Text(banner.message)
                    .font(HandyDesign.font(size: 14, weight: .medium))
                    .foregroundStyle(Color.red.opacity(0.82))
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)

                Button(action: appModel.clearLastErrorMessage) {
                    HandyHugeIcon(kind: .cancelCircle, color: Color.red.opacity(0.75), size: 18)
                }
                .buttonStyle(.plain)
                .help("Dismiss")
                .accessibilityLabel("Dismiss error")
            }
            .padding(16)
            .background(Color.red.opacity(0.1))
            .clipShape(RoundedRectangle(cornerRadius: HandyDesign.cornerRadius, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: HandyDesign.cornerRadius, style: .continuous)
                    .stroke(Color.red.opacity(0.18), lineWidth: 1)
            }
        }
    }
}

struct LastErrorBannerModel: Equatable {
    let message: String

    static func make(message: String?) -> LastErrorBannerModel? {
        guard let trimmed = message?.trimmingCharacters(in: .whitespacesAndNewlines),
              !trimmed.isEmpty else {
            return nil
        }

        return LastErrorBannerModel(message: trimmed)
    }
}

struct SettingsPermissionBannerModel: Equatable {
    let message: String
    let buttonTitle: String

    static func make(snapshot: PermissionSnapshot) -> SettingsPermissionBannerModel? {
        guard snapshot.accessibilityTrusted == false else {
            return nil
        }

        return SettingsPermissionBannerModel(
            message: "Handy needs accessibility permissions to type transcribed text.",
            buttonTitle: "Open Settings"
        )
    }
}
