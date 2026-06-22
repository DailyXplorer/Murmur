import Foundation

struct SpeechActivityTrimmer {
    struct Configuration: Equatable {
        var frameDuration: TimeInterval
        var prefillFrameCount: Int
        var hangoverFrameCount: Int
        var onsetFrameCount: Int
        var peakThreshold: Float
        var rootMeanSquareThreshold: Float
        var flatNoiseMaximumPeak: Float
        var flatNoiseMaximumRootMeanSquare: Float
        var flatNoiseMaximumRelativeRootMeanSquareRange: Float
        var flatNoiseMinimumFrameCount: Int
        var adaptiveNoiseMaximumRootMeanSquare: Float
        var adaptiveNoiseMaximumPeak: Float
        var adaptiveNoiseRootMeanSquareMultiplier: Float
        var adaptiveNoisePeakMultiplier: Float
        var adaptiveNoiseMinimumFrameCount: Int

        static let handy = Configuration(
            frameDuration: 0.03,
            prefillFrameCount: 15,
            hangoverFrameCount: 15,
            onsetFrameCount: 2,
            peakThreshold: AudioRecording.audiblePeakThreshold,
            rootMeanSquareThreshold: AudioRecording.audibleRootMeanSquareThreshold,
            flatNoiseMaximumPeak: 0.004,
            flatNoiseMaximumRootMeanSquare: 0.002,
            flatNoiseMaximumRelativeRootMeanSquareRange: 0.12,
            flatNoiseMinimumFrameCount: 8,
            adaptiveNoiseMaximumRootMeanSquare: 0.006,
            adaptiveNoiseMaximumPeak: 0.012,
            adaptiveNoiseRootMeanSquareMultiplier: 3,
            adaptiveNoisePeakMultiplier: 3,
            adaptiveNoiseMinimumFrameCount: 8
        )
    }

    var configuration: Configuration = .handy

    func trim(_ recording: AudioRecording) -> AudioRecording {
        guard recording.sampleRate > 0,
              recording.samples.isEmpty == false,
              recording.hasAudibleSignal,
              configuration.frameDuration > 0
        else {
            return recording
        }

        let frameSize = max(1, Int((recording.sampleRate * configuration.frameDuration).rounded()))
        let analyses = frameAnalyses(for: recording, frameSize: frameSize)
        if isLikelyFlatNoise(analyses) {
            return AudioRecording(
                samples: [],
                sampleRate: recording.sampleRate,
                startedAt: recording.startedAt,
                endedAt: recording.endedAt
            )
        }

        let adaptiveNoiseProfile = adaptiveNoiseProfile(from: analyses)
        var frameBuffer: [Range<Int>] = []
        var outputSamples: [Float] = []
        outputSamples.reserveCapacity(recording.samples.count)

        var hangoverCounter = 0
        var onsetCounter = 0
        var inSpeech = false

        for analysis in analyses {
            frameBuffer.append(analysis.range)
            while frameBuffer.count > configuration.prefillFrameCount + 1 {
                frameBuffer.removeFirst()
            }

            let isVoice = isVoiceFrame(analysis, adaptiveNoiseProfile: adaptiveNoiseProfile)
            switch (inSpeech, isVoice) {
            case (false, true):
                onsetCounter += 1
                if onsetCounter >= configuration.onsetFrameCount {
                    inSpeech = true
                    hangoverCounter = configuration.hangoverFrameCount
                    onsetCounter = 0
                    for bufferedRange in frameBuffer {
                        outputSamples.append(contentsOf: recording.samples[bufferedRange])
                    }
                }

            case (true, true):
                hangoverCounter = configuration.hangoverFrameCount
                outputSamples.append(contentsOf: recording.samples[analysis.range])

            case (true, false):
                if hangoverCounter > 0 {
                    hangoverCounter -= 1
                    outputSamples.append(contentsOf: recording.samples[analysis.range])
                } else {
                    inSpeech = false
                }

            case (false, false):
                onsetCounter = 0
            }
        }

        guard outputSamples.isEmpty == false else {
            return AudioRecording(
                samples: [],
                sampleRate: recording.sampleRate,
                startedAt: recording.startedAt,
                endedAt: recording.endedAt
            )
        }

        guard outputSamples.count != recording.samples.count else {
            return recording
        }

        return AudioRecording(
            samples: outputSamples,
            sampleRate: recording.sampleRate,
            startedAt: recording.startedAt,
            endedAt: recording.endedAt
        )
    }

