import AVFoundation
import Foundation

final class AudioSampleAccumulator: @unchecked Sendable {
    static let defaultOutputSampleRate: Double = 16_000

    private let lock = NSLock()
    private var samples: [Float] = []
    private let startedAt: Date
    private let outputSampleRate: Double

    let sampleRate: Double

    init(
        sampleRate: Double,
        outputSampleRate: Double = AudioSampleAccumulator.defaultOutputSampleRate,
        startedAt: Date = Date()
    ) {
        self.sampleRate = sampleRate
        self.outputSampleRate = outputSampleRate
        self.startedAt = startedAt
    }

    func append(_ buffer: AVAudioPCMBuffer) -> Float {
        guard let channelData = buffer.floatChannelData else {
            return 0
        }

        let channelCount = Int(buffer.format.channelCount)
        let frameCount = Int(buffer.frameLength)
        guard channelCount > 0, frameCount > 0 else {
            return 0
        }

        var monoSamples: [Float] = []
        monoSamples.reserveCapacity(frameCount)

        var sumOfSquares: Double = 0
        for frame in 0..<frameCount {
            var mixedSample: Float = 0
            for channel in 0..<channelCount {
                mixedSample += channelData[channel][frame]
            }
            mixedSample /= Float(channelCount)
            monoSamples.append(mixedSample)
            sumOfSquares += Double(mixedSample * mixedSample)
        }

        lock.lock()
        samples.append(contentsOf: monoSamples)
        lock.unlock()

        let rootMeanSquare = sqrt(sumOfSquares / Double(frameCount))
        return min(1, Float(rootMeanSquare * 4))
    }

    func recording(endedAt: Date = Date()) -> AudioRecording {
        lock.lock()
        let capturedSamples = samples
        lock.unlock()
        let outputSamples = Self.resample(
            capturedSamples,
            from: sampleRate,
            to: outputSampleRate
        )

        return AudioRecording(
            samples: outputSamples,
            sampleRate: outputSampleRate,
            startedAt: startedAt,
            endedAt: endedAt
        )
    }

    private static func resample(_ samples: [Float], from sourceSampleRate: Double, to targetSampleRate: Double) -> [Float] {
        guard samples.isEmpty == false,
              sourceSampleRate > 0,
              targetSampleRate > 0
        else {
            return samples
        }

        guard abs(sourceSampleRate - targetSampleRate) > 0.5 else {
            return samples
        }

        let outputCount = max(1, Int((Double(samples.count) * targetSampleRate / sourceSampleRate).rounded()))
        var output: [Float] = []
        output.reserveCapacity(outputCount)

        for index in 0..<outputCount {
            let sourcePosition = Double(index) * sourceSampleRate / targetSampleRate
            let lowerIndex = Int(sourcePosition.rounded(.down))
            let upperIndex = min(lowerIndex + 1, samples.count - 1)
            let fraction = Float(sourcePosition - Double(lowerIndex))
            let lowerSample = samples[min(lowerIndex, samples.count - 1)]
            let upperSample = samples[upperIndex]
            output.append(lowerSample + (upperSample - lowerSample) * fraction)
        }

        return output
    }
}
