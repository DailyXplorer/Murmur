import AVFoundation
import XCTest
@testable import HandyNative

final class AudioPipelineTests: XCTestCase {
    func testAccumulatorMixesChannelsToMono() throws {
        let format = try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 2))
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 2))
        buffer.frameLength = 2
        buffer.floatChannelData?[0][0] = 1
        buffer.floatChannelData?[1][0] = -1
        buffer.floatChannelData?[0][1] = 0.5
        buffer.floatChannelData?[1][1] = 0.5

        let accumulator = AudioSampleAccumulator(sampleRate: 48_000, outputSampleRate: 48_000)
        let level = accumulator.append(buffer)
        let recording = accumulator.recording()

        XCTAssertGreaterThan(level, 0)
        XCTAssertEqual(recording.sampleRate, 48_000)
        XCTAssertEqual(recording.samples.count, 2)
        XCTAssertEqual(recording.samples[0], 0, accuracy: 0.0001)
        XCTAssertEqual(recording.samples[1], 0.5, accuracy: 0.0001)
    }

    func testAccumulatorResamplesRecordingToWhisperSampleRateByDefault() throws {
        let frameCount: AVAudioFrameCount = 480
        let format = try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1))
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frameCount))
        buffer.frameLength = frameCount
        for index in 0..<Int(frameCount) {
            buffer.floatChannelData?[0][index] = 0.25
        }

        let accumulator = AudioSampleAccumulator(sampleRate: 48_000)
        _ = accumulator.append(buffer)
        let recording = accumulator.recording()

        XCTAssertEqual(recording.sampleRate, 16_000)
        XCTAssertEqual(recording.samples.count, 160)
        XCTAssertEqual(recording.duration, 0.01, accuracy: 0.0001)
        XCTAssertEqual(recording.samples.first ?? 0, 0.25, accuracy: 0.0001)
        XCTAssertEqual(recording.samples.last ?? 0, 0.25, accuracy: 0.0001)
    }

    func testAudioRecordingDetectsAudibleSignal() {
        let silentRecording = AudioRecording(
            samples: Array(repeating: 0, count: 16_000),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 1)
        )
        let quietNoiseRecording = AudioRecording(
            samples: Array(repeating: 0.00001, count: 16_000),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 1)
        )
        let audibleRecording = AudioRecording(
            samples: Array(repeating: 0.01, count: 16_000),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 1)
        )

        XCTAssertFalse(silentRecording.hasAudibleSignal)
        XCTAssertFalse(quietNoiseRecording.hasAudibleSignal)
        XCTAssertTrue(audibleRecording.hasAudibleSignal)
        XCTAssertEqual(audibleRecording.peakAmplitude, 0.01, accuracy: 0.0001)
        XCTAssertEqual(audibleRecording.rootMeanSquare, 0.01, accuracy: 0.0001)
    }

    func testAudioRecordingRejectsFlatLowLevelNoiseBeforeTranscription() {
        let recording = AudioRecording(
            samples: Array(repeating: 0.0012, count: 32_000),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 2)
        )

        XCTAssertTrue(recording.hasAudibleSignal)
        XCTAssertTrue(recording.trimmedAroundAudibleSignal().isEmpty)
        XCTAssertTrue(recording.preparedForTranscriptionInput().isEmpty)
    }

    func testAudioRecordingRejectsFluctuatingLowLevelNoiseBeforeTranscription() {
        var samples: [Float] = []
        for frameIndex in 0..<64 {
            let amplitude: Float = frameIndex.isMultiple(of: 2) ? 0.0012 : 0.0021
            samples.append(contentsOf: Array(repeating: amplitude, count: 480))
        }
        let recording = AudioRecording(
            samples: samples,
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 1.92)
        )

        XCTAssertTrue(recording.hasAudibleSignal)
        XCTAssertTrue(recording.trimmedAroundAudibleSignal().isEmpty)
        XCTAssertTrue(recording.preparedForTranscriptionInput().isEmpty)
    }

    func testAdaptiveNoiseFloorKeepsSpeechAboveLowBackgroundNoise() {
        let leadingNoise = Array(repeating: Float(0.0015), count: 16_000)
        let speech = Array(repeating: Float(0.012), count: 8_000)
        let trailingNoise = Array(repeating: Float(0.0015), count: 16_000)
        let recording = AudioRecording(
            samples: leadingNoise + speech + trailingNoise,
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 2.5)
        )

        let trimmed = recording.trimmedAroundAudibleSignal()

        XCTAssertFalse(trimmed.isEmpty)
        XCTAssertLessThan(trimmed.samples.count, recording.samples.count)
        XCTAssertTrue(trimmed.samples.contains { abs($0 - 0.012) < 0.0001 })
        XCTAssertEqual(trimmed.samples.first ?? 0, 0.0015, accuracy: 0.0001)
    }

    func testFlatNoiseGuardKeepsContinuousSpeechLevelInput() {
        let recording = AudioRecording(
            samples: Array(repeating: 0.02, count: 32_000),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 2)
        )

        let trimmed = recording.trimmedAroundAudibleSignal()

        XCTAssertFalse(trimmed.isEmpty)
        XCTAssertEqual(trimmed.samples.count, recording.samples.count)
        XCTAssertEqual(trimmed.samples.first ?? 0, 0.02, accuracy: 0.0001)
    }

    func testAudioRecordingTrimsSpeechActivityWithHandyPrefillAndHangover() {
        let leadingSilence = Array(repeating: Float(0), count: 16_000)
        let speech = Array(repeating: Float(0.02), count: 8_000)
        let trailingSilence = Array(repeating: Float(0), count: 16_000)
        let recording = AudioRecording(
            samples: leadingSilence + speech + trailingSilence,
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 2.5)
        )

        let trimmed = recording.trimmedAroundAudibleSignal()

        XCTAssertEqual(trimmed.sampleRate, 16_000)
        XCTAssertEqual(trimmed.startedAt, recording.startedAt)
        XCTAssertEqual(trimmed.endedAt, recording.endedAt)
        XCTAssertEqual(trimmed.samples.count, 22_080)
        XCTAssertEqual(trimmed.samples[0], 0, accuracy: 0.0001)
        XCTAssertEqual(trimmed.samples[6_879], 0, accuracy: 0.0001)
        XCTAssertEqual(trimmed.samples[6_880], 0.02, accuracy: 0.0001)
        XCTAssertEqual(trimmed.samples[14_879], 0.02, accuracy: 0.0001)
        XCTAssertEqual(trimmed.samples[14_880], 0, accuracy: 0.0001)
        XCTAssertEqual(trimmed.samples[22_079], 0, accuracy: 0.0001)
    }

    func testAudioRecordingRemovesLongInternalSilenceBetweenSpeechBursts() {
        let firstSpeech = Array(repeating: Float(0.02), count: 2_880)
        let secondSpeech = Array(repeating: Float(0.03), count: 2_880)
        let recording = AudioRecording(
            samples: Array(repeating: 0, count: 16_000) +
                firstSpeech +
                Array(repeating: 0, count: 16_000) +
                secondSpeech +
                Array(repeating: 0, count: 16_000),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 3.36)
        )

        let trimmed = recording.trimmedAroundAudibleSignal()

        XCTAssertLessThan(trimmed.samples.count, recording.samples.count - 4_800)
        XCTAssertTrue(trimmed.samples.contains { abs($0 - 0.02) < 0.0001 })
        XCTAssertTrue(trimmed.samples.contains { abs($0 - 0.03) < 0.0001 })
    }

    func testAudioRecordingDropsIsolatedAudibleFrameBeforeOnset() {
        let recording = AudioRecording(
            samples: Array(repeating: 0, count: 9_600) +
                Array(repeating: Float(0.02), count: 480) +
                Array(repeating: 0, count: 9_600),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 1.23)
        )

        XCTAssertTrue(recording.hasAudibleSignal)
        XCTAssertTrue(recording.trimmedAroundAudibleSignal().isEmpty)
    }

    func testAudioRecordingDoesNotTrimSilentInput() {
        let recording = AudioRecording(
            samples: Array(repeating: 0, count: 16_000),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 1)
        )

        XCTAssertEqual(recording.trimmedAroundAudibleSignal(), recording)
    }

    func testShortRecordingPadsForTranscriptionInput() {
        let recording = AudioRecording(
            samples: Array(repeating: 0.25, count: 4_800),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 0.3)
        )

        let padded = recording.paddedForShortTranscriptionInput()

        XCTAssertEqual(padded.sampleRate, 16_000)
        XCTAssertEqual(padded.samples.count, 20_000)
        XCTAssertEqual(padded.duration, 1.25, accuracy: 0.0001)
        XCTAssertEqual(padded.samples[0], 0.25, accuracy: 0.0001)
        XCTAssertEqual(padded.samples[4_799], 0.25, accuracy: 0.0001)
        XCTAssertEqual(padded.samples[4_800], 0, accuracy: 0.0001)
    }

    func testLongRecordingDoesNotPadForTranscriptionInput() {
        let recording = AudioRecording(
            samples: Array(repeating: 0.25, count: 16_000),
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 1)
        )

        XCTAssertEqual(recording.paddedForShortTranscriptionInput(), recording)
    }

    func testWAVWriterCreatesPCM16File() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let fileURL = directory.appendingPathComponent("sample.wav")
        let recording = AudioRecording(
            samples: [-1, 0, 0.5, 1],
            sampleRate: 16_000,
            startedAt: Date(timeIntervalSince1970: 0),
            endedAt: Date(timeIntervalSince1970: 1)
        )

        try WAVFileWriter.write(recording, to: fileURL)
        let data = try Data(contentsOf: fileURL)

        XCTAssertEqual(String(data: data[0..<4], encoding: .ascii), "RIFF")
        XCTAssertEqual(String(data: data[8..<12], encoding: .ascii), "WAVE")
        XCTAssertEqual(String(data: data[12..<16], encoding: .ascii), "fmt ")
        XCTAssertEqual(String(data: data[36..<40], encoding: .ascii), "data")
        XCTAssertEqual(data.count, 44 + 8)

        try? FileManager.default.removeItem(at: directory)
    }
}
