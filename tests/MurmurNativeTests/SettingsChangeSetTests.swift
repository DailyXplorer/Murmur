import XCTest
@testable import MurmurNative

final class SettingsChangeSetTests: XCTestCase {
    func testNoOpUpdateFlagsNothing() {
        let old = AppSettings.defaults

        let changes = SettingsChangeSet(old: old, new: old)

        XCTAssertFalse(changes.shortcutsChanged)
        XCTAssertFalse(changes.transcriptionCredentialsChanged)
        XCTAssertFalse(changes.postProcessCredentialsChanged)
        XCTAssertFalse(changes.historyRetentionChanged)
    }

    func testThemeOnlyChangeFlagsNothing() {
        let old = AppSettings.defaults
        var new = old
        new.appTheme = old.appTheme == .pink ? .blue : .pink

        let changes = SettingsChangeSet(old: old, new: new)

        XCTAssertFalse(changes.shortcutsChanged)
        XCTAssertFalse(changes.transcriptionCredentialsChanged)
        XCTAssertFalse(changes.postProcessCredentialsChanged)
        XCTAssertFalse(changes.historyRetentionChanged)
    }

    func testTranscribeBindingChangeFlagsOnlyShortcuts() {
        let old = AppSettings.defaults
        var new = old
        new.shortcutBindings[ShortcutBinding.transcribeID]?.currentBinding = "cmd+shift+t"

        let changes = SettingsChangeSet(old: old, new: new)

        XCTAssertTrue(changes.shortcutsChanged)
        XCTAssertFalse(changes.transcriptionCredentialsChanged)
        XCTAssertFalse(changes.postProcessCredentialsChanged)
        XCTAssertFalse(changes.historyRetentionChanged)
    }

    func testPostProcessEnabledToggleFlagsOnlyShortcuts() {
        let old = AppSettings.defaults
        var new = old
        new.postProcessEnabled.toggle()

        let changes = SettingsChangeSet(old: old, new: new)

        XCTAssertTrue(changes.shortcutsChanged)
        XCTAssertFalse(changes.transcriptionCredentialsChanged)
        XCTAssertFalse(changes.postProcessCredentialsChanged)
        XCTAssertFalse(changes.historyRetentionChanged)
    }

    func testHistoryLimitChangeFlagsOnlyHistoryRetention() {
        let old = AppSettings.defaults
        var new = old
        new.historyLimit = old.historyLimit + 5

        let changes = SettingsChangeSet(old: old, new: new)

        XCTAssertFalse(changes.shortcutsChanged)
        XCTAssertFalse(changes.transcriptionCredentialsChanged)
        XCTAssertFalse(changes.postProcessCredentialsChanged)
        XCTAssertTrue(changes.historyRetentionChanged)
    }

    func testRecordingRetentionPeriodChangeFlagsOnlyHistoryRetention() {
        let old = AppSettings.defaults
        var new = old
        new.recordingRetentionPeriod = old.recordingRetentionPeriod == .days3 ? .preserveLimit : .days3

        let changes = SettingsChangeSet(old: old, new: new)

        XCTAssertFalse(changes.shortcutsChanged)
        XCTAssertFalse(changes.transcriptionCredentialsChanged)
        XCTAssertFalse(changes.postProcessCredentialsChanged)
        XCTAssertTrue(changes.historyRetentionChanged)
    }

    func testTranscriptionProviderBaseURLEditFlagsOnlyTranscriptionCredentials() throws {
        let old = AppSettings.defaults
        var new = old
        let index = try XCTUnwrap(new.transcriptionAPIProviders.firstIndex { $0.allowBaseURLEdit } ?? new.transcriptionAPIProviders.indices.first)
        new.transcriptionAPIProviders[index].baseURL = "https://example.invalid/v1"

        let changes = SettingsChangeSet(old: old, new: new)

        XCTAssertFalse(changes.shortcutsChanged)
        XCTAssertTrue(changes.transcriptionCredentialsChanged)
        XCTAssertFalse(changes.postProcessCredentialsChanged)
        XCTAssertFalse(changes.historyRetentionChanged)
    }

    func testSelectedModelChangeFlagsOnlyTranscriptionCredentials() {
        let old = AppSettings.defaults
        var new = old
        new.selectedModel = "some-other-model"

        let changes = SettingsChangeSet(old: old, new: new)

        XCTAssertFalse(changes.shortcutsChanged)
        XCTAssertTrue(changes.transcriptionCredentialsChanged)
        XCTAssertFalse(changes.postProcessCredentialsChanged)
        XCTAssertFalse(changes.historyRetentionChanged)
    }

    func testPostProcessProviderChangeFlagsOnlyPostProcessCredentials() {
        let old = AppSettings.defaults
        var new = old
        new.postProcessProviderID = PostProcessProvider.appleIntelligenceProviderID

        let changes = SettingsChangeSet(old: old, new: new)

        XCTAssertFalse(changes.shortcutsChanged)
        XCTAssertFalse(changes.transcriptionCredentialsChanged)
        XCTAssertTrue(changes.postProcessCredentialsChanged)
        XCTAssertFalse(changes.historyRetentionChanged)
    }

    func testPostProcessProviderBaseURLEditFlagsOnlyPostProcessCredentials() throws {
        let old = AppSettings.defaults
        var new = old
        let index = try XCTUnwrap(new.postProcessProviders.indices.first)
        new.postProcessProviders[index].baseURL = "https://example.invalid/v1"

        let changes = SettingsChangeSet(old: old, new: new)

        XCTAssertFalse(changes.shortcutsChanged)
        XCTAssertFalse(changes.transcriptionCredentialsChanged)
        XCTAssertTrue(changes.postProcessCredentialsChanged)
        XCTAssertFalse(changes.historyRetentionChanged)
    }
}
