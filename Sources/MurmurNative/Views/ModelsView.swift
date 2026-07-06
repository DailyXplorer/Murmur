import AppKit
import SwiftUI

struct ModelsView: View {
    @Environment(\.colorScheme) private var colorScheme
    @EnvironmentObject private var appModel: AppModel
    @Environment(\.murmurTheme) private var murmurTheme

    @State private var draftAPIKey = ""
    @State private var draftModelID = ""
    @State private var draftDisplayName = ""
    @State private var pendingModelDeletion: PendingModelDeletion?
    @State private var languageFilter = ModelsLanguageFilter.allLanguagesID
    @State private var languageSearch = ""
    @State private var languageFilterPopoverPresented = false
    @FocusState private var focusedField: FocusedField?

    private enum FocusedField {
        case transcriptionAPIKey
    }

    private struct PendingModelDeletion: Identifiable {
        var model: LocalTranscriptionModel
        var confirmation: LocalModelDeletionConfirmation

        var id: String {
            model.id
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 8) {
                Text("Models")
                    .font(MurmurDesign.font(size: 20, weight: .semibold))
                    .foregroundStyle(MurmurDesign.text)
                Text("Manage local and API transcription models.")
                    .font(MurmurDesign.font(size: 14))
                    .foregroundStyle(MurmurDesign.text.opacity(0.6))
            }

            apiTranscriptionGroup

