import SwiftUI

struct MurmurFooterView: View {
    @EnvironmentObject private var appModel: AppModel

    var body: some View {
        HStack {
            MurmurPill(text: appModel.selectedTranscriptionModelDisplayName, emphasized: true)

            Spacer()

            HStack(spacing: 6) {
                Text("v\(appModel.appVersion)")
                    .foregroundStyle(MurmurDesign.text.opacity(0.6))
            }
            .font(MurmurDesign.font(size: 12))
        }
        .padding(.horizontal, 16)
        .padding(.top, 12)
        .padding(.bottom, 12)
        .overlay(alignment: .top) {
            Rectangle()
                .fill(MurmurDesign.midGray.opacity(0.2))
                .frame(height: 1)
        }
        .background(MurmurDesign.background)
    }
}
