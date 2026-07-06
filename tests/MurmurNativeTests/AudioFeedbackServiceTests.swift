import Foundation
@testable import MurmurNative
import XCTest

final class AudioFeedbackServiceTests: XCTestCase {
    func testSoundFileNamesMatchExistingMurmurResources() {
        XCTAssertEqual(
            AudioFeedbackService.soundFileName(for: .start, theme: .marimba),
            "marimba_start.wav"
        )
        XCTAssertEqual(
            AudioFeedbackService.soundFileName(for: .stop, theme: .pop),
            "pop_stop.wav"
        )
        XCTAssertEqual(
            AudioFeedbackService.soundFileName(for: .start, theme: .custom),
            "custom_start.wav"
        )
    }

    func testCustomSoundUsesSharedAppDataDirectory() {
        let appDataDirectory = URL(fileURLWithPath: "/tmp/MurmurData", isDirectory: true)
        let paths = AppPaths(
            appDataDirectory: appDataDirectory,
            recordingsDirectory: appDataDirectory.appendingPathComponent("recordings", isDirectory: true),
            modelsDirectory: appDataDirectory.appendingPathComponent("models", isDirectory: true),
            logsDirectory: appDataDirectory.appendingPathComponent("logs", isDirectory: true)
        )
        var settings = AppSettings.defaults
        settings.soundTheme = .custom

        XCTAssertEqual(
            AudioFeedbackService.soundURL(for: .stop, settings: settings, paths: paths),
            appDataDirectory.appendingPathComponent("custom_stop.wav")
        )
    }
}
