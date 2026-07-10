import AppKit
import Carbon.HIToolbox
import Combine
import Foundation

@MainActor
final class AppModel: ObservableObject {
    @Published var selectedSection: AppSection = .general
    @Published var settings: AppSettings
    @Published var permissionSnapshot: PermissionSnapshot = .unknown
    @Published private(set) var recordingState: RecordingState = .idle
    @Published private(set) var audioLevel: Float = 0
    @Published private(set) var lastRecordingURL: URL?
    @Published private(set) var lastOutputText: String?
    @Published private(set) var lastErrorMessage: String?
    @Published private(set) var globalShortcutStatus = "Not installed"
    @Published private(set) var historyEntries: [HistoryEntry] = []
    @Published private(set) var historyHasMore = false
    @Published private(set) var copiedHistoryEntryID: Int64?
    @Published private(set) var inputDevices: [AudioDevice] = [.defaultDevice(direction: .input)]
    @Published private(set) var outputDevices: [AudioDevice] = [.defaultDevice(direction: .output)]
    @Published private(set) var isLaptop = AudioDeviceService.isLaptop()
    @Published private(set) var postProcessAPIKeyConfigured = false
    @Published private(set) var transcriptionAPIKeyConfigured = false
    @Published private(set) var postProcessModelOptions: [String] = []
    @Published private(set) var isFetchingPostProcessModels = false
    @Published private(set) var retryingHistoryEntryIDs = Set<Int64>()
    @Published private(set) var audioPlaybackState: AudioPlaybackState = .idle
    @Published private(set) var launchAtLoginStatus: LaunchAtLoginStatus = .disabled
    @Published private(set) var appleIntelligenceAvailability: AppleIntelligenceAvailability = .unchecked
    @Published private(set) var localModelStorageStates: [String: LocalModelStorageState] = [:]
    @Published private(set) var localModelRuntimeStates: [String: LocalModelRuntimeState] = [:]
    @Published private(set) var localModelDownloadStates: [String: LocalModelDownloadState] = [:]
    @Published private(set) var onboardingStep: NativeOnboardingStep = .checking
    @Published private(set) var audioInputVoiceProcessingStatus: AudioInputVoiceProcessingStatus = .notConfigured

    let paths: AppPaths

    private let settingsStore: SettingsStore
    private let postProcessCredentialStore: any PostProcessCredentialStoring
    private let historyStore: HistoryStore?
    private let logStore: NativeLogStore
    private let permissionService = PermissionService()
    private let audioCaptureService: any AudioCaptureServicing
    private let audioFeedbackService: any AudioFeedbackPlaying
    private let audioPlaybackService = AudioPlaybackService()
    private let launchAtLoginService: any LaunchAtLoginServicing
    private let pasteService: any PasteServicing
    private let recordingWorkflow: RecordingWorkflow
    // The recognition timeout is the termination backstop for Apple Speech: its
    // completion handler has no provably terminal nil-result/nil-error signal, so
    // without a timeout a stalled recognition would hang the pipeline forever.
    private let appleSpeechTranscriptionService = AppleSpeechTranscriptionService(recognitionTimeout: 120)
    private let whisperKitTranscriptionService = WhisperKitTranscriptionService()
    private let overlayPanelController = RecordingOverlayPanelController()
    private let globalShortcutService = GlobalShortcutService()
    private let remoteControlService = NativeRemoteControlService()
    private let launchArguments: NativeLaunchArguments
    private let persistedDebugModeAtLaunch: Bool
    private let persistedShowMenuBarIconAtLaunch: Bool
    private var coordinator = RecordingCoordinator()
    private var activeRecordingShortcutID: String?
    private var activeRecordingPostProcessRequested = false
    private var activeRecordingOperationID: UUID?
    private var activeRecordingHistoryEntryID: Int64?
    private var activeRecordingTask: Task<Void, Never>?
    private var prewarmingLocalModelIDs = Set<String>()
    private var localModelDownloadTasks: [String: Task<Void, Never>] = [:]
    private var localModelRuntimeRefreshTask: Task<Void, Never>?
    private var debugModeShortcutMonitor: Any?
    private var shortcutHealthTask: Task<Void, Never>?
    nonisolated(unsafe) private var wakeObserver: NSObjectProtocol?
    private let historyPageSize = 30

    init(
        launchArguments: NativeLaunchArguments = .none,
        launchAtLoginService: any LaunchAtLoginServicing = LaunchAtLoginService(),
        dependencies: AppModelDependencies = .live()
    ) {
        self.launchArguments = launchArguments
        self.launchAtLoginService = launchAtLoginService
        audioCaptureService = dependencies.audioCaptureService
        audioFeedbackService = dependencies.audioFeedbackService
        pasteService = dependencies.pasteService
        recordingWorkflow = dependencies.recordingWorkflow

        let resolvedPaths = (try? AppPaths.resolve()) ?? AppPaths(
            appDataDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("MurmurNative", isDirectory: true),
            recordingsDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("MurmurNative/recordings", isDirectory: true),
            modelsDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("MurmurNative/models", isDirectory: true),
            logsDirectory: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("MurmurNative/logs", isDirectory: true)
        )

        paths = resolvedPaths
        logStore = NativeLogStore(logsDirectory: resolvedPaths.logsDirectory)
        let credentialStore = LocalPostProcessCredentialStore(paths: resolvedPaths)
        postProcessCredentialStore = credentialStore
        settingsStore = SettingsStore(paths: resolvedPaths, credentialStore: credentialStore)
        let settingsLoadResult = settingsStore.loadResult()
        let persistedSettings = settingsLoadResult.settings
        persistedDebugModeAtLaunch = persistedSettings.debugMode
        persistedShowMenuBarIconAtLaunch = persistedSettings.showMenuBarIcon
        let runtimeSettings = Self.applyingRuntimeOverrides(launchArguments, to: persistedSettings)
        settings = runtimeSettings
        if let initialSection = launchArguments.initialSection,
           AppSection.visibleSections(settings: runtimeSettings).contains(initialSection) {
            selectedSection = initialSection
        }

        let resolvedHistoryStore: HistoryStore?
        let historyInitializationError: Error?
        do {
            resolvedHistoryStore = try HistoryStore(paths: resolvedPaths)
            historyInitializationError = nil
        } catch {
            resolvedHistoryStore = nil
            historyInitializationError = error
        }
        historyStore = resolvedHistoryStore
        log(.info, "Native app initialized.")

        overlayPanelController.onCancel = { [weak self] in
            self?.cancelRecording()
        }
        audioPlaybackService.onStateChange = { [weak self] state in
            self?.audioPlaybackState = state
        }

        if let historyInitializationError {
            lastErrorMessage = historyInitializationError.localizedDescription
            log(.error, "History store failed to initialize: \(historyInitializationError.localizedDescription)")
        } else {
            reloadHistory()
        }
        if let settingsWarning = settingsLoadResult.warningMessage {
            lastErrorMessage = settingsWarning
            log(.warn, settingsWarning)
        }
        refreshLocalModelStorageStates()
        refreshLocalModelRuntimeStates()
        refreshAudioDevices()
        refreshPostProcessCredentialStatus()
        refreshTranscriptionCredentialStatus()
        refreshOnboardingState()
        synchronizeLaunchAtLoginWithStoredSetting()
        remoteControlService.start { [weak self] command in
            Task { @MainActor in
                self?.handleRemoteControlCommand(command)
            }
        }
        installDebugModeShortcutMonitor()
        if let smokeOverlayState = launchArguments.smokeOverlayState {
            Task { @MainActor in
                try? await Task.sleep(for: .milliseconds(300))
                showOverlayForVisualSmoke(smokeOverlayState)
                if launchArguments.smokeOverlayOutputPath != nil || launchArguments.smokeOverlayImageOutputPath != nil {
                    try? await Task.sleep(for: .milliseconds(450))
                    let diagnostics = overlayPanelController.diagnostics(expectedState: smokeOverlayState)
                    do {
                        if let imageOutputPath = launchArguments.smokeOverlayImageOutputPath {
                            try Self.writeOverlaySmokeImage(
                                overlayPanelController.visualSnapshotPNGData(),
                                to: imageOutputPath
                            )
                        }
                        if let outputPath = launchArguments.smokeOverlayOutputPath {
                            try Self.writeOverlaySmokeDiagnostics(diagnostics, to: outputPath)
                        }
                        exit(diagnostics.success ? 0 : 1)
                    } catch {
                        FileHandle.standardError.writeLine(error.localizedDescription)
                        exit(1)
                    }
                }
            }
        }
        Task { @MainActor [weak self] in
            await self?.refreshPermissionsAtLaunch()
        }
        startShortcutHealthWatchdog()
    }

