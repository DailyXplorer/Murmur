import SwiftUI

struct NativeOnboardingView: View {
    @EnvironmentObject private var appModel: AppModel

    var body: some View {
        switch appModel.onboardingStep {
        case .checking:
            ProgressView()
                .controlSize(.large)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(MurmurDesign.background)
        case let .permissions(returningUser):
            NativePermissionsOnboardingView(returningUser: returningUser)
                .environmentObject(appModel)
        case .model:
            NativeModelOnboardingView()
                .environmentObject(appModel)
        case .done:
            EmptyView()
        }
    }
}

private struct NativePermissionsOnboardingView: View {
    @EnvironmentObject private var appModel: AppModel

    let returningUser: Bool
    @State private var requestedMicrophone = false
    @State private var requestedAccessibility = false
    @State private var requestedSpeechRecognition = false

    var body: some View {
        VStack(spacing: 24) {
            OnboardingLogo()

            VStack(spacing: 8) {
                Text("Permissions Required")
                    .font(MurmurDesign.font(size: 20, weight: .semibold))
                    .foregroundStyle(MurmurDesign.text)

                Text("Murmur needs a couple of permissions to work properly.")
                    .font(MurmurDesign.font(size: 14))
                    .foregroundStyle(MurmurDesign.text.opacity(0.7))
                    .multilineTextAlignment(.center)
            }

            VStack(spacing: 16) {
                PermissionCard(
                    title: "Microphone Access",
                    description: "Required to hear your voice for transcription.",
                    icon: .mic,
                    isGranted: appModel.permissionSnapshot.microphone == .granted,
                    waiting: requestedMicrophone && appModel.permissionSnapshot.microphone == .notDetermined
                ) {
                    requestedMicrophone = true
                    appModel.requestMicrophone()
                }

                PermissionCard(
                    title: "Accessibility Access",
                    description: "Required to type transcribed text into your applications.",
                    icon: .keyboard,
                    isGranted: appModel.permissionSnapshot.accessibilityTrusted,
                    waiting: requestedAccessibility && !appModel.permissionSnapshot.accessibilityTrusted
                ) {
                    requestedAccessibility = true
                    appModel.requestAccessibility()
                }

                if appModel.settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID {
                    PermissionCard(
                        title: "Speech Recognition",
                        description: "Required when Apple Speech is the selected transcription model.",
                        icon: .languages,
                        isGranted: appModel.permissionSnapshot.speechRecognition == .granted,
                        waiting: requestedSpeechRecognition && appModel.permissionSnapshot.speechRecognition == .notDetermined
                    ) {
                        requestedSpeechRecognition = true
                        appModel.requestSpeechRecognition()
                    }
                }
            }
            .frame(maxWidth: 448)

            if returningUser {
                Text("Your transcription model is ready. Grant permissions to continue.")
                    .font(MurmurDesign.font(size: 13))
                    .foregroundStyle(MurmurDesign.text.opacity(0.55))
            }
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(MurmurDesign.background)
        .task {
            while !Task.isCancelled {
                await appModel.refreshPermissions()
                if case .permissions = appModel.onboardingStep {
                    try? await Task.sleep(for: .seconds(1))
                } else {
                    return
                }
            }
        }
    }
}

private struct PermissionCard: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme

    let title: String
    let description: String
    let icon: MurmurHugeIconKind
    let isGranted: Bool
    let waiting: Bool
    let action: () -> Void

