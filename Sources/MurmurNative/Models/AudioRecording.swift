import Foundation

struct AudioRecording: Equatable {
    static let minimumTranscriptionSampleCount = 16_000
    static let shortRecordingPaddingSampleCount = 20_000
    static let audiblePeakThreshold: Float = 0.001
    static let audibleRootMeanSquareThreshold: Float = 0.0003
    static let activityFrameDuration: TimeInterval = SpeechActivityTrimmer.Configuration.murmur.frameDuration
    static let activityLeadingPaddingDuration: TimeInterval = SpeechActivityTrimmer.Configuration.murmur.frameDuration *
        Double(SpeechActivityTrimmer.Configuration.murmur.prefillFrameCount)
    static let activityTrailingPaddingDuration: TimeInterval = SpeechActivityTrimmer.Configuration.murmur.frameDuration *
        Double(SpeechActivityTrimmer.Configuration.murmur.hangoverFrameCount)

    let samples: [Float]
    let sampleRate: Double
    let startedAt: Date
    let endedAt: Date

    var duration: TimeInterval {
        guard sampleRate > 0 else {
            return 0
        }

        return Double(samples.count) / sampleRate
    }

    var isEmpty: Bool {
        samples.isEmpty
    }

    var peakAmplitude: Float {
        samples.reduce(Float(0)) { max($0, abs($1)) }
    }

    var rootMeanSquare: Float {
        guard samples.isEmpty == false else {
            return 0
        }

        let sumOfSquares = samples.reduce(Double(0)) { partial, sample in
            partial + Double(sample * sample)
        }
        return Float(sqrt(sumOfSquares / Double(samples.count)))
    }

    var hasAudibleSignal: Bool {
        peakAmplitude >= Self.audiblePeakThreshold ||
            rootMeanSquare >= Self.audibleRootMeanSquareThreshold
    }

    func trimmedAroundAudibleSignal(
        frameDuration: TimeInterval = Self.activityFrameDuration,
        leadingPadding: TimeInterval = Self.activityLeadingPaddingDuration,
        trailingPadding: TimeInterval = Self.activityTrailingPaddingDuration
    ) -> AudioRecording {
        guard frameDuration > 0 else {
            return self
        }

        return SpeechActivityTrimmer(
            configuration: SpeechActivityTrimmer.Configuration(
                frameDuration: frameDuration,
                prefillFrameCount: max(0, Int((leadingPadding / frameDuration).rounded())),
                hangoverFrameCount: max(0, Int((trailingPadding / frameDuration).rounded())),
                onsetFrameCount: SpeechActivityTrimmer.Configuration.murmur.onsetFrameCount,
                peakThreshold: Self.audiblePeakThreshold,
                rootMeanSquareThreshold: Self.audibleRootMeanSquareThreshold,
                flatNoiseMaximumPeak: SpeechActivityTrimmer.Configuration.murmur.flatNoiseMaximumPeak,
                flatNoiseMaximumRootMeanSquare: SpeechActivityTrimmer.Configuration.murmur.flatNoiseMaximumRootMeanSquare,
                flatNoiseMaximumRelativeRootMeanSquareRange: SpeechActivityTrimmer.Configuration.murmur.flatNoiseMaximumRelativeRootMeanSquareRange,
                flatNoiseMinimumFrameCount: SpeechActivityTrimmer.Configuration.murmur.flatNoiseMinimumFrameCount,
                adaptiveNoiseMaximumRootMeanSquare: SpeechActivityTrimmer.Configuration.murmur.adaptiveNoiseMaximumRootMeanSquare,
                adaptiveNoiseMaximumPeak: SpeechActivityTrimmer.Configuration.murmur.adaptiveNoiseMaximumPeak,
                adaptiveNoiseRootMeanSquareMultiplier: SpeechActivityTrimmer.Configuration.murmur.adaptiveNoiseRootMeanSquareMultiplier,
                adaptiveNoisePeakMultiplier: SpeechActivityTrimmer.Configuration.murmur.adaptiveNoisePeakMultiplier,
                adaptiveNoiseMinimumFrameCount: SpeechActivityTrimmer.Configuration.murmur.adaptiveNoiseMinimumFrameCount
            )
        ).trim(self)
    }

    func preparedForTranscriptionInput() -> AudioRecording {
        trimmedAroundAudibleSignal().paddedForShortTranscriptionInput()
    }

    func paddedForShortTranscriptionInput() -> AudioRecording {
        guard sampleRate > 0,
              samples.isEmpty == false,
              samples.count < Self.minimumTranscriptionSampleCount
        else {
            return self
        }

        var paddedSamples = samples
        paddedSamples.append(
            contentsOf: repeatElement(0, count: Self.shortRecordingPaddingSampleCount - samples.count)
        )
        return AudioRecording(
            samples: paddedSamples,
            sampleRate: sampleRate,
            startedAt: startedAt,
            endedAt: endedAt
        )
    }

}