    deinit {
        shortcutHealthTask?.cancel()
        if let wakeObserver {
            NSWorkspace.shared.notificationCenter.removeObserver(wakeObserver)
        }
    }

    var recordingActionTitle: String {
        switch recordingState {
        case .idle:
            "Start Recording"
        case .recording:
            "Stop Recording"
        case .transcribing, .processing:
            "Cancel"
        }
    }

    var canEnableGlobalShortcut: Bool {
        permissionSnapshot.accessibilityTrusted && !globalShortcutService.isRunning
    }

    var canCopyLatestTranscript: Bool {
        historyEntries.contains { $0.hasTranscription }
    }

    func clearLastErrorMessage() {
        lastErrorMessage = nil
    }

    var selectedMicrophoneDisplayName: String {
        AudioDeviceService.displayName(for: settings.selectedMicrophoneName, devices: inputDevices)
    }

    var selectedClamshellMicrophoneDisplayName: String {
        AudioDeviceService.displayName(for: settings.clamshellMicrophoneName, devices: inputDevices)
    }

    var selectedOutputDeviceDisplayName: String {
        AudioDeviceService.displayName(for: settings.selectedOutputDeviceName, devices: outputDevices)
    }

    var appVersion: String {
        AppVersionProvider.currentBundleVersion()
    }

    var canUnloadCurrentModelFromMenuBar: Bool {
        localModelRuntimeStates.values.contains { $0.isLoaded }
    }

    var selectedTranscriptionModelDisplayName: String {
        settings.selectedTranscriptionModelDisplayName
    }

    var menuBarModelOptions: [MenuBarModelOption] {
        MenuBarModelOptions.make(
            settings: settings,
            localModelStorageStates: localModelStorageStates
        )
    }

    func refreshPermissions() async {
        permissionSnapshot = permissionService.snapshot()
        refreshGlobalShortcutForPermissionSnapshot()
        refreshOnboardingState()
    }

    private var didRunLaunchPermissionCheck = false

    func refreshPermissionsAtLaunch() async {
        guard didRunLaunchPermissionCheck == false else { return }
        didRunLaunchPermissionCheck = true
        await refreshPermissions()

        if settings.nativeOnboardingCompleted,
           permissionSnapshot.accessibilityTrusted == false {
            // A previously working install lost its grant (typically a re-signed
            // rebuild): ask the OS to show the Accessibility prompt once.
            _ = permissionService.requestAccessibilityPrompt()
            log(.error, "Accessibility permission missing at launch; dictation shortcut is disabled until it is granted.")
        }
    }

    func requestAccessibility() {
        _ = permissionService.requestAccessibilityPrompt()
        permissionSnapshot = permissionService.snapshot()
        refreshGlobalShortcutForPermissionSnapshot()
        refreshOnboardingState()
    }

    func requestMicrophone() {
        Task {
            _ = await permissionService.requestMicrophoneAccess()
            await refreshPermissions()
        }
    }

    func requestSpeechRecognition() {
        Task {
            _ = await permissionService.requestSpeechRecognitionAccess()
            await refreshPermissions()
        }
    }

    func refreshOnboardingState() {
        if let smokeOnboardingStep = launchArguments.smokeOnboardingStep {
            onboardingStep = smokeOnboardingStep
            return
        }

        let nextStep = NativeOnboardingEvaluator.nextStep(
            permissionSnapshot: permissionSnapshot,
            settings: settings,
            localModelStorageStates: localModelStorageStates,
            transcriptionAPIKeyConfigured: transcriptionAPIKeyConfigured,
            bypass: shouldBypassNativeOnboarding
        )
        onboardingStep = nextStep

        if nextStep == .done,
           settings.nativeOnboardingCompleted == false,
           shouldBypassNativeOnboarding == false {
            settings.nativeOnboardingCompleted = true
            settingsStore.save(persistableSettings())
        }
    }

    func toggleRecording() {
        if recordingState.isRecording {
            stopRecording()
        } else if recordingState == .idle {
            startRecording(postProcessRequested: false, shortcutID: nil)
        } else {
            cancelRecording()
        }
    }

    func startRecording(postProcessRequested: Bool = false, shortcutID: String? = nil) {
        guard coordinator.start() else {
            log(.warn, "Ignored start recording request while coordinator was busy (state: \(recordingState)).")
            return
        }

        if permissionSnapshot.microphone == .denied || permissionSnapshot.microphone == .restricted {
            coordinator.cancel()
            recordingState = coordinator.state
            lastErrorMessage = "Microphone access is denied. Enable it in System Settings > Privacy & Security > Microphone."
            presentRecordingOutcome(.failure(message: "Microphone access denied"))
            log(.error, "Recording blocked: microphone permission denied.")
            return
        }

        do {
            lastErrorMessage = nil
            lastRecordingURL = nil
            activeRecordingOperationID = UUID()
            activeRecordingHistoryEntryID = nil
            activeRecordingTask?.cancel()
            activeRecordingTask = nil
            activeRecordingShortcutID = shortcutID
            activeRecordingPostProcessRequested = postProcessRequested
            audioLevel = 0
            showOverlay(.recording)
            prewarmSelectedLocalModelIfAvailable()
            let engineStart = ContinuousClock.now
            try recordingWorkflow.start(
                settings: settings,
                paths: paths,
                selectedMicrophoneName: effectiveRecordingMicrophoneName()
            ) { [weak self] level in
                Task { @MainActor in
                    self?.handleAudioLevel(level)
                }
            }
            let engineElapsed = ContinuousClock.now - engineStart
            log(.info, "Recording started (audio engine ready in \(engineElapsed.formattedMilliseconds)).")
            recordingState = coordinator.state
            refreshGlobalShortcutMonitorIfRunning()
            handleAudioInputVoiceProcessingStatus()
            applyMuteAfterFeedbackDelay()
        } catch {
            coordinator.cancel()
            activeRecordingOperationID = nil
            activeRecordingHistoryEntryID = nil
            activeRecordingTask?.cancel()
            activeRecordingTask = nil
            activeRecordingShortcutID = nil
            activeRecordingPostProcessRequested = false
            recordingState = coordinator.state
            lastErrorMessage = error.localizedDescription
            overlayPanelController.hide(animated: false)
            presentRecordingOutcome(.failure(message: "Could not start recording"))
            log(.error, "Recording failed to start: \(error.localizedDescription)")
        }
    }