    var body: some View {
        HStack(alignment: .center, spacing: 16) {
            MurmurHugeIcon(kind: icon, color: murmurTheme.logoPrimary(for: colorScheme), size: 24)
                .frame(width: 48, height: 48)
                .background(murmurTheme.logoPrimary(for: colorScheme).opacity(0.2))
                .clipShape(Circle())

            VStack(alignment: .leading, spacing: 6) {
                Text(title)
                    .font(MurmurDesign.font(size: 15, weight: .medium))
                    .foregroundStyle(MurmurDesign.text)
                Text(description)
                    .font(MurmurDesign.font(size: 13))
                    .foregroundStyle(MurmurDesign.text.opacity(0.6))

                if isGranted {
                    PermissionStatusLabel(text: "Granted", icon: .check, color: Color.green)
                } else if waiting {
                    PermissionStatusLabel(
                        text: "Waiting...",
                        icon: .loading,
                        color: MurmurDesign.text.opacity(0.55),
                        spinning: true
                    )
                } else {
                    Button("Grant Permission", action: action)
                        .buttonStyle(MurmurButtonStyle())
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(16)
        .background(Color.white.opacity(0.05))
        .clipShape(RoundedRectangle(cornerRadius: MurmurDesign.cornerRadius, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: MurmurDesign.cornerRadius, style: .continuous)
                .stroke(MurmurDesign.midGray.opacity(0.2), lineWidth: 1)
        }
    }
}

private struct PermissionStatusLabel: View {
    let text: String
    let icon: MurmurHugeIconKind
    let color: Color
    var spinning = false

    var body: some View {
        HStack(spacing: 6) {
            if spinning {
                SpinningMurmurHugeIcon(kind: icon, color: color, size: 16)
            } else {
                MurmurHugeIcon(kind: icon, color: color, size: 16)
            }

            Text(text)
                .font(MurmurDesign.font(size: 13, weight: .medium))
                .foregroundStyle(color)
        }
    }
}

private struct NativeModelOnboardingView: View {
    @EnvironmentObject private var appModel: AppModel

    private var sortedModels: [LocalTranscriptionModel] {
        LocalTranscriptionModel.catalog.sorted { first, second in
            if first.isRecommended != second.isRecommended {
                return first.isRecommended
            }
            return first.accuracyScore > second.accuracyScore
        }
    }

    var body: some View {
        VStack(spacing: 16) {
            VStack(spacing: 8) {
                OnboardingLogo()
                Text("To get started, choose a transcription model")
                    .font(MurmurDesign.font(size: 15, weight: .medium))
                    .foregroundStyle(MurmurDesign.text.opacity(0.7))
            }
            .padding(.top, 24)

            ScrollView {
                VStack(spacing: 16) {
                    ForEach(sortedModels) { model in
                        OnboardingModelCard(
                            title: model.name,
                            description: model.description,
                            sizeDescription: model.sizeDescription,
                            speedScore: model.speedScore,
                            accuracyScore: model.accuracyScore,
                            supportsTranslation: model.supportsTranslation,
                            isRecommended: model.isRecommended,
                            status: status(for: model),
                            downloadProgress: appModel.localModelDownloadState(for: model.id)?.fractionCompleted,
                            disabled: isAnyModelDownloading
                        ) {
                            appModel.downloadLocalTranscriptionModel(id: model.id, selectWhenReady: true)
                        }
                    }

                    OnboardingModelCard(
                        title: AppleSpeechModelPresentation.title,
                        description: AppleSpeechModelPresentation.description,
                        sizeDescription: AppleSpeechModelPresentation.sizeDescription,
                        speedScore: AppleSpeechModelPresentation.speedScore,
                        accuracyScore: AppleSpeechModelPresentation.accuracyScore,
                        supportsTranslation: false,
                        isRecommended: false,
                        status: appleSpeechStatus,
                        disabled: false
                    ) {
                        appModel.useAppleSpeechTranscription()
                    }
                }
                .frame(maxWidth: 600)
                .padding(.horizontal, 24)
                .padding(.top, 2)
                .padding(.bottom, 24)
                .frame(maxWidth: .infinity)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(MurmurDesign.background)
        .onAppear {
            appModel.refreshLocalModelStorageStates()
        }
    }

    private var isAnyModelDownloading: Bool {
        LocalTranscriptionModel.catalog.contains { appModel.isDownloadingLocalModel(id: $0.id) }
    }

    private func status(for model: LocalTranscriptionModel) -> String {
        if let downloadState = appModel.localModelDownloadState(for: model.id) {
            return downloadState.statusLabel
        }
        if appModel.localModelStorageState(for: model).isDownloaded {
            return appModel.settings.selectedModel == model.id ? "Active" : "Use"
        }
        return "Download"
    }

    private var appleSpeechStatus: String {
        guard appModel.settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID else {
            return "Use"
        }

        switch appModel.permissionSnapshot.speechRecognition {
        case .granted:
            return "Active"
        case .notDetermined:
            return "Needs Permission"
        case .denied, .restricted:
            return "Permission Denied"
        case .unknown:
            return "Checking"
        }
    }
}

private struct OnboardingModelCard: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme

    let title: String
    let description: String
    let sizeDescription: String
    let speedScore: Double
    let accuracyScore: Double
    let supportsTranslation: Bool
    let isRecommended: Bool
    let status: String
    var downloadProgress: Double? = nil
    let disabled: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .center) {
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 8) {
                            Text(title)
                                .font(MurmurDesign.font(size: 16, weight: .semibold))
                                .foregroundStyle(MurmurDesign.text)
                            if isRecommended {
                                Badge("Recommended", emphasized: true)
                            }
                            if status == "Active" {
                                Badge("Active", emphasized: true, icon: .check)
                            }
                        }

                        Text(description)
                            .font(MurmurDesign.font(size: 14))
                            .foregroundStyle(MurmurDesign.text.opacity(0.6))
                            .fixedSize(horizontal: false, vertical: true)
                    }

                    Spacer()

                    ScoreBars(accuracyScore: accuracyScore, speedScore: speedScore)
                }

                MurmurDivider()
                    .padding(.leading, -16)
                    .padding(.trailing, -16)

                HStack(spacing: 12) {
                    ModelCapabilityLabel("Multi-language", icon: .globe)
                    if supportsTranslation {
                        ModelCapabilityLabel("Translate to English", icon: .languages)
                    }
                    Spacer()
                    ModelCapabilityLabel(sizeDescription, icon: status == "Download" ? .download : .hardDrive)
                }
                .font(MurmurDesign.font(size: 12))
                .foregroundStyle(MurmurDesign.text.opacity(0.5))

                if isDownloading {
                    VStack(alignment: .leading, spacing: 4) {
                        if let downloadProgress {
                            ProgressView(value: downloadProgress, total: 1)
                                .progressViewStyle(.linear)
                                .tint(murmurTheme.logoPrimary(for: colorScheme))
                        } else {
                            ProgressView()
                                .progressViewStyle(.linear)
                                .tint(murmurTheme.logoPrimary(for: colorScheme))
                        }

                        Text(status)
                            .font(MurmurDesign.font(size: 12))
                            .foregroundStyle(MurmurDesign.text.opacity(0.5))
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(background)
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(border, lineWidth: 2)
            }
        }
        .buttonStyle(.plain)
        .disabled(disabled && !isDownloading)
    }

