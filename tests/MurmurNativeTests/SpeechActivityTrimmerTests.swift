import XCTest
@testable import MurmurNative

final class SpeechActivityTrimmerTests: XCTestCase {
    private let trimmer = SpeechActivityTrimmer()

    func testQuietSpeechInFlatNoiseBandIsNotDeleted() {
        let recording = makeRecording(
            samples: sineSamples(amplitude: 0.003, count: 32_000),
            duration: 2
        )

        XCTAssertTrue(recording.hasAudibleSignal)
        let trimmed = trimmer.trim(recording)

        XCTAssertFalse(trimmed.isEmpty)
        XCTAssertEqual(trimmed, recording)
    }

    func testTrimNeverReturnsEmptyForAudibleInput() {
        let constantQuiet = makeRecording(
            samples: Array(repeating: Float(0.002), count: 32_000),
            duration: 2
        )
        let ramp = makeRecording(
            samples: (0..<32_000).map { index in
                0.001 + 0.019 * Float(index) / Float(31_999)
            },
            duration: 2
        )
        let sineBurstBetweenSilence = makeRecording(
            samples: Array(repeating: Float(0), count: 8_000) +
                sineSamples(amplitude: 0.02, count: 8_000) +
                Array(repeating: Float(0), count: 8_000),
            duration: 1.5
        )

        for recording in [constantQuiet, ramp, sineBurstBetweenSilence] {
            XCTAssertTrue(recording.hasAudibleSignal)
            XCTAssertFalse(trimmer.trim(recording).isEmpty)
        }
    }

    func testAdaptiveProfileDisabledWhenFloorNearMax() {
        var samples: [Float] = []
        for frameIndex in 0..<64 {
            let amplitude: Float = frameIndex.isMultiple(of: 2) ? 0.004 : 0.006
            samples.append(contentsOf: Array(repeating: amplitude, count: 480))
        }
        let recording = makeRecording(samples: samples, duration: 1.92)

        XCTAssertTrue(recording.hasAudibleSignal)
        let trimmed = trimmer.trim(recording)

        XCTAssertFalse(trimmed.isEmpty)
        XCTAssertTrue(trimmed.samples.contains { abs($0 - 0.006) < 0.0001 })
    }

    func testLoudSpeechWithQuietGapsStillTrimsGaps() {
        let recording = makeRecording(
            samples: sineSamples(amplitude: 0.3, count: 16_000) +
                Array(repeating: Float(0.0001), count: 32_000) +
                sineSamples(amplitude: 0.3, count: 16_000),
            duration: 4
        )

        XCTAssertTrue(recording.hasAudibleSignal)
        let trimmed = trimmer.trim(recording)

        XCTAssertFalse(trimmed.isEmpty)
        XCTAssertLessThan(trimmed.samples.count, recording.samples.count)
    }

    private func makeRecording(samples: [Float], duration: TimeInterval) -> AudioRecording {
        AudioRecording(
            samples: samples,
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: duration)
        )
    }

    private func sineSamples(
        amplitude: Float,
        frequency: Double = 200,
        count: Int,
        sampleRate: Double = 16_000
    ) -> [Float] {
        (0..<count).map { index in
            amplitude * Float(sin(2 * .pi * frequency * Double(index) / sampleRate))
        }
    }
}
