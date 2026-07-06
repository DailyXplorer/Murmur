@testable import MurmurNative
import Foundation
import XCTest

final class LocalTranscriptionModelTests: XCTestCase {
    func testLocalCatalogKeepsMurmurWhisperIDsSelectable() {
        let ids = Set(LocalTranscriptionModel.catalog.map(\.id))

        XCTAssertTrue(ids.contains("small"))
        XCTAssertTrue(ids.contains("medium"))
        XCTAssertTrue(ids.contains("turbo"))
        XCTAssertTrue(ids.contains("large"))
    }

    func testSelectingLocalModelUpdatesDisplayAndTranslationSupport() {
        var settings = AppSettings.defaults

        settings.selectTranscriptionModel(id: "medium")

        XCTAssertEqual(settings.selectedModel, "medium")
        XCTAssertEqual(settings.selectedTranscriptionModelDisplayName, "Whisper Medium")
        XCTAssertTrue(settings.selectedTranscriptionSupportsTranslation)
        XCTAssertTrue(settings.selectedTranscriptionSupportsLanguageSelection)
        XCTAssertTrue(settings.selectedTranscriptionHasModelSettings)
    }

    func testTurboMatchesCurrentMurmurTranslationContract() {
        var settings = AppSettings.defaults

        settings.selectTranscriptionModel(id: "turbo")

        XCTAssertEqual(settings.selectedModel, "turbo")
        XCTAssertEqual(settings.selectedTranscriptionModelDisplayName, "Whisper Turbo")
        XCTAssertFalse(settings.selectedTranscriptionSupportsTranslation)
        XCTAssertTrue(settings.selectedTranscriptionSupportsLanguageSelection)
    }

    func testUnknownModelSelectionUsesRawDisplayNameAndRemainsUnavailable() {
        var settings = AppSettings.defaults
        settings.selectedModel = "parakeet-tdt-0.6b-v3"

        XCTAssertEqual(settings.selectedTranscriptionModelDisplayName, "parakeet-tdt-0.6b-v3")
        XCTAssertFalse(settings.selectedTranscriptionSupportsLanguageSelection)
        XCTAssertFalse(settings.selectedTranscriptionSupportsTranslation)
        XCTAssertFalse(settings.selectedTranscriptionHasModelSettings)
    }

    func testAppleSpeechShowsLanguageSettingsWithoutTranslation() {
        var settings = AppSettings.defaults

        settings.selectTranscriptionModel(id: TranscriptionAPIProvider.appleSpeechModelID)

        XCTAssertTrue(settings.selectedTranscriptionSupportsLanguageSelection)
        XCTAssertFalse(settings.selectedTranscriptionSupportsTranslation)
        XCTAssertTrue(settings.selectedTranscriptionHasModelSettings)
    }

    func testAppleSpeechPresentationHighlightsMacOS27DictationSupport() {
        XCTAssertEqual(AppleSpeechModelPresentation.title, "Apple Speech")
        XCTAssertTrue(AppleSpeechModelPresentation.description.contains("macOS 27"))
        XCTAssertTrue(AppleSpeechModelPresentation.description.contains("Apple silicon Macs"))
        XCTAssertTrue(AppleSpeechModelPresentation.description.contains("MacBook Air/Pro 2020+"))
        XCTAssertTrue(AppleSpeechModelPresentation.description.contains("Mac mini 2020+"))
        XCTAssertEqual(AppleSpeechModelPresentation.speedScore, 0.90, accuracy: 0.0001)
        XCTAssertEqual(AppleSpeechModelPresentation.accuracyScore, 0.84, accuracy: 0.0001)
    }

    func testUnknownModelIsNotSelectedThroughNativeSelector() {
        var settings = AppSettings.defaults
        let originalModel = settings.selectedModel

        settings.selectTranscriptionModel(id: "parakeet-tdt-0.6b-v3")

        XCTAssertEqual(settings.selectedModel, originalModel)
    }

    func testPrewarmDecisionRequiresDownloadedSelectedLocalModel() {
        var settings = AppSettings.defaults
        settings.selectTranscriptionModel(id: "turbo")

        XCTAssertNil(
            LocalModelPrewarmDecision.modelToPrewarm(
                settings: settings,
                storageStates: [:]
            )
        )
        XCTAssertEqual(
            LocalModelPrewarmDecision.modelToPrewarm(
                settings: settings,
                storageStates: [
                    "turbo": LocalModelStorageState(
                        modelID: "turbo",
                        isDownloaded: true,
                        byteCount: 12,
                        directories: []
                    )
                ]
            )?.id,
            "turbo"
        )
    }