    private var isDownloading: Bool {
        status.hasPrefix("Downloading") || status == "Canceling"
    }

    private var background: Color {
        if status == "Active" || isRecommended {
            return murmurTheme.logoPrimary(for: colorScheme).opacity(status == "Active" ? 0.1 : 0.05)
        }
        return .clear
    }

    private var border: Color {
        if status == "Active" {
            return murmurTheme.logoPrimary(for: colorScheme).opacity(0.5)
        }
        if isRecommended {
            return murmurTheme.logoPrimary(for: colorScheme).opacity(0.25)
        }
        return MurmurDesign.midGray.opacity(0.2)
    }
}

private struct ModelCapabilityLabel: View {
    let text: String
    let icon: MurmurHugeIconKind

    init(_ text: String, icon: MurmurHugeIconKind) {
        self.text = text
        self.icon = icon
    }

    var body: some View {
        HStack(spacing: 4) {
            MurmurHugeIcon(kind: icon, color: MurmurDesign.text.opacity(0.5), size: 14)
            Text(text)
        }
    }
}

private struct ScoreBars: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme

    let accuracyScore: Double
    let speedScore: Double

    var body: some View {
        VStack(spacing: 6) {
            scoreRow("accuracy", value: accuracyScore)
            scoreRow("speed", value: speedScore)
        }
        .frame(width: 152)
    }

    private func scoreRow(_ label: String, value: Double) -> some View {
        HStack(spacing: 8) {
            Text(label)
                .font(MurmurDesign.font(size: 12))
                .foregroundStyle(MurmurDesign.text.opacity(0.6))
                .frame(width: 64, alignment: .trailing)
            GeometryReader { proxy in
                ZStack(alignment: .leading) {
                    Capsule()
                        .fill(MurmurDesign.midGray.opacity(0.2))
                    Capsule()
                        .fill(murmurTheme.logoPrimary(for: colorScheme))
                        .frame(width: proxy.size.width * max(0, min(1, value)))
                }
            }
            .frame(width: 64, height: 6)
        }
    }
}

private struct Badge: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme

    let text: String
    var emphasized = false
    var icon: MurmurHugeIconKind?

    init(_ text: String, emphasized: Bool = false, icon: MurmurHugeIconKind? = nil) {
        self.text = text
        self.emphasized = emphasized
        self.icon = icon
    }

    var body: some View {
        HStack(spacing: 4) {
            if let icon {
                MurmurHugeIcon(kind: icon, color: MurmurDesign.text, size: 12)
            }
            Text(text)
        }
        .font(MurmurDesign.font(size: 11, weight: .medium))
        .foregroundStyle(MurmurDesign.text)
        .padding(.horizontal, 7)
        .padding(.vertical, 3)
        .background(emphasized ? murmurTheme.logoPrimary(for: colorScheme).opacity(0.2) : MurmurDesign.midGray.opacity(0.1))
        .clipShape(RoundedRectangle(cornerRadius: 999, style: .continuous))
    }
}

private struct OnboardingLogo: View {
    var body: some View {
        MurmurLogoView(width: 240, height: 88)
    }
}
