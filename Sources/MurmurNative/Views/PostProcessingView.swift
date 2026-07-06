import AppKit
import SwiftUI

struct PostProcessingView: View {
    @EnvironmentObject private var appModel: AppModel

    @State private var isCreating = false
    @State private var draftName = ""
    @State private var draftPrompt = ""
    @State private var draftAPIKey = ""
    @State private var draftAPIKeyHadUserInput = false
    @FocusState private var focusedField: FocusedField?

    private enum FocusedField {
        case apiKey
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 24) {
            hotkeyGroup
            apiGroup
            promptsGroup
        }
        .frame(maxWidth: .infinity)
        .onAppear(perform: syncDraftFromSelectedPrompt)
        .onChange(of: appModel.settings.postProcessSelectedPromptID) {
            syncDraftFromSelectedPrompt()
        }
        .onChange(of: appModel.settings.postProcessPrompts) {
            syncDraftFromSelectedPrompt()
        }
        .onChange(of: appModel.settings.postProcessProviderID) {
            draftAPIKey = ""
            draftAPIKeyHadUserInput = false
            focusedField = nil
        }
        .onChange(of: focusedField) { _, focusedField in
            if focusedField != .apiKey {
                saveDraftPostProcessAPIKeyIfNeeded()
            }
        }
        .background(PostProcessingInitialFocusSink())
    }

    private var hotkeyGroup: some View {
        MurmurSettingsGroup("HOTKEY") {
            MurmurSettingRow(
                "Transcribe with Post-Processing",
                description: "Converts your speech into text and applies AI post-processing."
            ) {
                HStack(spacing: 8) {
                    ShortcutBindingField(
                        placeholder: "option+shift+space",
                        currentBinding: appModel.settings.transcribeWithPostProcessShortcutBinding.currentBinding,
                        width: 190,
                        reservedBindings: reservedShortcutBindings(
                            excluding: ShortcutBinding.transcribeWithPostProcessID
                        )
                    ) {
                        appModel.updateShortcutBinding(
                            id: ShortcutBinding.transcribeWithPostProcessID,
                            currentBinding: $0
                        )
                    }
                }
            }
        }
    }

    private func reservedShortcutBindings(excluding id: String) -> [String] {
        appModel.settings.shortcutBindings.values
            .filter { $0.id != id }
            .map(\.currentBinding)
    }

    private var apiGroup: some View {
        MurmurSettingsGroup("API (OpenAI Compatible)") {
            MurmurSettingRow("Provider", description: "Select an OpenAI-compatible provider.") {
                Menu(selectedProvider.label) {
                    ForEach(appModel.settings.postProcessProviders) { provider in
                        Button(provider.label) {
                            appModel.selectPostProcessProvider(id: provider.id)
                        }
                    }
                }
                .buttonStyle(MurmurButtonStyle(variant: .secondary))
            }

            if selectedProvider.allowBaseURLEdit {
                MurmurDivider()
                MurmurSettingRow("Base URL", description: "API base URL for the selected provider.") {
                    TextField("https://api.openai.com/v1", text: baseURLBinding)
                        .textFieldStyle(MurmurTextFieldStyle())
                        .frame(width: 300)
                }
            }

            if isAppleProvider {
                if let detail = appModel.appleIntelligenceAvailability.detail {
                    MurmurDivider()
                    PostProcessContainedAlert(text: detail)
                }
            } else {
                MurmurDivider()
                MurmurSettingRow("API Key", description: "API key for the selected provider.") {
                    ZStack(alignment: .leading) {
                        SecureField(appModel.postProcessAPIKeyConfigured ? "" : "sk-...", text: $draftAPIKey)
                            .textFieldStyle(MurmurTextFieldStyle())
                            .focused($focusedField, equals: .apiKey)
                            .onChange(of: draftAPIKey) {
                                if focusedField == .apiKey {
                                    draftAPIKeyHadUserInput = true
                                }
                            }
                            .onSubmit(saveDraftPostProcessAPIKeyIfNeeded)

                        if appModel.postProcessAPIKeyConfigured,
                           draftAPIKey.isEmpty,
                           focusedField != .apiKey {
                            Text(String(repeating: "*", count: 18))
                                .font(MurmurDesign.font(size: 14, weight: .semibold))
                                .foregroundStyle(MurmurDesign.text.opacity(0.55))
                                .padding(.horizontal, 10)
                                .allowsHitTesting(false)
                        }
                    }
                    .frame(width: 320)
                }

                MurmurDivider()
                PostProcessStackedSetting("Model", description: modelDescription) {
                    HStack(spacing: 8) {
                        TextField(modelPlaceholder, text: modelBinding)
                            .textFieldStyle(MurmurTextFieldStyle())
                            .frame(minWidth: 380)
                        if !appModel.postProcessModelOptions.isEmpty {
                            Menu("Models") {
                                ForEach(appModel.postProcessModelOptions, id: \.self) { model in
                                    Button(model) {
                                        appModel.updateSelectedPostProcessModel(model)
                                    }
                                }
                            }
                            .buttonStyle(MurmurButtonStyle(variant: .secondary))
                        }
                        Button {
                            appModel.fetchPostProcessModels()
                        } label: {
                            if appModel.isFetchingPostProcessModels {
                                SpinningMurmurHugeIcon(kind: .refresh, color: MurmurDesign.text, size: 16)
                            } else {
                                MurmurHugeIcon(kind: .refresh, color: MurmurDesign.text, size: 16)
                            }
                        }
                        .buttonStyle(MurmurButtonStyle(variant: .secondary))
                        .disabled(appModel.isFetchingPostProcessModels)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }

    private var promptsGroup: some View {
        MurmurSettingsGroup("PROMPT") {
            MurmurSettingRow(
                "Selected Prompt",
                description: "Select a template for refining transcriptions or create a new one. Use ${output} inside the prompt text to reference the captured transcript."
            ) {
                HStack(spacing: 8) {
                    Menu(selectedPrompt?.name ?? promptPlaceholder) {
                        ForEach(appModel.settings.postProcessPrompts) { prompt in
                            Button(prompt.name) {
                                isCreating = false
                                appModel.selectPostProcessPrompt(id: prompt.id)
                            }
                        }
                    }
                    .buttonStyle(MurmurButtonStyle(variant: .secondary))
                    .disabled(appModel.settings.postProcessPrompts.isEmpty || isCreating)

                    Button("Create New Prompt") {
                        startCreating()
                    }
                    .buttonStyle(MurmurButtonStyle())
                    .disabled(isCreating)
                }
            }

            MurmurDivider()

            if isCreating {
                promptEditor(
                    primaryTitle: "Create Prompt",
                    secondaryTitle: "Cancel",
                    canSubmit: canSubmitDraft,
                    primaryAction: createPrompt,
                    secondaryAction: cancelCreating
                )
            } else if selectedPrompt != nil {
                promptEditor(
                    primaryTitle: "Update Prompt",
                    secondaryTitle: "Delete Prompt",
                    canSubmit: promptHasChanges,
                    primaryAction: updatePrompt,
                    secondaryAction: deletePrompt,
                    secondaryDisabled: appModel.settings.postProcessPrompts.count <= 1
                )
            } else {
                promptEmptyState
            }
        }
    }

    private var selectedProvider: PostProcessProvider {
        appModel.settings.selectedPostProcessProvider ?? PostProcessProvider.defaults[0]
    }

    private var selectedPrompt: PostProcessPrompt? {
        appModel.settings.selectedPostProcessPrompt
    }

    private var isAppleProvider: Bool {
        selectedProvider.id == PostProcessProvider.appleIntelligenceProviderID
    }

    private var modelDescription: String {
        if isAppleProvider {
            return "Provide an optional numeric token limit or keep the default on-device preset."
        }
        if selectedProvider.id == "custom" {
            return "Provide the model identifier expected by your custom endpoint."
        }
        return "Choose a model exposed by the selected provider."
    }

    private var modelPlaceholder: String {
        appModel.settings.selectedPostProcessModelDisplay == "Not configured" ? "Type a model name" : appModel.settings.selectedPostProcessModelDisplay
    }

    private var promptPlaceholder: String {
        appModel.settings.postProcessPrompts.isEmpty ? "No prompts available" : "Select a prompt"
    }

    private var canSubmitDraft: Bool {
        !draftName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
            !draftPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var promptHasChanges: Bool {
        guard let selectedPrompt, canSubmitDraft else {
            return false
        }

        return draftName.trimmingCharacters(in: .whitespacesAndNewlines) != selectedPrompt.name ||
            draftPrompt.trimmingCharacters(in: .whitespacesAndNewlines) != selectedPrompt.prompt.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var modelBinding: Binding<String> {
        Binding(
            get: { appModel.settings.postProcessModels[appModel.settings.postProcessProviderID] ?? "" },
            set: { appModel.updateSelectedPostProcessModel($0) }
        )
    }

    private var baseURLBinding: Binding<String> {
        Binding(
            get: { selectedProvider.baseURL },
            set: { appModel.updateSelectedPostProcessBaseURL($0) }
        )
    }

    private var promptEmptyState: some View {
        Text(appModel.settings.postProcessPrompts.isEmpty ? "Click 'Create New Prompt' above to create your first post-processing prompt." : "Select a prompt above to view and edit its details.")
            .font(MurmurDesign.font(size: 14))
            .foregroundStyle(MurmurDesign.text.opacity(0.6))
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .background(MurmurDesign.midGray.opacity(0.05))
            .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .stroke(MurmurDesign.midGray.opacity(0.2), lineWidth: 1)
            }
            .padding(16)
    }

    private func promptEditor(
        primaryTitle: String,
        secondaryTitle: String,
        canSubmit: Bool,
        primaryAction: @escaping () -> Void,
        secondaryAction: @escaping () -> Void,
        secondaryDisabled: Bool = false
    ) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 8) {
                Text("Prompt Label")
                    .font(MurmurDesign.font(size: 14, weight: .semibold))
                    .foregroundStyle(MurmurDesign.text)
                TextField("Enter prompt name", text: $draftName)
                    .textFieldStyle(MurmurTextFieldStyle())
            }

            VStack(alignment: .leading, spacing: 8) {
                Text("Prompt Instructions")
                    .font(MurmurDesign.font(size: 14, weight: .semibold))
                    .foregroundStyle(MurmurDesign.text)
                TextEditor(text: $draftPrompt)
                    .font(MurmurDesign.font(size: 14))
                    .foregroundStyle(MurmurDesign.text)
                    .scrollContentBackground(.hidden)
                    .padding(8)
                    .frame(minHeight: 168)
                    .background(MurmurDesign.midGray.opacity(0.1))
                    .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .stroke(MurmurDesign.midGray.opacity(0.8), lineWidth: 1)
                    }

                Text("Tip: Use ${output} to insert the transcribed text in your prompt.")
                    .font(MurmurDesign.font(size: 12))
                    .foregroundStyle(MurmurDesign.midGray.opacity(0.7))
            }

            HStack(spacing: 8) {
                Button(primaryTitle, action: primaryAction)
                    .buttonStyle(MurmurButtonStyle())
                    .disabled(!canSubmit)
                Button(secondaryTitle, action: secondaryAction)
                    .buttonStyle(MurmurButtonStyle(variant: .secondary))
                    .disabled(secondaryDisabled)
            }
            .padding(.top, 2)
        }
        .padding(16)
    }

    private func syncDraftFromSelectedPrompt() {
        guard !isCreating else {
            return
        }

        if let selectedPrompt {
            draftName = selectedPrompt.name
            draftPrompt = selectedPrompt.prompt
        } else {
            draftName = ""
            draftPrompt = ""
        }
    }

    private func startCreating() {
        isCreating = true
        draftName = ""
        draftPrompt = ""
    }

    private func cancelCreating() {
        isCreating = false
        syncDraftFromSelectedPrompt()
    }

    private func createPrompt() {
        guard canSubmitDraft else {
            return
        }
        appModel.addPostProcessPrompt(name: draftName, prompt: draftPrompt)
        isCreating = false
    }

    private func updatePrompt() {
        guard let selectedPrompt, promptHasChanges else {
            return
        }
        appModel.updatePostProcessPrompt(id: selectedPrompt.id, name: draftName, prompt: draftPrompt)
    }

    private func deletePrompt() {
        guard let selectedPrompt else {
            return
        }
        appModel.deletePostProcessPrompt(id: selectedPrompt.id)
    }

    private func saveDraftPostProcessAPIKeyIfNeeded() {
        let trimmed = draftAPIKey.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty == false {
            appModel.savePostProcessAPIKey(trimmed)
            draftAPIKey = ""
            draftAPIKeyHadUserInput = false
            return
        }

        if draftAPIKeyHadUserInput {
            appModel.clearPostProcessAPIKey()
            draftAPIKeyHadUserInput = false
        }
    }
}

private struct PostProcessStackedSetting<Content: View>: View {
    let title: String
    let description: String?
    @ViewBuilder let content: Content

    init(_ title: String, description: String? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.description = description
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Text(title)
                    .font(MurmurDesign.font(size: 14, weight: .medium))
                    .foregroundStyle(MurmurDesign.text)

                if let description {
                    MurmurTooltipIcon(text: description)
                }
            }

            content
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
    }
}

private struct PostProcessContainedAlert: View {
    let text: String

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            MurmurHugeIcon(kind: .alertCircle, color: Color.red.opacity(0.85), size: 20)
                .padding(.top, 1)

            Text(text)
                .font(MurmurDesign.font(size: 14))
                .foregroundStyle(Color.red.opacity(0.82))
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .background(Color.red.opacity(0.1))
    }
}

private struct PostProcessingInitialFocusSink: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = PostProcessingFocusSinkView()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            view.window?.makeFirstResponder(view)
        }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}
}

private final class PostProcessingFocusSinkView: NSView {
    override var acceptsFirstResponder: Bool {
        true
    }
}
