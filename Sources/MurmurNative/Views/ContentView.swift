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
        .background(MurmurDesign.background)
        .foregroundStyle(MurmurDesign.text)
        .font(MurmurDesign.font(size: 15))
    }

    private var mainSettingsShell: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                SidebarView(selection: $appModel.selectedSection)
                    .frame(width: MurmurDesign.sidebarWidth)

                DetailView(section: appModel.selectedSection)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            MurmurFooterView()
        }
    }
}