    func testPrewarmDecisionIgnoresNonLocalModels() {
        var settings = AppSettings.defaults
        settings.selectTranscriptionModel(id: TranscriptionAPIProvider.appleSpeechModelID)

        XCTAssertNil(
            LocalModelPrewarmDecision.modelToPrewarm(
                settings: settings,
                storageStates: [
                    "turbo": LocalModelStorageState(
                        modelID: "turbo",
                        isDownloaded: true,
                        byteCount: 12,
                        directories: []
                    )
                ]
            )
        )
    }

    func testModelsLanguageFilterSearchExcludesAutoDetect() {
        let filteredLanguages = ModelsLanguageFilter.filteredLanguages(searchText: "chin")

        XCTAssertFalse(ModelsLanguageFilter.selectableLanguages.contains { $0.code == "auto" })
        XCTAssertEqual(
            filteredLanguages.map(\.code),
            ["zh-Hans", "zh-Hant"]
        )
        XCTAssertEqual(ModelsLanguageFilter.label(for: ModelsLanguageFilter.allLanguagesID), "All Languages")
    }

    func testModelsLanguageFilterUsesSupportedLanguageCodes() {
        XCTAssertTrue(
            ModelsLanguageFilter.supports(
                languageCode: "fr",
                supportedLanguageCodes: ["en", "fr"]
            )
        )
        XCTAssertFalse(
            ModelsLanguageFilter.supports(
                languageCode: "de",
                supportedLanguageCodes: ["en", "fr"]
            )
        )
        XCTAssertTrue(
            ModelsLanguageFilter.supports(
                languageCode: ModelsLanguageFilter.allLanguagesID,
                supportedLanguageCodes: []
            )
        )
    }

    func testModelsListSectionsSplitDownloadedDownloadingAndAvailableModels() {
        var settings = AppSettings.defaults
        settings.selectTranscriptionModel(id: "turbo")

        let sections = ModelsListSections.make(
            settings: settings,
            localStorageStates: [
                "base": LocalModelStorageState(
                    modelID: "base",
                    isDownloaded: true,
                    byteCount: 12,
                    directories: []
                )
            ],
            localDownloadStates: [
                "small": LocalModelDownloadState(modelID: "small")
            ],
            languageFilter: ModelsLanguageFilter.allLanguagesID
        )

        XCTAssertEqual(sections.yourModels.first, .local("turbo"))
        XCTAssertTrue(sections.yourModels.contains(.local("base")))
        XCTAssertTrue(sections.yourModels.contains(.local("small")))
        XCTAssertTrue(sections.yourModels.contains(.api(TranscriptionAPIProvider.mistralVoxtralModelID)))
        XCTAssertTrue(sections.availableModels.contains(.local("tiny")))
        XCTAssertTrue(sections.availableModels.contains(.local("large")))
        XCTAssertTrue(sections.availableModels.contains(.appleSpeech))
        XCTAssertFalse(sections.availableModels.contains(.local("turbo")))
        XCTAssertFalse(sections.availableModels.contains(.local("base")))
        XCTAssertFalse(sections.availableModels.contains(.local("small")))
    }

    func testModelsListSectionsDoesNotShowUnknownSelectedModel() {
        var settings = AppSettings.defaults
        settings.selectedModel = "my-custom-model"

        let unfilteredSections = ModelsListSections.make(
            settings: settings,
            localStorageStates: [:],
            localDownloadStates: [:],
            languageFilter: ModelsLanguageFilter.allLanguagesID
        )
        let filteredSections = ModelsListSections.make(
            settings: settings,
            localStorageStates: [:],
            localDownloadStates: [:],
            languageFilter: "fr"
        )

        XCTAssertFalse(unfilteredSections.yourModels.contains { $0.id.contains("my-custom-model") })
        XCTAssertFalse(filteredSections.yourModels.contains { $0.id.contains("my-custom-model") })
    }

