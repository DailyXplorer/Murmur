import Foundation
@testable import MurmurNative
import XCTest

final class AudioPlaybackTests: XCTestCase {
    func testPlaybackTimeFormatterMatchesInlinePlayerDisplay() {
        XCTAssertEqual(AudioPlaybackTimeFormatter.formatted(0), "0:00")
        XCTAssertEqual(AudioPlaybackTimeFormatter.formatted(7.9), "0:07")
        XCTAssertEqual(AudioPlaybackTimeFormatter.formatted(65.2), "1:05")
        XCTAssertEqual(AudioPlaybackTimeFormatter.formatted(.infinity), "0:00")
        XCTAssertEqual(AudioPlaybackTimeFormatter.formatted(-3), "0:00")
    }

    func testPlaybackStateProgressIsClamped() {
        XCTAssertEqual(AudioPlaybackState.idle.progress, 0)

        let overrun = AudioPlaybackState(
            entryID: 1,
            isPlaying: true,
            currentTime: 12,
            duration: 10
        )
        XCTAssertEqual(overrun.progress, 1)

        let negative = AudioPlaybackState(
            entryID: 1,
            isPlaying: true,
            currentTime: -2,
            duration: 10
        )
        XCTAssertEqual(negative.progress, 0)
    }
}
