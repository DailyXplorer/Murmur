import Foundation

enum NativeAudioRecordingSmokeRunner {
    @MainActor
    static func runSynchronouslyAndExit(_ request: NativeAudioRecordingSmokeRequest) -> Never {
        do {
            let output = try record(request)
            FileHandle.standardOutput.writeLine(output)
            exit(0)
        } catch {
            FileHandle.standardError.writeLine(error.localizedDescription)
            exit(1)
        }
    }

    @MainActor
    private static func record(_ request: NativeAudioRecordingSmokeRequest) throws -> String {
        let permission = PermissionService().snapshot().microphone
        guard permission == .granted else {
            throw NativeAudioRecordingSmokeError.microphonePermission(permission)
        }

        let outputURL = URL(fileURLWithPath: request.outputPath)
        let outputDirectory = outputURL.deletingLastPathComponent()
        try FileManager.default.createDirectory(
            at: outputDirectory,
            withIntermediateDirectories: true
        )

        let stats = LockedAudioRecordingStats()
        let audioCaptureService = AudioCaptureService()
        try audioCaptureService.start(selectedMicrophoneName: request.microphoneName) { level in
            stats.observe(level)
        }
        Thread.sleep(forTimeInterval: TimeInterval(request.durationMilliseconds) / 1_000)
        let recording = try audioCaptureService.stop()

        guard recording.isEmpty == false else {
            throw NativeAudioRecordingSmokeError.emptyRecording
        }

        try WAVFileWriter.write(recording, to: outputURL)
        let attributes = try FileManager.default.attributesOfItem(atPath: outputURL.path)
        let byteCount = (attributes[.size] as? NSNumber)?.int64Value ?? 0
        let observedStats = stats.snapshot()
        let output = NativeAudioRecordingSmokeOutput(
            outputPath: outputURL.path,
            requestedDurationMilliseconds: request.durationMilliseconds,
            sampleCount: recording.samples.count,
            sampleRate: recording.sampleRate,
            durationSeconds: recording.duration,
            maxLevel: observedStats.maxLevel,
            levelObservationCount: observedStats.observationCount,
            byteCount: byteCount,
            microphoneName: request.microphoneName
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(output)
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

private struct NativeAudioRecordingSmokeOutput: Encodable {
    var outputPath: String
    var requestedDurationMilliseconds: Int
    var sampleCount: Int
    var sampleRate: Double
    var durationSeconds: TimeInterval
    var maxLevel: Float
    var levelObservationCount: Int
    var byteCount: Int64
    var microphoneName: String?
}

private final class LockedAudioRecordingStats: @unchecked Sendable {
    private let lock = NSLock()
    private var maxObservedLevel: Float = 0
    private var observations = 0

    func observe(_ level: Float) {
        lock.withLock {
            maxObservedLevel = max(maxObservedLevel, level)
            observations += 1
        }
    }

    func snapshot() -> (maxLevel: Float, observationCount: Int) {
        lock.withLock {
            (maxObservedLevel, observations)
        }
    }
}

private enum NativeAudioRecordingSmokeError: LocalizedError {
    case microphonePermission(PermissionSnapshot.Microphone)
    case emptyRecording

    var errorDescription: String? {
        switch self {
        case let .microphonePermission(status):
            "Microphone permission is \(status.rawValue); grant microphone access before running --smoke-record-audio."
        case .emptyRecording:
            "The native audio capture smoke produced an empty recording."
        }
    }
}
