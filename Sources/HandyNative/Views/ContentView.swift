import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var appModel: AppModel

    var body: some View {
        Group {
            if appModel.onboardingStep == .done {
                mainSettingsShell
            } else {
                NativeOnboardingView()
                    .environmentObject(appModel)
            }
        }
        .background(HandyDesign.background)
        .foregroundStyle(HandyDesign.text)
        .font(HandyDesign.font(size: 15))
    }

    private var mainSettingsShell: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                SidebarView(selection: $appModel.selectedSection)
                    .frame(width: HandyDesign.sidebarWidth)

                DetailView(section: appModel.selectedSection)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            HandyFooterView()
        }
    }
}