    func testDownloadStateFormatsProgress() {
        let state = LocalModelDownloadState(modelID: "turbo", fractionCompleted: 0.424)

        XCTAssertEqual(state.percentComplete, 42)
        XCTAssertEqual(state.statusLabel, "Downloading 42%")
    }

    func testDownloadStateClampsProgress() {
        let underflowState = LocalModelDownloadState(modelID: "tiny", fractionCompleted: -1)
        let overflowState = LocalModelDownloadState(modelID: "large", fractionCompleted: 2)

        XCTAssertEqual(underflowState.percentComplete, 0)
        XCTAssertEqual(underflowState.statusLabel, "Downloading 0%")
        XCTAssertEqual(overflowState.percentComplete, 100)
        XCTAssertEqual(overflowState.statusLabel, "Downloading 100%")
    }

    func testDownloadStateCanReadFoundationProgress() {
        let progress = Progress(totalUnitCount: 100)
        progress.completedUnitCount = 25

        let state = LocalModelDownloadState(modelID: "base", progress: progress)

        XCTAssertEqual(state.percentComplete, 25)
        XCTAssertEqual(state.statusLabel, "Downloading 25%")
    }

    func testDownloadStateShowsCancelling() {
        let state = LocalModelDownloadState(modelID: "small", fractionCompleted: 0.5, isCancelling: true)

        XCTAssertEqual(state.percentComplete, 50)
        XCTAssertEqual(state.statusLabel, "Canceling")
    }

    func testLocalModelRuntimePresentationPrioritizesDownloadAndLoadedStates() {
        let downloadedStorage = LocalModelStorageState(
            modelID: "turbo",
            isDownloaded: true,
            byteCount: 12,
            directories: []
        )
        let missingStorage = LocalModelStorageState(
            modelID: "turbo",
            isDownloaded: false,
            byteCount: 0,
            directories: []
        )

        XCTAssertEqual(
            LocalModelRuntimePresentation.status(
                downloadState: LocalModelDownloadState(modelID: "turbo", fractionCompleted: 0.42),
                runtimeState: LocalModelRuntimeState(modelID: "turbo", isLoaded: true),
                isActive: true,
                storageState: downloadedStorage
            ),
            "Downloading 42%"
        )
        XCTAssertEqual(
            LocalModelRuntimePresentation.status(
                downloadState: nil,
                runtimeState: LocalModelRuntimeState(modelID: "turbo", isLoaded: true),
                isActive: true,
                storageState: downloadedStorage
            ),
            "Loaded"
        )
        XCTAssertEqual(
            LocalModelRuntimePresentation.status(
                downloadState: nil,
                runtimeState: nil,
                isActive: true,
                storageState: downloadedStorage
            ),
            "Active"
        )
        XCTAssertEqual(
            LocalModelRuntimePresentation.status(
                downloadState: nil,
                runtimeState: nil,
                isActive: false,
                storageState: downloadedStorage
            ),
            "Downloaded"
        )
        XCTAssertEqual(
            LocalModelRuntimePresentation.status(
                downloadState: nil,
                runtimeState: nil,
                isActive: false,
                storageState: missingStorage
            ),
            "Download"
        )
        XCTAssertEqual(
            LocalModelRuntimePresentation.status(
                downloadState: nil,
                runtimeState: nil,
                isActive: true,
                storageState: missingStorage
            ),
            "Download"
        )
    }

    func testLocalStorageDoesNotTreatPartialWhisperKitDirectoryAsDownloaded() throws {
        let paths = try makeTemporaryStoragePaths()
        let repositoryDirectory = paths.modelsDirectory
            .appendingPathComponent("whisperkit-coreml", isDirectory: true)
            .appendingPathComponent("models", isDirectory: true)
            .appendingPathComponent("argmaxinc", isDirectory: true)
            .appendingPathComponent("whisperkit-coreml", isDirectory: true)
        let partialDirectory = repositoryDirectory
            .appendingPathComponent("openai_whisper-large-v3-v20240930_626MB", isDirectory: true)
        try FileManager.default.createDirectory(
            at: partialDirectory.appendingPathComponent("AudioEncoder.mlmodelc", isDirectory: true),
            withIntermediateDirectories: true
        )
        try Data("partial".utf8).write(
            to: partialDirectory
                .appendingPathComponent("AudioEncoder.mlmodelc", isDirectory: true)
                .appendingPathComponent("coremldata.bin")
        )

        let model = try XCTUnwrap(LocalTranscriptionModel.model(for: "large"))
        let state = LocalModelStorageService.state(for: model, paths: paths)

        XCTAssertFalse(state.isDownloaded)
        XCTAssertNil(LocalModelStorageService.downloadedModelDirectory(for: model, paths: paths))
        XCTAssertEqual(state.directories.map(\.lastPathComponent), ["openai_whisper-large-v3-v20240930_626MB"])
    }