    private func frameAnalyses(for recording: AudioRecording, frameSize: Int) -> [FrameAnalysis] {
        var analyses: [FrameAnalysis] = []
        analyses.reserveCapacity(max(1, recording.samples.count / max(frameSize, 1)))

        var frameStart = 0
        while frameStart < recording.samples.count {
            let frameEnd = min(frameStart + frameSize, recording.samples.count)
            let range = frameStart..<frameEnd
            analyses.append(
                FrameAnalysis(
                    range: range,
                    metrics: frameMetrics(recording.samples[range])
                )
            )
            frameStart = frameEnd
        }

        return analyses
    }

    private func isLikelyFlatNoise(_ analyses: [FrameAnalysis]) -> Bool {
        guard configuration.flatNoiseMinimumFrameCount > 0,
              analyses.isEmpty == false
        else {
            return false
        }

        var maxPeak = Float(0)
        var minRootMeanSquare = Float.greatestFiniteMagnitude
        var maxRootMeanSquare = Float(0)

        for analysis in analyses {
            let metrics = analysis.metrics
            maxPeak = max(maxPeak, metrics.peak)
            minRootMeanSquare = min(minRootMeanSquare, metrics.rootMeanSquare)
            maxRootMeanSquare = max(maxRootMeanSquare, metrics.rootMeanSquare)
        }

        guard analyses.count >= configuration.flatNoiseMinimumFrameCount,
              maxPeak >= configuration.peakThreshold,
              maxPeak <= configuration.flatNoiseMaximumPeak,
              maxRootMeanSquare >= configuration.rootMeanSquareThreshold,
              maxRootMeanSquare <= configuration.flatNoiseMaximumRootMeanSquare
        else {
            return false
        }

        let relativeRange = (maxRootMeanSquare - minRootMeanSquare) / max(maxRootMeanSquare, .leastNonzeroMagnitude)
        return relativeRange <= configuration.flatNoiseMaximumRelativeRootMeanSquareRange
    }

    private func adaptiveNoiseProfile(from analyses: [FrameAnalysis]) -> AdaptiveNoiseProfile? {
        guard configuration.adaptiveNoiseMinimumFrameCount > 0,
              analyses.count >= configuration.adaptiveNoiseMinimumFrameCount else {
            return nil
        }

        let rootMeanSquares = analyses.map(\.metrics.rootMeanSquare).sorted()
        let peaks = analyses.map(\.metrics.peak).sorted()
        let percentileIndex = max(0, min(rootMeanSquares.count - 1, rootMeanSquares.count / 5))
        let rootMeanSquareFloor = rootMeanSquares[percentileIndex]
        let peakFloor = peaks[percentileIndex]

        guard rootMeanSquareFloor >= configuration.rootMeanSquareThreshold,
              rootMeanSquareFloor <= configuration.adaptiveNoiseMaximumRootMeanSquare,
              peakFloor >= configuration.peakThreshold,
              peakFloor <= configuration.adaptiveNoiseMaximumPeak else {
            return nil
        }

        return AdaptiveNoiseProfile(
            peakThreshold: max(
                configuration.peakThreshold,
                peakFloor * configuration.adaptiveNoisePeakMultiplier
            ),
            rootMeanSquareThreshold: max(
                configuration.rootMeanSquareThreshold,
                rootMeanSquareFloor * configuration.adaptiveNoiseRootMeanSquareMultiplier
            )
        )
    }

    private func isVoiceFrame(_ analysis: FrameAnalysis, adaptiveNoiseProfile: AdaptiveNoiseProfile?) -> Bool {
        guard analysis.range.isEmpty == false else {
            return false
        }

        let metrics = analysis.metrics
        let peakThreshold = adaptiveNoiseProfile?.peakThreshold ?? configuration.peakThreshold
        let rootMeanSquareThreshold = adaptiveNoiseProfile?.rootMeanSquareThreshold ?? configuration.rootMeanSquareThreshold
        return metrics.peak >= peakThreshold ||
            metrics.rootMeanSquare >= rootMeanSquareThreshold
    }

    private func frameMetrics(_ frame: ArraySlice<Float>) -> FrameMetrics {
        var peak = Float(0)
        var sumOfSquares = Double(0)
        for sample in frame {
            peak = max(peak, abs(sample))
            sumOfSquares += Double(sample * sample)
        }

        let rootMeanSquare = Float(sqrt(sumOfSquares / Double(frame.count)))
        return FrameMetrics(peak: peak, rootMeanSquare: rootMeanSquare)
    }

    private struct FrameAnalysis {
        let range: Range<Int>
        let metrics: FrameMetrics
    }

    private struct FrameMetrics {
        let peak: Float
        let rootMeanSquare: Float
    }

    private struct AdaptiveNoiseProfile {
        let peakThreshold: Float
        let rootMeanSquareThreshold: Float
    }
}