    func stopRecording() {
        guard coordinator.stop() else {
            return
        }
        recordingState = coordinator.state
        guard let operationID = activeRecordingOperationID else {
            coordinator.cancel()
            recordingState = coordinator.state
            refreshGlobalShortcutMonitorIfRunning()
            return
        }
        let postProcessRequested = activeRecordingPostProcessRequested
        activeRecordingShortcutID = nil
        activeRecordingPostProcessRequested = false
        refreshGlobalShortcutMonitorIfRunning()
        showOverlay(.transcribing)
        log(.info, "Recording stopped; preparing transcription.")

        activeRecordingTask?.cancel()
        activeRecordingTask = Task { @MainActor [weak self] in
            await self?.stopRecordingAfterTrailingBuffer(
                postProcessRequested: postProcessRequested,
                operationID: operationID
            )
        }
    }

    private func stopRecordingAfterTrailingBuffer(postProcessRequested: Bool, operationID: UUID) async {
        guard isCurrentRecordingOperation(operationID), Task.isCancelled == false else {
            return
        }

        let fileURL: URL
        let historyEntryID: Int64?
        do {
            let capturedRecording = try await recordingWorkflow.stopAfterTrailingBuffer(settings: settings, paths: paths)
            guard capturedRecording.hasAudibleSignal else {
                audioLevel = 0
                lastErrorMessage = "No speech detected. Check that the right microphone is selected and its input level."
                finishRecordingOperation(operationID: operationID)
                presentRecordingOutcome(.notice(message: "No speech detected"))
                log(.warn, "Recording discarded: no audible signal (peak \(capturedRecording.peakAmplitude), rms \(capturedRecording.rootMeanSquare)).")
                return
            }

            var recording = capturedRecording.preparedForTranscriptionInput()
            if recording.isEmpty {
                log(.warn, "Prepared audio was empty; falling back to the untrimmed recording.")
                recording = capturedRecording.paddedForShortTranscriptionInput()
            }
            fileURL = paths.recordingsDirectory
                .appendingPathComponent(recordingFileName(for: recording.startedAt))
            try WAVFileWriter.write(recording, to: fileURL)
            lastRecordingURL = fileURL
            historyEntryID = saveHistoryEntry(for: fileURL, postProcessRequested: postProcessRequested)?.id
            activeRecordingHistoryEntryID = historyEntryID
            audioLevel = 0
            log(.debug, "Recording saved as \(fileURL.lastPathComponent).")
        } catch is CancellationError {
            return
        } catch {
            if isCurrentRecordingOperation(operationID) {
                lastErrorMessage = error.localizedDescription
                finishRecordingOperation(operationID: operationID)
                presentRecordingOutcome(.failure(message: "Recording failed"))
                log(.error, "Recording failed to finish: \(error.localizedDescription)")
            }
            return
        }
        guard isCurrentRecordingOperation(operationID), Task.isCancelled == false else {
            return
        }

        await transcribeAndOutput(
            fileURL: fileURL,
            historyEntryID: historyEntryID,
            postProcessRequested: postProcessRequested,
            operationID: operationID
        )
    }

    func cancelRecording() {
        let pendingHistoryEntryID = activeRecordingHistoryEntryID
        let hadActiveOperation = recordingState.isActive || activeRecordingOperationID != nil || pendingHistoryEntryID != nil
        activeRecordingOperationID = nil
        activeRecordingHistoryEntryID = nil
        activeRecordingTask?.cancel()
        activeRecordingTask = nil
        recordingWorkflow.cancel(settings: settings)
        coordinator.cancel()
        activeRecordingShortcutID = nil
        activeRecordingPostProcessRequested = false
        recordingState = coordinator.state
        refreshGlobalShortcutMonitorIfRunning()
        audioLevel = 0
        deletePendingRecordingHistoryEntryIfNeeded(id: pendingHistoryEntryID)
        overlayPanelController.hide()
        log(.info, hadActiveOperation ? "Recording operation cancelled." : "Cancel ignored because no recording operation was active.")
    }

    func pasteText(_ text: String) {
        Task { @MainActor in
            do {
                lastErrorMessage = nil
                lastOutputText = text
                try await pasteService.paste(text, options: PasteOutputOptions(settings: settings))
                log(.debug, "Manual paste completed.")
            } catch {
                lastErrorMessage = error.localizedDescription
                log(.error, "Manual paste failed: \(error.localizedDescription)")
            }
        }
    }