    func testLocalStorageTreatsCompleteWhisperKitDirectoryAsDownloaded() throws {
        let paths = try makeTemporaryStoragePaths()
        let repositoryDirectory = paths.modelsDirectory
            .appendingPathComponent("whisperkit-coreml", isDirectory: true)
            .appendingPathComponent("models", isDirectory: true)
            .appendingPathComponent("argmaxinc", isDirectory: true)
            .appendingPathComponent("whisperkit-coreml", isDirectory: true)
        let completeDirectory = repositoryDirectory
            .appendingPathComponent("openai_whisper-tiny", isDirectory: true)
        try writeCompleteCompiledModelComponent(named: "AudioEncoder.mlmodelc", in: completeDirectory)
        try writeCompleteCompiledModelComponent(named: "MelSpectrogram.mlmodelc", in: completeDirectory)
        try writeCompleteCompiledModelComponent(named: "TextDecoder.mlmodelc", in: completeDirectory)
        try Data("{}".utf8).write(to: completeDirectory.appendingPathComponent("config.json"))
        try Data("{}".utf8).write(to: completeDirectory.appendingPathComponent("generation_config.json"))

        let model = try XCTUnwrap(LocalTranscriptionModel.model(for: "tiny"))
        let state = LocalModelStorageService.state(for: model, paths: paths)

        XCTAssertTrue(state.isDownloaded)
        XCTAssertEqual(
            LocalModelStorageService.downloadedModelDirectory(for: model, paths: paths)?.lastPathComponent,
            "openai_whisper-tiny"
        )
        XCTAssertEqual(state.directories.map(\.lastPathComponent), ["openai_whisper-tiny"])
    }

    func testDeletionConfirmationForInactiveModel() throws {
        let model = try XCTUnwrap(LocalTranscriptionModel.model(for: "base"))

        let confirmation = LocalModelDeletionConfirmation.request(for: model, isActive: false)

        XCTAssertEqual(confirmation.title, "Delete Model")
        XCTAssertEqual(
            confirmation.message,
            "Are you sure you want to delete Whisper Base? You will need to download it again to use it."
        )
        XCTAssertEqual(confirmation.destructiveButtonTitle, "Delete")
        XCTAssertEqual(confirmation.cancelButtonTitle, "Cancel")
    }

    func testDeletionConfirmationWarnsForActiveModel() throws {
        let model = try XCTUnwrap(LocalTranscriptionModel.model(for: "turbo"))

        let confirmation = LocalModelDeletionConfirmation.request(for: model, isActive: true)

        XCTAssertEqual(
            confirmation.message,
            "Whisper Turbo is your active model. Deleting it will stop transcriptions until you select a new model. Are you sure?"
        )
    }

    private func makeTemporaryStoragePaths() throws -> AppPaths {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("MurmurNativeModelStorage-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        return AppPaths(
            appDataDirectory: root,
            recordingsDirectory: root.appendingPathComponent("recordings", isDirectory: true),
            modelsDirectory: root.appendingPathComponent("models", isDirectory: true),
            logsDirectory: root.appendingPathComponent("logs", isDirectory: true)
        )
    }

    private func writeCompleteCompiledModelComponent(named name: String, in modelDirectory: URL) throws {
        let componentDirectory = modelDirectory.appendingPathComponent(name, isDirectory: true)
        try FileManager.default.createDirectory(
            at: componentDirectory.appendingPathComponent("weights", isDirectory: true),
            withIntermediateDirectories: true
        )
        try Data("{}".utf8).write(to: componentDirectory.appendingPathComponent("metadata.json"))
        try Data("mil".utf8).write(to: componentDirectory.appendingPathComponent("model.mil"))
        try Data("coreml".utf8).write(to: componentDirectory.appendingPathComponent("coremldata.bin"))
        try Data("weights".utf8).write(to: componentDirectory.appendingPathComponent("weights/weight.bin"))
    }
}
