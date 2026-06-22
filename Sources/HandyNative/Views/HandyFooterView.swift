import SwiftUI

struct HandyFooterView: View {
    @EnvironmentObject private var appModel: AppModel

    var body: some View {
        HStack {
            HandyPill(text: appModel.selectedTranscriptionModelDisplayName, emphasized: true)

            Spacer()

            HStack(spacing: 6) {
                Text("v\(appModel.appVersion)")
                    .foregroundStyle(HandyDesign.text.opacity(0.6))
            }
            .font(HandyDesign.font(size: 12))
        }
        .padding(.horizontal, 16)
        .padding(.top, 12)
        .padding(.bottom, 12)
        .overlay(alignment: .top) {
            Rectangle()
                .fill(HandyDesign.midGray.opacity(0.2))
                .frame(height: 1)
        }
        .background(HandyDesign.background)
    }
}