            let sections = modelListSections
            if sections.hasAnyVisibleModel {
                if sections.yourModels.isEmpty == false {
                    VStack(alignment: .leading, spacing: 8) {
                        modelsSectionHeader("YOUR MODELS") {
                            languageFilterButton
                        }
                        MurmurSettingsGroup {
                            modelList(sections.yourModels)
                        }
                    }
                }

                if sections.availableModels.isEmpty == false {
                    VStack(alignment: .leading, spacing: 8) {
                        modelsSectionHeader("AVAILABLE MODELS") {
                            if sections.yourModels.isEmpty {
                                languageFilterButton
                            }
                        }
                        MurmurSettingsGroup {
                            modelList(sections.availableModels)
                        }
                    }
                }
            } else {
                Text("No models match")
                    .font(MurmurDesign.font(size: 14))
                    .foregroundStyle(MurmurDesign.text.opacity(0.5))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 32)
            }
        }
        .frame(maxWidth: .infinity)
        .background(InitialFocusSink())
        .onAppear {
            resetAPIModelDraft()
            appModel.refreshLocalModelStorageStates()
        }
        .onChange(of: appModel.settings.transcriptionAPIProviderID) {
            draftAPIKey = ""
            resetAPIModelDraft()
        }
        .onChange(of: appModel.settings.transcriptionAPIModels) {
            if !canAddDraftAPIModel {
                resetAPIModelDraft()
            }
        }
        .onChange(of: languageFilterPopoverPresented) { _, isPresented in
            if !isPresented {
                languageSearch = ""
            }
        }
        .onChange(of: focusedField) { _, focusedField in
            if focusedField != .transcriptionAPIKey {
                saveDraftTranscriptionAPIKeyIfNeeded()
            }
        }
        .alert(
            pendingModelDeletion?.confirmation.title ?? "Delete Model",
            isPresented: pendingModelDeletionBinding,
            presenting: pendingModelDeletion
        ) { pendingDeletion in
            Button(pendingDeletion.confirmation.cancelButtonTitle, role: .cancel) {
                pendingModelDeletion = nil
            }
            Button(pendingDeletion.confirmation.destructiveButtonTitle, role: .destructive) {
                appModel.deleteLocalTranscriptionModel(id: pendingDeletion.model.id)
                pendingModelDeletion = nil
            }
        } message: { pendingDeletion in
            Text(pendingDeletion.confirmation.message)
        }
    }

    private var apiTranscriptionGroup: some View {
        MurmurSettingsGroup("API TRANSCRIPTION") {
            MurmurSettingRow("Provider", description: "Select an OpenAI-compatible transcription provider.") {
                Menu(selectedProvider.label) {
                    ForEach(appModel.settings.transcriptionAPIProviders) { provider in
                        Button(provider.label) {
                            appModel.selectTranscriptionAPIProvider(id: provider.id)
                        }
                    }
                }
                .buttonStyle(MurmurButtonStyle(variant: .secondary))
            }

            if selectedProvider.allowBaseURLEdit {
                MurmurDivider()
                MurmurSettingRow("Base URL", description: "API base URL for the selected transcription provider.") {
                    TextField("https://api.openai.com/v1", text: baseURLBinding)
                        .textFieldStyle(MurmurTextFieldStyle())
                        .frame(width: 300)
                }
            }

            MurmurDivider()
            MurmurSettingRow("API Key", description: apiKeyDescription) {
                SecureField(apiKeyPlaceholder, text: $draftAPIKey)
                    .textFieldStyle(MurmurTextFieldStyle())
                    .frame(width: 320)
                    .focused($focusedField, equals: .transcriptionAPIKey)
                    .onSubmit(saveDraftTranscriptionAPIKeyIfNeeded)
            }

            MurmurDivider()
            MurmurSettingRow("Model", description: modelDescription) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 8) {
                        TextField(modelPlaceholder, text: $draftModelID)
                            .textFieldStyle(MurmurTextFieldStyle())
                            .frame(width: 210)

                        Button("Add model") {
                            addDraftAPIModel()
                        }
                        .buttonStyle(MurmurButtonStyle())
                        .disabled(!canAddDraftAPIModel)
                        .frame(width: 102)
                        .help(modelAlreadyExists ? "Model already exists for this provider." : "")
                    }

                    TextField(displayNamePlaceholder, text: $draftDisplayName)
                        .textFieldStyle(MurmurTextFieldStyle())
                        .frame(width: 320)
                }
                .fixedSize(horizontal: true, vertical: false)
            }
        }
    }

    private var selectedProvider: TranscriptionAPIProvider {
        appModel.settings.selectedTranscriptionAPIProvider ?? TranscriptionAPIProvider.defaults[0]
    }

    private var modelListSections: ModelsListSections {
        ModelsListSections.make(
            settings: appModel.settings,
            localStorageStates: appModel.localModelStorageStates,
            localDownloadStates: appModel.localModelDownloadStates,
            languageFilter: languageFilter
        )
    }

    private func localModelCard(_ model: LocalTranscriptionModel) -> some View {
        let state = appModel.localModelStorageState(for: model)
        let isActive = appModel.settings.selectedModel == model.id
        let downloadState = appModel.localModelDownloadState(for: model.id)
        let isDownloading = downloadState != nil
        let primaryAction: () -> Void = {
            if state.isDownloaded {
                appModel.selectTranscriptionModel(id: model.id)
            } else {
                appModel.downloadLocalTranscriptionModel(id: model.id)
            }
        }

        return modelCard(
            title: model.name,
            status: localModelStatus(model),
            emphasized: isActive && state.isDownloaded,
            primaryIcon: localModelPrimaryIcon(model),
            primaryTitle: localModelPrimaryTitle(model),
            primaryDisabled: (isActive && state.isDownloaded) || isDownloading,
            secondaryIcon: isDownloading ? .cancelCircle : state.isDownloaded ? .delete : nil,
            secondaryDisabled: downloadState?.isCancelling == true || (!isDownloading && !state.isDownloaded),
            secondaryHelp: isDownloading ? "Cancel download" : state.isDownloaded ? "Delete model" : nil,
            secondaryAction: {
                if isDownloading {
                    appModel.cancelLocalTranscriptionModelDownload(id: model.id)
                } else {
                    requestLocalModelDeletion(model)
                }
            },
            downloadState: downloadState,
            details: AnyView(localModelMetrics(model)),
            action: primaryAction
        )
    }

    private func appleSpeechCard() -> some View {
        modelCard(
            title: AppleSpeechModelPresentation.title,
            status: appleSpeechStatus,
            emphasized: appModel.settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID &&
                appModel.permissionSnapshot.speechRecognition == .granted,
            primaryTitle: appleSpeechButtonTitle,
            details: AnyView(appleSpeechMetrics)
        ) {
            appModel.useAppleSpeechTranscription()
        }
    }

    private var appleSpeechStatus: String {
        guard appModel.settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID else {
            return "Available"
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

    private var appleSpeechButtonTitle: String {
        guard appModel.settings.selectedModel == TranscriptionAPIProvider.appleSpeechModelID else {
            return "Use"
        }

        return appModel.permissionSnapshot.speechRecognition == .granted ? "Active" : "Grant"
    }

    private func apiModelCard(_ model: TranscriptionAPIModel) -> some View {
        modelCard(
            title: model.displayName,
            status: appModel.settings.selectedModel == model.id ? "Active" : "Use",
            emphasized: appModel.settings.selectedModel == model.id,
            primaryTitle: appModel.settings.selectedModel == model.id ? "Active" : "Use",
            details: AnyView(apiModelIdentifier(model))
        ) {
            appModel.selectTranscriptionModel(id: model.id)
        }
    }

    private func modelList(_ items: [ModelsListItem]) -> some View {
        ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
            modelListItemCard(item)
            if index < items.count - 1 {
                MurmurDivider()
            }
        }
    }

    @ViewBuilder
    private func modelListItemCard(_ item: ModelsListItem) -> some View {
        switch item {
        case let .local(id):
            if let model = LocalTranscriptionModel.model(for: id) {
                localModelCard(model)
            }
        case .appleSpeech:
            appleSpeechCard()
        case let .api(id):
            if let model = appModel.settings.transcriptionAPIModels.first(where: { $0.id == id }) {
                apiModelCard(model)
            }
        }
    }

    private func modelsSectionHeader<Accessory: View>(
        _ title: String,
        @ViewBuilder accessory: () -> Accessory
    ) -> some View {
        HStack(spacing: 12) {
            Text(title)
                .font(MurmurDesign.font(size: 12, weight: .medium))
                .foregroundStyle(MurmurDesign.midGray)
                .textCase(.uppercase)
                .tracking(0.6)

            Spacer()

            accessory()
        }
        .padding(.horizontal, 16)
    }

    private var languageFilterButton: some View {
        Button {
            languageFilterPopoverPresented.toggle()
        } label: {
            HStack(spacing: 6) {
                MurmurHugeIcon(kind: .globe, color: languageFilterColor, size: 14)
                Text(ModelsLanguageFilter.label(for: languageFilter))
                    .font(MurmurDesign.font(size: 14, weight: .medium))
                    .foregroundStyle(languageFilterColor)
                    .lineLimit(1)
                    .frame(maxWidth: 120, alignment: .leading)
                MurmurHugeIcon(kind: .chevronDown, color: languageFilterColor, size: 14)
                    .rotationEffect(.degrees(languageFilterPopoverPresented ? 180 : 0))
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(languageFilterBackground)
            .clipShape(RoundedRectangle(cornerRadius: MurmurDesign.cornerRadius, style: .continuous))
        }
        .buttonStyle(.plain)
        .popover(isPresented: $languageFilterPopoverPresented, arrowEdge: .bottom) {
            languageFilterPopover
        }
    }

    private var languageFilterColor: Color {
        languageFilter == ModelsLanguageFilter.allLanguagesID
            ? MurmurDesign.text.opacity(0.6)
            : murmurTheme.logoPrimary(for: colorScheme)
    }

    private var languageFilterBackground: Color {
        languageFilter == ModelsLanguageFilter.allLanguagesID
            ? MurmurDesign.midGray.opacity(0.1)
            : murmurTheme.logoPrimary(for: colorScheme).opacity(0.2)
    }

    private var languageFilterPopover: some View {
        VStack(alignment: .leading, spacing: 0) {
            TextField("Search...", text: $languageSearch)
                .textFieldStyle(MurmurTextFieldStyle())
                .padding(8)

            MurmurDivider()
                .padding(.leading, -16)

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    languageChoiceButton(title: ModelsLanguageFilter.label(for: ModelsLanguageFilter.allLanguagesID), code: ModelsLanguageFilter.allLanguagesID)
                    ForEach(ModelsLanguageFilter.filteredLanguages(searchText: languageSearch)) { language in
                        languageChoiceButton(title: language.name, code: language.code)
                    }

                    if ModelsLanguageFilter.filteredLanguages(searchText: languageSearch).isEmpty {
                        Text("No results")
                            .font(MurmurDesign.font(size: 14))
                            .foregroundStyle(MurmurDesign.text.opacity(0.5))
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 12)
                    }
                }
            }
            .frame(maxHeight: 210)
        }
        .frame(width: 224)
        .background(MurmurDesign.background)
    }

    private func languageChoiceButton(title: String, code: String) -> some View {
        Button {
            languageFilter = code
            languageSearch = ""
            languageFilterPopoverPresented = false
        } label: {
            HStack(spacing: 8) {
                Text(title)
                    .font(MurmurDesign.font(size: 14, weight: languageFilter == code ? .semibold : .regular))
                    .foregroundStyle(languageFilter == code ? murmurTheme.logoPrimary(for: colorScheme) : MurmurDesign.text)
                    .lineLimit(1)
                Spacer()
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .background(languageFilter == code ? murmurTheme.logoPrimary(for: colorScheme).opacity(0.2) : Color.clear)
        }
        .buttonStyle(.plain)
    }

    private var pendingModelDeletionBinding: Binding<Bool> {
        Binding(
            get: { pendingModelDeletion != nil },
            set: { isPresented in
                if !isPresented {
                    pendingModelDeletion = nil
                }
            }
        )
    }

    private func requestLocalModelDeletion(_ model: LocalTranscriptionModel) {
        pendingModelDeletion = PendingModelDeletion(
            model: model,
            confirmation: LocalModelDeletionConfirmation.request(
                for: model,
                isActive: appModel.settings.selectedModel == model.id
            )
        )
    }

    private func localModelStatus(_ model: LocalTranscriptionModel) -> String {
        LocalModelRuntimePresentation.status(
            downloadState: appModel.localModelDownloadState(for: model.id),
            runtimeState: appModel.localModelRuntimeState(for: model),
            isActive: appModel.settings.selectedModel == model.id,
            storageState: appModel.localModelStorageState(for: model)
        )
    }

    private func localModelPrimaryIcon(_ model: LocalTranscriptionModel) -> MurmurHugeIconKind {
        let state = appModel.localModelStorageState(for: model)
        if appModel.isDownloadingLocalModel(id: model.id) {
            return .loading
        }
        if appModel.settings.selectedModel == model.id && state.isDownloaded {
            return .checkCircle
        }
        if state.isDownloaded {
            return .checkCircle
        }
        return .download
    }

    private func localModelPrimaryTitle(_ model: LocalTranscriptionModel) -> String {
        let state = appModel.localModelStorageState(for: model)
        if appModel.isDownloadingLocalModel(id: model.id) {
            return "Downloading"
        }
        if appModel.settings.selectedModel == model.id && state.isDownloaded {
            return "Active"
        }
        if state.isDownloaded {
            return "Use"
        }
        return "Download"
    }

    private func localModelMetrics(_ model: LocalTranscriptionModel) -> some View {
        ModelScoreBars(
            accuracyScore: model.accuracyScore,
            speedScore: model.speedScore
        )
    }

    private var appleSpeechMetrics: some View {
        ModelScoreBars(
            accuracyScore: AppleSpeechModelPresentation.accuracyScore,
            speedScore: AppleSpeechModelPresentation.speedScore
        )
    }

    private func apiModelIdentifier(_ model: TranscriptionAPIModel) -> some View {
        Text(model.modelID)
            .font(MurmurDesign.font(size: 12, weight: .medium))
            .foregroundStyle(MurmurDesign.text.opacity(0.45))
            .lineLimit(1)
            .truncationMode(.middle)
            .monospacedDigit()
    }

    private var modelDescription: String {
        "Type the exact model ID exposed by this provider."
    }

    private var apiKeyPlaceholder: String {
        appModel.transcriptionAPIKeyConfigured ? "Configured" : "sk-..."
    }

    private var apiKeyDescription: String {
        selectedProvider.requiresAPIKey
            ? "API key used when this provider transcribes audio."
            : "Optional API key for local or trusted custom endpoints."
    }

    private var modelPlaceholder: String {
        "Model ID"
    }

    private var displayNamePlaceholder: String {
        "Display name (optional)"
    }

    private var modelAlreadyExists: Bool {
        appModel.settings.transcriptionAPIModelExistsForSelectedProvider(modelID: draftModelID)
    }

    private var canAddDraftAPIModel: Bool {
        !draftModelID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !modelAlreadyExists
    }

    private var baseURLBinding: Binding<String> {
        Binding(
            get: { selectedProvider.baseURL },
            set: { appModel.updateSelectedTranscriptionAPIBaseURL($0) }
        )
    }

    private func modelCard(
        title: String,
        status: String,
        emphasized: Bool,
        primaryIcon: MurmurHugeIconKind? = nil,
        primaryTitle: String? = nil,
        primaryDisabled: Bool? = nil,
        secondaryIcon: MurmurHugeIconKind? = nil,
        secondaryDisabled: Bool = true,
        secondaryHelp: String? = nil,
        secondaryAction: (() -> Void)? = nil,
        downloadState: LocalModelDownloadState? = nil,
        details: AnyView? = nil,
        action: @escaping () -> Void
    ) -> some View {
        let unavailable = status == "Unavailable"
        let isDownloading = downloadState != nil
        let isPrimaryDisabled = primaryDisabled ?? (emphasized || unavailable)
        let resolvedPrimaryIcon = primaryIcon ?? (unavailable ? .alertTriangle : emphasized ? .checkCircle : .checkCircle)
        let resolvedPrimaryTitle = primaryTitle ?? (emphasized ? "Active" : unavailable ? "Unavailable" : "Use")
        let primaryColor = emphasized ? murmurTheme.logoPrimary(for: colorScheme) : MurmurDesign.text.opacity(isPrimaryDisabled ? 0.35 : 0.5)
        return VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .center, spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        Text(title)
                            .font(MurmurDesign.font(size: 16, weight: .semibold))
                            .foregroundStyle(emphasized ? murmurTheme.logoPrimary(for: colorScheme) : MurmurDesign.text)
                            .lineLimit(1)
                        MurmurPill(text: status, emphasized: emphasized)
                    }
                }
                Spacer()
                if let secondaryIcon, let secondaryAction {
                    Button {
                        secondaryAction()
                    } label: {
                        ModelCardHugeIcon(
                            kind: secondaryIcon,
                            color: MurmurDesign.text.opacity(secondaryDisabled ? 0.3 : 0.45),
                            size: 16
                        )
                    }
                    .buttonStyle(.plain)
                    .disabled(secondaryDisabled)
                    .help(secondaryHelp ?? "")
                }
                Button {
                    action()
                } label: {
                    HStack(spacing: 6) {
                        ModelCardHugeIcon(
                            kind: resolvedPrimaryIcon,
                            color: primaryColor,
                            size: 18,
                            spinning: isDownloading
                        )
                        Text(resolvedPrimaryTitle)
                    }
                }
                .buttonStyle(MurmurButtonStyle(variant: emphasized ? .soft : .secondary))
                .disabled(isPrimaryDisabled)
            }

            if let details {
                details
            }

            if let downloadState {
                modelDownloadProgress(downloadState)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    private func modelDownloadProgress(_ downloadState: LocalModelDownloadState) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            if let fractionCompleted = downloadState.fractionCompleted {
                ProgressView(value: fractionCompleted, total: 1)
                    .progressViewStyle(.linear)
                    .tint(murmurTheme.logoPrimary(for: colorScheme))
            } else {
                ProgressView()
                    .progressViewStyle(.linear)
                    .tint(murmurTheme.logoPrimary(for: colorScheme))
            }

            Text(downloadState.statusLabel)
                .font(MurmurDesign.font(size: 12))
                .foregroundStyle(MurmurDesign.text.opacity(0.5))
        }
    }

    private func addDraftAPIModel() {
        guard canAddDraftAPIModel else {
            return
        }

        appModel.addSelectedTranscriptionAPIModel(modelID: draftModelID, displayName: draftDisplayName)
        resetAPIModelDraft()
    }

    private func resetAPIModelDraft() {
        draftModelID = ""
        draftDisplayName = ""
    }

    private func saveDraftTranscriptionAPIKeyIfNeeded() {
        let trimmed = draftAPIKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.isEmpty == false else {
            return
        }

        appModel.saveTranscriptionAPIKey(trimmed)
        draftAPIKey = ""
    }
}

private struct ModelScoreBars: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.murmurTheme) private var murmurTheme

    let accuracyScore: Double
    let speedScore: Double

    var body: some View {
        VStack(spacing: 6) {
            scoreRow("Accuracy", value: accuracyScore)
            scoreRow("Speed", value: speedScore)
        }
        .frame(width: 168)
    }

    private func scoreRow(_ label: String, value: Double) -> some View {
        HStack(spacing: 8) {
            Text(label)
                .font(MurmurDesign.font(size: 12, weight: .medium))
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
            .frame(width: 76, height: 6)
        }
    }
}

private struct ModelCardHugeIcon: View {
    let kind: MurmurHugeIconKind
    let color: Color
    var size: CGFloat
    var spinning = false

    var body: some View {
        if spinning {
            SpinningMurmurHugeIcon(kind: kind, color: color, size: size)
        } else {
            MurmurHugeIcon(kind: kind, color: color, size: size)
        }
    }
}

private struct InitialFocusSink: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = FocusSinkView()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            view.window?.makeFirstResponder(view)
        }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}
}

private final class FocusSinkView: NSView {
    override var acceptsFirstResponder: Bool {
        true
    }
}