    func previewFeedbackSounds() {
        audioFeedbackService.play(.start, settings: settings, paths: paths)
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(350))
            audioFeedbackService.play(.stop, settings: settings, paths: paths)
        }
    }

    func updateSettings(_ update: (inout AppSettings) -> Void) {
        let previousLazyStreamClose = settings.lazyStreamClose
        let previousWhisperAccelerator = settings.whisperAccelerator
        let previousAlwaysOnMicrophone = settings.alwaysOnMicrophone
        let previousMicrophoneName = settings.selectedMicrophoneName
        let previousClamshellMicrophoneName = settings.clamshellMicrophoneName
        let previousAppleVoiceProcessingEnabled = settings.appleVoiceProcessingEnabled
        let previousLogLevel = settings.logLevel
        update(&settings)
        settingsStore.save(persistableSettings())
        if previousLogLevel != settings.logLevel {
            log(.info, "Log level changed to \(settings.logLevel.rawValue).")
        }
        if settings.overlayPosition == .none {
            overlayPanelController.hide()
        }
        if previousLazyStreamClose, settings.lazyStreamClose == false {
            audioCaptureService.closeIdleStream()
        }
        if previousWhisperAccelerator != settings.whisperAccelerator {
            Task {
                await whisperKitTranscriptionService.unloadAll()
            }
        }
        if previousAlwaysOnMicrophone != settings.alwaysOnMicrophone ||
            previousMicrophoneName != settings.selectedMicrophoneName ||
            previousClamshellMicrophoneName != settings.clamshellMicrophoneName ||
            previousAppleVoiceProcessingEnabled != settings.appleVoiceProcessingEnabled {
            applyIdleMicrophonePreference()
        }
        if globalShortcutService.isRunning {
            startGlobalShortcutMonitoring()
        } else {
            refreshGlobalShortcutStatus()
        }
        if !AppSection.visibleSections(settings: settings).contains(selectedSection) {
            selectedSection = .advanced
        }
        refreshPostProcessCredentialStatus()
        refreshTranscriptionCredentialStatus()
        refreshOnboardingState()
        cleanupHistoryIfNeeded()
    }

    private func installDebugModeShortcutMonitor() {
        guard debugModeShortcutMonitor == nil else {
            return
        }

        debugModeShortcutMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard DebugModeShortcut.matches(event) else {
                return event
            }

            Task { @MainActor in
                self?.toggleDebugModeFromShortcut()
            }
            return nil
        }
    }

    private func toggleDebugModeFromShortcut() {
        updateSettings {
            $0.debugMode.toggle()
        }
        log(.info, "Debug mode toggled from keyboard shortcut.")
    }

    func applyIdleMicrophonePreference() {
        guard settings.alwaysOnMicrophone else {
            audioCaptureService.closeIdleStream()
            audioInputVoiceProcessingStatus = audioCaptureService.voiceProcessingStatus
            return
        }

        do {
            try audioCaptureService.openIdleStream(
                selectedMicrophoneName: effectiveRecordingMicrophoneName(),
                voiceProcessingConfiguration: settings.audioInputVoiceProcessingConfiguration
            )
            let didWarn = handleAudioInputVoiceProcessingStatus()
            if didWarn == false {
                lastErrorMessage = nil
            }
            log(.debug, "Idle microphone stream opened.")
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "Idle microphone stream failed: \(error.localizedDescription)")
        }
    }

    func handleRemoteControlCommand(_ command: RemoteControlCommand) {
        switch command {
        case .toggleTranscription:
            if recordingState.isRecording,
               activeRecordingShortcutID == ShortcutBinding.transcribeID {
                log(.debug, "Remote control requested transcription stop.")
                stopRecording()
            } else if recordingState == .idle {
                log(.debug, "Remote control requested transcription start.")
                startRecording(
                    postProcessRequested: false,
                    shortcutID: ShortcutBinding.transcribeID
                )
            }
        case .togglePostProcess:
            if recordingState.isRecording,
               activeRecordingShortcutID == ShortcutBinding.transcribeWithPostProcessID {
                log(.debug, "Remote control requested post-process recording stop.")
                stopRecording()
            } else if recordingState == .idle {
                log(.debug, "Remote control requested post-process recording start.")
                startRecording(
                    postProcessRequested: true,
                    shortcutID: ShortcutBinding.transcribeWithPostProcessID
                )
            }
        case .cancel:
            log(.debug, "Remote control requested cancel.")
            cancelRecording()
        }
    }

    func unloadCurrentModelFromMenuBar() {
        Task { @MainActor in
            await whisperKitTranscriptionService.unloadAll()
            await refreshLocalModelRuntimeStatesNow()
            localModelRuntimeRefreshTask?.cancel()
            localModelRuntimeRefreshTask = nil
            lastErrorMessage = nil
            log(.info, "Unloaded local transcription models from menu bar.")
        }
    }

    func setLaunchAtLoginEnabled(_ enabled: Bool) {
        do {
            lastErrorMessage = nil
            try launchAtLoginService.setEnabled(enabled)
            updateSettings {
                $0.autostartEnabled = enabled
            }
            log(.info, "Launch at login set to \(enabled).")
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.error, "Launch at login update failed: \(error.localizedDescription)")
        }

        refreshLaunchAtLoginStatus()
    }

    func refreshAudioDevices() {
        inputDevices = AudioDeviceService.inputDevices()
        outputDevices = AudioDeviceService.outputDevices()
        isLaptop = AudioDeviceService.isLaptop()
    }

    func selectMicrophone(_ device: AudioDevice) {
        updateSettings {
            $0.selectedMicrophoneName = device.isDefault ? nil : device.name
        }
    }

    func selectClamshellMicrophone(_ device: AudioDevice) {
        updateSettings {
            $0.clamshellMicrophoneName = device.isDefault ? nil : device.name
        }
    }

    func selectOutputDevice(_ device: AudioDevice) {
        updateSettings {
            $0.selectedOutputDeviceName = device.isDefault ? nil : device.name
        }
    }

    func addCustomWord(_ word: String) -> Bool {
        guard let sanitized = AppSettings.sanitizeCustomWord(word),
              settings.customWords.contains(sanitized) == false
        else {
            return false
        }

        updateSettings {
            _ = $0.addCustomWord(sanitized)
        }
        return true
    }

    func removeCustomWord(_ word: String) {
        updateSettings {
            $0.removeCustomWord(word)
        }
    }

    func updateShortcutBinding(id: String, currentBinding: String) {
        updateSettings {
            $0.updateShortcutBinding(id: id, currentBinding: currentBinding)
        }
    }

    func selectPostProcessProvider(id: String) {
        updateSettings {
            $0.selectPostProcessProvider(id: id)
        }
        postProcessModelOptions = []
        if id == PostProcessProvider.appleIntelligenceProviderID {
            checkAppleIntelligenceAvailability()
        }
    }

    func checkAppleIntelligenceAvailability() {
        guard appleIntelligenceAvailability != .checking else {
            return
        }

        appleIntelligenceAvailability = .checking
        Task { @MainActor in
            let availability = await Task.detached(priority: .userInitiated) {
                AppleIntelligenceService.availability()
            }.value
            appleIntelligenceAvailability = availability
        }
    }

    func updateSelectedPostProcessModel(_ model: String) {
        updateSettings {
            $0.updateSelectedPostProcessModel(model)
        }
    }

    func updateSelectedPostProcessBaseURL(_ baseURL: String) {
        updateSettings {
            $0.updateSelectedPostProcessBaseURL(baseURL)
        }
    }

    func selectPostProcessPrompt(id: String) {
        updateSettings {
            $0.selectPostProcessPrompt(id: id)
        }
    }

    func addPostProcessPrompt(name: String, prompt: String) {
        updateSettings {
            _ = $0.addPostProcessPrompt(name: name, prompt: prompt)
        }
    }

    func updatePostProcessPrompt(id: String, name: String, prompt: String) {
        updateSettings {
            _ = $0.updatePostProcessPrompt(id: id, name: name, prompt: prompt)
        }
    }

    func deletePostProcessPrompt(id: String) {
        updateSettings {
            _ = $0.deletePostProcessPrompt(id: id)
        }
    }

    func savePostProcessAPIKey(_ apiKey: String) {
        do {
            try postProcessCredentialStore.saveAPIKey(apiKey, providerID: settings.postProcessProviderID)
            lastErrorMessage = nil
            log(.info, "Saved post-processing API key for provider \(settings.postProcessProviderID).")
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.error, "Saving post-processing API key failed: \(error.localizedDescription)")
        }
        refreshPostProcessCredentialStatus()
    }

    func clearPostProcessAPIKey() {
        do {
            try postProcessCredentialStore.deleteAPIKey(providerID: settings.postProcessProviderID)
            lastErrorMessage = nil
            log(.info, "Cleared post-processing API key for provider \(settings.postProcessProviderID).")
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.error, "Clearing post-processing API key failed: \(error.localizedDescription)")
        }
        refreshPostProcessCredentialStatus()
    }

    func fetchPostProcessModels() {
        guard !isFetchingPostProcessModels,
              let provider = settings.selectedPostProcessProvider
        else {
            return
        }

        isFetchingPostProcessModels = true
        Task { @MainActor in
            defer { isFetchingPostProcessModels = false }

            do {
                let models = try await PostProcessingService.fetchModels(
                    provider: provider,
                    credentialStore: postProcessCredentialStore
                )
                postProcessModelOptions = models
                lastErrorMessage = nil
                log(.debug, "Fetched \(models.count) post-processing models for provider \(provider.id).")
            } catch {
                lastErrorMessage = error.localizedDescription
                postProcessModelOptions = []
                log(.warn, "Fetching post-processing models failed: \(error.localizedDescription)")
            }
        }
    }

    func selectTranscriptionAPIProvider(id: String) {
        updateSettings {
            $0.selectTranscriptionAPIProvider(id: id)
        }
    }

    func updateSelectedTranscriptionAPIBaseURL(_ baseURL: String) {
        updateSettings {
            $0.updateSelectedTranscriptionAPIBaseURL(baseURL)
        }
    }

    func saveTranscriptionAPIKey(_ apiKey: String) {
        do {
            try postProcessCredentialStore.saveAPIKey(apiKey, providerID: settings.transcriptionAPIProviderID)
            lastErrorMessage = nil
            log(.info, "Saved transcription API key for provider \(settings.transcriptionAPIProviderID).")
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.error, "Saving transcription API key failed: \(error.localizedDescription)")
        }
        refreshTranscriptionCredentialStatus()
    }

    func clearTranscriptionAPIKey() {
        do {
            try postProcessCredentialStore.deleteAPIKey(providerID: settings.transcriptionAPIProviderID)
            lastErrorMessage = nil
            log(.info, "Cleared transcription API key for provider \(settings.transcriptionAPIProviderID).")
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.error, "Clearing transcription API key failed: \(error.localizedDescription)")
        }
        refreshTranscriptionCredentialStatus()
    }

    func useAppleSpeechTranscription() {
        updateSettings {
            $0.selectTranscriptionModel(id: TranscriptionAPIProvider.appleSpeechModelID)
        }
        if permissionSnapshot.speechRecognition != .granted {
            requestSpeechRecognition()
        }
    }

    func selectTranscriptionModel(id: String) {
        updateSettings {
            $0.selectTranscriptionModel(id: id)
        }
    }

    func refreshLocalModelStorageStates() {
        localModelStorageStates = LocalModelStorageService.states(paths: paths)
        refreshOnboardingState()
    }

    func refreshLocalModelRuntimeStates() {
        Task { @MainActor in
            await refreshLocalModelRuntimeStatesNow()
        }
    }

    func localModelStorageState(for model: LocalTranscriptionModel) -> LocalModelStorageState {
        localModelStorageStates[model.id] ?? LocalModelStorageService.state(for: model, paths: paths)
    }

    func localModelRuntimeState(for model: LocalTranscriptionModel) -> LocalModelRuntimeState {
        localModelRuntimeStates[model.id] ?? LocalModelRuntimeState(modelID: model.id, isLoaded: false)
    }

    private func refreshLocalModelRuntimeStatesNow() async {
        let loadedModelIDs = await whisperKitTranscriptionService.loadedModelIDs()
        localModelRuntimeStates = Dictionary(uniqueKeysWithValues: LocalTranscriptionModel.catalog.map { model in
            (
                model.id,
                LocalModelRuntimeState(
                    modelID: model.id,
                    isLoaded: loadedModelIDs.contains(model.id)
                )
            )
        })
    }

    func isDownloadingLocalModel(id: String) -> Bool {
        localModelDownloadStates[id] != nil
    }

    func localModelDownloadState(for id: String) -> LocalModelDownloadState? {
        localModelDownloadStates[id]
    }

    func downloadLocalTranscriptionModel(id: String, selectWhenReady: Bool = false) {
        guard let model = LocalTranscriptionModel.model(for: id),
              localModelDownloadTasks[id] == nil
        else {
            return
        }

        if localModelStorageState(for: model).isDownloaded {
            if selectWhenReady {
                selectTranscriptionModel(id: id)
            }
            return
        }

        localModelDownloadStates[id] = LocalModelDownloadState(modelID: id)
        lastErrorMessage = nil
        log(.info, "Starting local model download: \(id).")

        let progressChannel = AsyncStream<Double>.makeStream()
        let progressStream = progressChannel.stream
        let progressContinuation = progressChannel.continuation
        let progressTask = Task { @MainActor [weak self] in
            for await fractionCompleted in progressStream {
                self?.updateLocalModelDownloadState(id: id, fractionCompleted: fractionCompleted)
            }
        }

        let task = Task { @MainActor in
            defer {
                progressContinuation.finish()
                progressTask.cancel()
                localModelDownloadTasks[id] = nil
                localModelDownloadStates[id] = nil
                refreshLocalModelStorageStates()
            }

            do {
                try await whisperKitTranscriptionService.download(
                    model: model,
                    paths: paths
                ) { fractionCompleted in
                    progressContinuation.yield(fractionCompleted)
                }
                try Task.checkCancellation()
                lastErrorMessage = nil
                log(.info, "Local model download completed: \(id).")
                if selectWhenReady {
                    selectTranscriptionModel(id: id)
                }
            } catch is CancellationError {
                lastErrorMessage = nil
                log(.info, "Local model download cancelled: \(id).")
            } catch {
                lastErrorMessage = error.localizedDescription
                log(.error, "Local model download failed for \(id): \(error.localizedDescription)")
            }
        }
        localModelDownloadTasks[id] = task
    }

    func cancelLocalTranscriptionModelDownload(id: String) {
        guard let task = localModelDownloadTasks[id] else {
            return
        }

        if var state = localModelDownloadStates[id] {
            state.isCancelling = true
            localModelDownloadStates[id] = state
        }
        log(.debug, "Cancelling local model download: \(id).")
        task.cancel()
    }

    func deleteLocalTranscriptionModel(id: String) {
        guard let model = LocalTranscriptionModel.model(for: id) else {
            return
        }

        do {
            try LocalModelStorageService.delete(model: model, paths: paths)
            lastErrorMessage = nil
            refreshLocalModelStorageStates()
            log(.info, "Deleted local model cache: \(id).")
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.error, "Deleting local model cache failed for \(id): \(error.localizedDescription)")
        }
    }

    func openLogsFolder() {
        NSWorkspace.shared.open(paths.logsDirectory)
        log(.debug, "Opened logs folder.")
    }

    private func updateLocalModelDownloadState(id: String, fractionCompleted: Double) {
        guard localModelDownloadStates[id] != nil else {
            return
        }
        localModelDownloadStates[id] = LocalModelDownloadState(modelID: id, fractionCompleted: fractionCompleted)
    }

    func addSelectedTranscriptionAPIModel(modelID: String, displayName: String) {
        updateSettings {
            _ = $0.addTranscriptionAPIModelForSelectedProvider(modelID: modelID, displayName: displayName)
        }
    }

    func useSelectedTranscriptionAPIModel(modelID: String) {
        updateSettings {
            _ = $0.upsertTranscriptionAPIModelForSelectedProvider(modelID: modelID)
        }
    }

    func reloadHistory() {
        guard let historyStore else {
            return
        }

        do {
            let page = try historyStore.entries(limit: historyPageSize)
            historyEntries = page.entries
            historyHasMore = page.hasMore
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "Loading history failed: \(error.localizedDescription)")
        }
    }

    func loadMoreHistory() {
        guard let historyStore, historyHasMore, let cursor = historyEntries.last?.id else {
            return
        }

        do {
            let page = try historyStore.entries(cursor: cursor, limit: historyPageSize)
            historyEntries.append(contentsOf: page.entries)
            historyHasMore = page.hasMore
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "Loading more history failed: \(error.localizedDescription)")
        }
    }

    func toggleHistoryEntrySaved(id: Int64) {
        guard let historyStore else {
            return
        }

        do {
            let updated = try historyStore.toggleSavedStatus(id: id)
            if let index = historyEntries.firstIndex(where: { $0.id == id }) {
                historyEntries[index] = updated
            }
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "Toggling history saved state failed: \(error.localizedDescription)")
        }
    }

    func copyHistoryText(_ entry: HistoryEntry) {
        copyHistoryTextValue(entry.transcriptionText, entryID: entry.id)
    }

    func copyLatestTranscript() {
        guard let historyStore else {
            return
        }

        do {
            guard let entry = try historyStore.latestCompletedEntry() else {
                return
            }
            copyHistoryTextValue(entry.outputText, entryID: entry.id)
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "Copying latest transcript failed: \(error.localizedDescription)")
        }
    }

    private func copyHistoryTextValue(_ value: String, entryID: Int64) {
        let text = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard text.isEmpty == false else {
            return
        }

        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
        lastOutputText = text
        copiedHistoryEntryID = entryID

        Task { @MainActor in
            try? await Task.sleep(for: .seconds(2))
            if copiedHistoryEntryID == entryID {
                copiedHistoryEntryID = nil
            }
        }
    }

    func deleteHistoryEntry(id: Int64) {
        guard let historyStore else {
            return
        }

        do {
            audioPlaybackService.stopIfPlaying(entryID: id)
            try historyStore.deleteEntry(id: id)
            historyEntries.removeAll { $0.id == id }
            log(.debug, "Deleted history entry \(id).")
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "Deleting history entry failed: \(error.localizedDescription)")
        }
    }

    func retryHistoryEntryTranscription(id: Int64) {
        guard let historyStore,
              retryingHistoryEntryIDs.contains(id) == false,
              recordingState == .idle
        else {
            return
        }

        retryingHistoryEntryIDs.insert(id)
        recordingState = .processing
        lastErrorMessage = nil

        Task { @MainActor in
            defer {
                retryingHistoryEntryIDs.remove(id)
                recordingState = coordinator.state
            }

            do {
                guard let entry = try historyStore.entry(id: id) else {
                    throw HistoryStoreError.missingEntry(id)
                }

                let audioURL = historyStore.audioFileURL(fileName: entry.fileName)
                guard FileManager.default.fileExists(atPath: audioURL.path) else {
                    throw HistoryRetryError.missingAudioFile(entry.fileName)
                }

                let result = try await transcribeAudioFile(
                    audioURL,
                    postProcessRequested: entry.postProcessRequested
                )
                updateHistoryTranscription(
                    id: id,
                    text: result.transcriptionText,
                    postProcessedText: result.postProcessedText,
                    postProcessPrompt: result.postProcessPrompt
                )
                refreshLocalModelStorageStates()
                await refreshLocalModelRuntimeStatesNow()
                scheduleLocalModelRuntimeStateRefreshAfterCurrentUnloadTimeout()
                log(.info, "Retried history transcription for entry \(id).")
            } catch {
                lastErrorMessage = error.localizedDescription
                log(.error, "Retrying history transcription failed for entry \(id): \(error.localizedDescription)")
            }
        }
    }

    func openRecordingsFolder() {
        NSWorkspace.shared.open(paths.recordingsDirectory)
        log(.debug, "Opened recordings folder.")
    }

    func toggleHistoryAudioPlayback(_ entry: HistoryEntry) {
        guard let historyStore else {
            return
        }

        do {
            try audioPlaybackService.toggle(
                entryID: entry.id,
                fileURL: historyStore.audioFileURL(fileName: entry.fileName)
            )
            lastErrorMessage = nil
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "Audio playback failed: \(error.localizedDescription)")
        }
    }

    func seekHistoryAudio(_ entry: HistoryEntry, to time: TimeInterval) {
        audioPlaybackService.seek(entryID: entry.id, to: time)
    }

    private func recordingFileName(for date: Date) -> String {
        RecordingFileNameFormatter.fileName(for: date)
    }

    private func prewarmSelectedLocalModelIfAvailable() {
        guard let model = LocalModelPrewarmDecision.modelToPrewarm(
            settings: settings,
            storageStates: localModelStorageStates
        ),
            prewarmingLocalModelIDs.contains(model.id) == false
        else {
            return
        }

        prewarmingLocalModelIDs.insert(model.id)
        Task { @MainActor in
            defer {
                prewarmingLocalModelIDs.remove(model.id)
            }

            do {
                try await whisperKitTranscriptionService.prepare(
                    model: model,
                    settings: settings,
                    paths: paths,
                    unloadTimeout: nil
                )
                await refreshLocalModelRuntimeStatesNow()
                scheduleLocalModelRuntimeStateRefreshAfterCurrentUnloadTimeout()
            } catch {
                log(.debug, "Prewarming local model \(model.id) failed: \(error.localizedDescription)")
            }
        }
    }

    private func scheduleLocalModelRuntimeStateRefreshAfterCurrentUnloadTimeout() {
        localModelRuntimeRefreshTask?.cancel()

        guard let delaySeconds = settings.modelUnloadTimeout.unloadDelaySeconds else {
            localModelRuntimeRefreshTask = nil
            return
        }

        localModelRuntimeRefreshTask = Task { @MainActor [weak self] in
            if delaySeconds > 0 {
                try? await Task.sleep(for: .seconds(delaySeconds + 1))
            }
            guard Task.isCancelled == false else {
                return
            }
            await self?.refreshLocalModelRuntimeStatesNow()
        }
    }

    private func saveHistoryEntry(for fileURL: URL, postProcessRequested: Bool) -> HistoryEntry? {
        guard let historyStore else {
            return nil
        }

        do {
            let entry = try historyStore.saveEntry(
                fileName: fileURL.lastPathComponent,
                transcriptionText: "",
                postProcessRequested: postProcessRequested
            )
            historyEntries.insert(entry, at: 0)
            activeRecordingHistoryEntryID = entry.id
            cleanupHistoryIfNeeded()
            return entry
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "Saving history entry failed: \(error.localizedDescription)")
            return nil
        }
    }

    private func transcribeAndOutput(
        fileURL: URL,
        historyEntryID: Int64?,
        postProcessRequested: Bool,
        operationID: UUID
    ) async {
        var failureOutcome: RecordingOverlayState?
        defer {
            finishRecordingOperation(operationID: operationID)
            if let failureOutcome {
                presentRecordingOutcome(failureOutcome)
            }
        }

        do {
            guard isCurrentRecordingOperation(operationID), Task.isCancelled == false else {
                return
            }
            recordingState = .processing
            refreshGlobalShortcutMonitorIfRunning()
            showOverlay(.processing)
            let result = try await transcribeAudioFile(
                fileURL,
                postProcessRequested: postProcessRequested
            )
            guard isCurrentRecordingOperation(operationID), Task.isCancelled == false else {
                return
            }
            lastOutputText = result.outputText

            if let historyEntryID {
                updateHistoryTranscription(
                    id: historyEntryID,
                    text: result.transcriptionText,
                    postProcessedText: result.postProcessedText,
                    postProcessPrompt: result.postProcessPrompt
                )
            }

            refreshLocalModelStorageStates()
            await refreshLocalModelRuntimeStatesNow()
            scheduleLocalModelRuntimeStateRefreshAfterCurrentUnloadTimeout()

            guard isCurrentRecordingOperation(operationID), Task.isCancelled == false else {
                return
            }
            try await pasteService.paste(result.outputText, options: PasteOutputOptions(settings: settings))
            log(.info, "Transcription completed and output inserted.")
        } catch is CancellationError {
            log(.info, "Transcription operation cancelled.")
        } catch {
            if isCurrentRecordingOperation(operationID) {
                lastErrorMessage = error.localizedDescription
                failureOutcome = .failure(message: "Transcription failed")
                log(.error, "Transcription or output insertion failed: \(error.localizedDescription)")
            }
        }
    }

    private func isCurrentRecordingOperation(_ operationID: UUID) -> Bool {
        activeRecordingOperationID == operationID
    }

    private func finishRecordingOperation(operationID: UUID) {
        guard isCurrentRecordingOperation(operationID) else {
            return
        }

        activeRecordingOperationID = nil
        activeRecordingHistoryEntryID = nil
        activeRecordingTask = nil
        coordinator.finishProcessing()
        recordingState = coordinator.state
        refreshGlobalShortcutMonitorIfRunning()
        overlayPanelController.hide()
    }

    private func transcribeAudioFile(_ fileURL: URL, postProcessRequested: Bool) async throws -> ProcessedAudioTranscription {
        try await AudioFileTranscriptionPipeline.transcribe(
            fileURL: fileURL,
            settings: settings,
            paths: paths,
            credentialStore: postProcessCredentialStore,
            appleSpeechTranscriptionService: appleSpeechTranscriptionService,
            whisperKitTranscriptionService: whisperKitTranscriptionService,
            postProcessRequested: postProcessRequested
        )
    }

    private func updateHistoryTranscription(
        id: Int64,
        text: String,
        postProcessedText: String? = nil,
        postProcessPrompt: String? = nil
    ) {
        guard let historyStore else {
            return
        }

        do {
            let updated = try historyStore.updateTranscription(
                id: id,
                transcriptionText: text,
                postProcessedText: postProcessedText,
                postProcessPrompt: postProcessPrompt
            )
            if let index = historyEntries.firstIndex(where: { $0.id == id }) {
                historyEntries[index] = updated
            }
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "Updating history transcription failed: \(error.localizedDescription)")
        }
    }

    private func deletePendingRecordingHistoryEntryIfNeeded(id: Int64?) {
        guard let id,
              let historyStore
        else {
            return
        }

        do {
            if try historyStore.deleteEntryIfPending(id: id) {
                historyEntries.removeAll { $0.id == id }
                log(.debug, "Deleted pending history entry \(id) after recording cancellation.")
            }
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "Deleting pending recording history entry failed: \(error.localizedDescription)")
        }
    }

    private func cleanupHistoryIfNeeded() {
        guard let historyStore else {
            return
        }

        do {
            try historyStore.cleanup(
                retentionPeriod: settings.recordingRetentionPeriod,
                historyLimit: settings.historyLimit,
                excludingID: activeRecordingHistoryEntryID
            )
            reloadHistory()
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "History cleanup failed: \(error.localizedDescription)")
        }
    }

    private func handleAudioLevel(_ level: Float) {
        audioLevel = level
        guard settings.overlayPosition != .none, recordingState.isActive else {
            return
        }

        overlayPanelController.updateLevels(RecordingOverlayLevelMapper.levels(from: level))
    }

    private func applyMuteAfterFeedbackDelay() {
        Task { @MainActor [weak self] in
            guard let self else {
                return
            }
            await recordingWorkflow.applyMuteAfterStartFeedback(settings: settings) {
                recordingState.isRecording
            }
        }
    }

    private func showOverlay(_ state: RecordingOverlayState) {
        guard settings.overlayPosition != .none else {
            return
        }

        overlayPanelController.show(
            state: state,
            palette: settings.appTheme.palette,
            position: settings.overlayPosition
        )
    }

    private func presentRecordingOutcome(_ state: RecordingOverlayState) {
        guard settings.overlayPosition != .none else {
            return
        }

        overlayPanelController.showTransientOutcome(
            state: state,
            palette: settings.appTheme.palette,
            position: settings.overlayPosition
        )
    }

    private func showOverlayForVisualSmoke(_ state: RecordingOverlayState) {
        let position = settings.overlayPosition == .none ? OverlayPosition.bottom : settings.overlayPosition
        overlayPanelController.show(
            state: state,
            palette: settings.appTheme.palette,
            position: position
        )
        if state == .recording {
            overlayPanelController.updateLevels([0.16, 0.34, 0.58, 0.82, 1, 0.74, 0.5, 0.29, 0.18])
        }
    }

    private static func writeOverlaySmokeDiagnostics(
        _ diagnostics: RecordingOverlayPanelDiagnostics,
        to path: String
    ) throws {
        let outputURL = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(diagnostics)
        try data.write(to: outputURL, options: .atomic)
    }

    private static func writeOverlaySmokeImage(_ data: Data, to path: String) throws {
        let outputURL = URL(fileURLWithPath: path)
        try FileManager.default.createDirectory(
            at: outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try data.write(to: outputURL, options: .atomic)
    }

    private func refreshTranscriptionCredentialStatus() {
        transcriptionAPIKeyConfigured = postProcessCredentialStore.hasAPIKey(providerID: settings.transcriptionAPIProviderID)
        refreshOnboardingState()
    }

    func enableGlobalShortcutMonitoring() {
        startGlobalShortcutMonitoring()
    }

    private func startGlobalShortcutMonitoring() {
        guard permissionSnapshot.accessibilityTrusted else {
            globalShortcutService.stop()
            globalShortcutStatus = "Accessibility required"
            log(.warn, "Global shortcut monitoring requires Accessibility permission.")
            return
        }

        let registrations = globalShortcutRegistrations()
        guard registrations.isEmpty == false else {
            globalShortcutService.stop()
            globalShortcutStatus = "No valid shortcut"
            log(.warn, "Global shortcut monitoring skipped because no valid shortcut is configured.")
            return
        }

        do {
            try globalShortcutService.start(registrations: registrations) { [weak self] bindingID in
                Task { @MainActor in
                    self?.handleGlobalShortcutPressed(bindingID: bindingID)
                }
            } onReleased: { [weak self] bindingID in
                Task { @MainActor in
                    self?.handleGlobalShortcutReleased(bindingID: bindingID)
                }
            }
            globalShortcutStatus = activeTranscribeShortcutStatus()
            log(.info, "Global shortcut monitoring started.")
        } catch {
            globalShortcutStatus = "Shortcut unavailable"
            lastErrorMessage = error.localizedDescription
            log(.error, "Global shortcut monitoring failed: \(error.localizedDescription)")
        }
    }

    private func refreshGlobalShortcutStatus() {
        if globalShortcutService.isRunning {
            globalShortcutStatus = activeTranscribeShortcutStatus()
        } else if permissionSnapshot.accessibilityTrusted {
            globalShortcutStatus = "Ready to enable"
        } else {
            globalShortcutStatus = "Accessibility required"
        }
    }

    private func activeTranscribeShortcutStatus() -> String {
        guard let resolution = Self.descriptorWithFallback(for: settings.transcribeShortcutBinding) else {
            return "Transcribe shortcut not set"
        }

        if resolution.usedFallback {
            return "\(ShortcutBinding.displayName(for: settings.transcribeShortcutBinding.defaultBinding)) active (default — stored shortcut invalid)"
        }

        return "\(settings.transcribeShortcutBinding.displayBinding) active"
    }

    private func refreshGlobalShortcutForPermissionSnapshot() {
        if permissionSnapshot.accessibilityTrusted {
            startGlobalShortcutMonitoring()
        } else {
            refreshGlobalShortcutStatus()
        }
    }

    static let secureInputStatusMessage = "Suspended: another app holds secure keyboard entry"

    private func startShortcutHealthWatchdog() {
        shortcutHealthTask?.cancel()
        shortcutHealthTask = Task { @MainActor [weak self] in
            while Task.isCancelled == false {
                try? await Task.sleep(for: .seconds(10))
                self?.runShortcutHealthCheck()
            }
        }

        wakeObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didWakeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor in
                self?.runShortcutHealthCheck()
            }
        }
    }

    private func runShortcutHealthCheck() {
        let snapshot = permissionService.snapshot()
        let action = ShortcutHealthPolicy.assess(
            accessibilityTrusted: snapshot.accessibilityTrusted,
            tapInstalled: globalShortcutService.isRunning,
            tapHealthy: globalShortcutService.tapHealth == .healthy,
            secureInputActive: IsSecureEventInputEnabled(),
            recordingActive: recordingState.isActive
        )

        switch action {
        case .none:
            if globalShortcutStatus == Self.secureInputStatusMessage {
                refreshGlobalShortcutStatus()
            }
        case .install, .reinstall:
            permissionSnapshot = snapshot
            log(.warn, "Shortcut health check: reinstalling global shortcut monitor (\(action == .install ? "not installed" : "tap dead")).")
            startGlobalShortcutMonitoring()
        case .teardownAndWarn:
            permissionSnapshot = snapshot
            globalShortcutService.stop()
            refreshGlobalShortcutStatus()
            log(.error, "Accessibility permission was revoked; dictation shortcut disabled until it is granted again.")
        case .warnSecureInput:
            guard globalShortcutStatus != Self.secureInputStatusMessage else {
                break
            }
            globalShortcutStatus = Self.secureInputStatusMessage
            log(.warn, "Global shortcut suspended: another process has secure keyboard entry enabled.")
        }
    }

    private func handleGlobalShortcutPressed(bindingID: String) {
        perform(
            GlobalShortcutActionRouter.action(
                for: .pressed,
                bindingID: bindingID,
                context: globalShortcutActionContext
            )
        )
    }

    private func handleGlobalShortcutReleased(bindingID: String) {
        perform(
            GlobalShortcutActionRouter.action(
                for: .released,
                bindingID: bindingID,
                context: globalShortcutActionContext
            )
        )
    }

    private var globalShortcutActionContext: GlobalShortcutActionContext {
        GlobalShortcutActionContext(
            pushToTalk: settings.pushToTalk,
            recordingState: recordingState,
            activeRecordingShortcutID: activeRecordingShortcutID
        )
    }

    private func perform(_ shortcutAction: GlobalShortcutAction) {
        switch shortcutAction {
        case .none:
            return
        case let .startRecording(postProcessRequested, shortcutID):
            startRecording(postProcessRequested: postProcessRequested, shortcutID: shortcutID)
        case .stopRecording:
            stopRecording()
        case .cancelRecording:
            cancelRecording()
        }
    }

    private func refreshPostProcessCredentialStatus() {
        postProcessAPIKeyConfigured = postProcessCredentialStore.hasAPIKey(providerID: settings.postProcessProviderID)
    }

    private var shouldBypassNativeOnboarding: Bool {
        launchArguments.initialSection != nil ||
            launchArguments.smokeOverlayState != nil ||
            launchArguments.smokeOnboardingStep != nil ||
            launchArguments.remoteCommand != nil
    }

    private func refreshGlobalShortcutMonitorIfRunning() {
        if globalShortcutService.isRunning {
            startGlobalShortcutMonitoring()
        }
    }

    private func effectiveRecordingMicrophoneName() -> String? {
        AudioDeviceService.effectiveInputDeviceName(
            selectedMicrophoneName: settings.selectedMicrophoneName,
            clamshellMicrophoneName: settings.clamshellMicrophoneName,
            isClamshellClosed: AudioDeviceService.isClamshellClosed()
        )
    }

    @discardableResult
    private func handleAudioInputVoiceProcessingStatus() -> Bool {
        audioInputVoiceProcessingStatus = audioCaptureService.voiceProcessingStatus
        guard settings.appleVoiceProcessingEnabled,
              case let .unavailable(reason) = audioInputVoiceProcessingStatus
        else {
            return false
        }

        let warning = "Apple voice processing unavailable for this microphone; using raw input."
        lastErrorMessage = warning
        log(.warn, "\(warning) Reason: \(reason)")
        return true
    }

    static func descriptorWithFallback(for binding: ShortcutBinding) -> (descriptor: GlobalShortcutDescriptor, usedFallback: Bool)? {
        if let descriptor = GlobalShortcutDescriptor.parse(binding.currentBinding) {
            return (descriptor, false)
        }
        if let descriptor = GlobalShortcutDescriptor.parse(binding.defaultBinding) {
            return (descriptor, true)
        }
        return nil
    }

    private func globalShortcutRegistrations() -> [GlobalShortcutRegistration] {
        var registrations: [GlobalShortcutRegistration] = []

        if let resolution = Self.descriptorWithFallback(for: settings.transcribeShortcutBinding) {
            if resolution.usedFallback {
                log(.warn, "Stored shortcut '\(settings.transcribeShortcutBinding.currentBinding)' is not usable; falling back to \(settings.transcribeShortcutBinding.defaultBinding).")
            }
            registrations.append(
                GlobalShortcutRegistration(
                    bindingID: ShortcutBinding.transcribeID,
                    descriptor: resolution.descriptor
                )
            )
        }

        if settings.postProcessEnabled,
           let resolution = Self.descriptorWithFallback(for: settings.transcribeWithPostProcessShortcutBinding) {
            registrations.append(
                GlobalShortcutRegistration(
                    bindingID: ShortcutBinding.transcribeWithPostProcessID,
                    descriptor: resolution.descriptor
                )
            )
        }

        if recordingState.isActive,
           let resolution = Self.descriptorWithFallback(for: settings.cancelShortcutBinding) {
            registrations.append(
                GlobalShortcutRegistration(
                    bindingID: ShortcutBinding.cancelID,
                    descriptor: resolution.descriptor
                )
            )
        }

        return registrations
    }

    private static func applyingRuntimeOverrides(
        _ launchArguments: NativeLaunchArguments,
        to persistedSettings: AppSettings
    ) -> AppSettings {
        var settings = persistedSettings
        if launchArguments.debug {
            settings.debugMode = true
        }
        if launchArguments.noTray {
            settings.showMenuBarIcon = false
        }
        return settings
    }

    private func persistableSettings() -> AppSettings {
        var settingsToSave = settings
        if launchArguments.debug {
            settingsToSave.debugMode = persistedDebugModeAtLaunch
        }
        if launchArguments.noTray {
            settingsToSave.showMenuBarIcon = persistedShowMenuBarIconAtLaunch
        }
        return settingsToSave
    }

    private func synchronizeLaunchAtLoginWithStoredSetting() {
        refreshLaunchAtLoginStatus()

        guard ProcessInfo.processInfo.environment["MURMUR_APP_DATA_DIR"] == nil,
              launchAtLoginStatus.shouldApplyStoredSetting(settings.autostartEnabled)
        else {
            return
        }

        do {
            try launchAtLoginService.setEnabled(settings.autostartEnabled)
        } catch {
            lastErrorMessage = error.localizedDescription
            log(.warn, "Launch at login synchronization failed: \(error.localizedDescription)")
        }

        refreshLaunchAtLoginStatus()
    }

    private func refreshLaunchAtLoginStatus() {
        launchAtLoginStatus = launchAtLoginService.currentStatus()
    }

    private func log(_ level: NativeLogLevel, _ message: String) {
        _ = try? logStore.write(level, message, minimumLevel: settings.logLevel)
    }
}

private enum HistoryRetryError: LocalizedError {
    case missingAudioFile(String)

    var errorDescription: String? {
        switch self {
        case let .missingAudioFile(fileName):
            "Recording audio file '\(fileName)' was not found."
        }
    }
}

private extension Duration {
    var formattedMilliseconds: String {
        let milliseconds = components.seconds * 1_000 + components.attoseconds / 1_000_000_000_000_000
        return "\(milliseconds) ms"
    }
}
