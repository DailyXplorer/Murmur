// murmur-meetings-helper: on-device analysis of a recorded meeting.
//
//   download --models-dir <dir>
//   analyze  --models-dir <dir> [--system <wav>] [--mic <wav>] [--diarize system|mic|none]
//
// Prints one JSON object per line on stdout (see `meetings/protocol.rs`).
// `analyze` never downloads: missing models are reported as an error.

import FluidAudio
import Foundation

struct Progress: Encodable {
    let type = "progress"
    let phase: String
    let track: String?
    let fraction: Double
}

struct Word: Encodable {
    let w: String
    let s: Double
    let e: Double
}

struct Words: Encodable {
    let type = "words"
    let track: String
    let words: [Word]
}

struct Turn: Encodable {
    let spk: String
    let s: Double
    let e: Double
}

struct Diarization: Encodable {
    let type = "diarization"
    let track: String
    let segments: [Turn]
}

struct Failure: Encodable {
    let type = "error"
    let stage: String
    let code: String
    let message: String
}

struct Done: Encodable {
    let type = "done"
}

let output = FileHandle.standardOutput
let encoder = JSONEncoder()

func emit<T: Encodable>(_ message: T) {
    guard var line = try? encoder.encode(message) else { return }
    line.append(0x0A)
    output.write(line)
}

func fail(_ stage: String, _ error: Error) {
    let code: String
    if case DownloadError.modelMissing = error {
        code = "model_missing"
    } else if case DownloadError.networkDisabled = error {
        code = "model_missing"
    } else {
        code = "io"
    }
    emit(Failure(stage: stage, code: code, message: String(describing: error)))
}

func value(of flag: String, in arguments: [String]) -> String? {
    guard let index = arguments.firstIndex(of: flag), index + 1 < arguments.count else {
        return nil
    }
    return arguments[index + 1]
}

/// Download, then compile: FluidAudio reports both through one handler.
func progressHandler(from: Double, to: Double) -> ProgressHandler {
    { progress in
        let phase: String
        switch progress.phase {
        case .compiling: phase = "compile"
        default: phase = "download"
        }
        emit(
            Progress(
                phase: phase, track: nil,
                fraction: from + (to - from) * min(max(progress.fractionCompleted, 0), 1)))
    }
}

func asrDirectory(_ models: URL) -> URL {
    models.appendingPathComponent(Repo.parakeetV3.folderName, isDirectory: true)
}

func download(models: URL) async -> Int32 {
    ModelHub.offlineMode = false
    do {
        try await AsrModels.download(
            to: asrDirectory(models), version: .v3,
            progressHandler: progressHandler(from: 0, to: 0.8))
        _ = try await OfflineDiarizerModels.load(
            from: models, progressHandler: progressHandler(from: 0.8, to: 0.9))
        // The first load compiles the models for this Mac; later loads are fast.
        emit(Progress(phase: "compile", track: nil, fraction: 0.9))
        _ = try await AsrModels.load(from: asrDirectory(models), version: .v3)
        emit(Progress(phase: "compile", track: nil, fraction: 1))
        emit(Done())
        return 0
    } catch {
        fail("download", error)
        return 1
    }
}

func transcribe(_ url: URL, track: String, models: AsrModels) async {
    emit(Progress(phase: "asr", track: track, fraction: 0))
    do {
        let manager = AsrManager(config: .default)
        try await manager.loadModels(models)
        var state = TdtDecoderState.make(decoderLayers: await manager.decoderLayerCount)
        let result = try await manager.transcribe(url, decoderState: &state)
        let words = buildWordTimings(from: result.tokenTimings ?? []).map {
            Word(w: $0.word, s: $0.startTime, e: $0.endTime)
        }
        emit(Words(track: track, words: words))
        emit(Progress(phase: "asr", track: track, fraction: 1))
    } catch {
        fail("asr", error)
    }
}

func diarize(_ url: URL, track: String, models: URL) async {
    emit(Progress(phase: "diarize", track: track, fraction: 0))
    do {
        let diarizer = OfflineDiarizerManager()
        diarizer.initialize(models: try await OfflineDiarizerModels.load(from: models))
        let result = try await diarizer.process(url) { done, total in
            guard total > 0 else { return }
            emit(Progress(phase: "diarize", track: track, fraction: Double(done) / Double(total)))
        }
        emit(
            Diarization(
                track: track,
                segments: result.segments.map {
                    Turn(
                        spk: $0.speakerId, s: Double($0.startTimeSeconds),
                        e: Double($0.endTimeSeconds))
                }))
    } catch {
        fail("diarize", error)
    }
}

func analyze(models: URL, arguments: [String]) async -> Int32 {
    // Everything the analysis needs is already on disk.
    ModelHub.offlineMode = true
    var tracks: [(String, URL)] = []
    for track in ["system", "mic"] {
        if let path = value(of: "--\(track)", in: arguments) {
            tracks.append((track, URL(fileURLWithPath: path)))
        }
    }
    let diarized = value(of: "--diarize", in: arguments) ?? "none"
    do {
        let asr = try await AsrModels.load(from: asrDirectory(models), version: .v3)
        for (track, url) in tracks {
            await transcribe(url, track: track, models: asr)
        }
    } catch {
        fail("asr", error)
    }
    if let (track, url) = tracks.first(where: { $0.0 == diarized }) {
        await diarize(url, track: track, models: models)
    }
    emit(Done())
    return 0
}

// Library logs below warnings could hold transcript text.
AppLogger.minimumLevel = .warning

// Murmur holds stdin open and never writes to it: end of file means Murmur
// quit or crashed, so the analysis stops instead of running on unseen.
Thread.detachNewThread {
    while !FileHandle.standardInput.availableData.isEmpty {}
    exit(1)
}

let arguments = Array(CommandLine.arguments.dropFirst())
guard let command = arguments.first, let modelsPath = value(of: "--models-dir", in: arguments)
else {
    FileHandle.standardError.write(Data("usage: download|analyze --models-dir <dir> …\n".utf8))
    exit(2)
}
let models = URL(fileURLWithPath: modelsPath, isDirectory: true)
switch command {
case "download":
    exit(await download(models: models))
case "analyze":
    exit(await analyze(models: models, arguments: arguments))
default:
    FileHandle.standardError.write(Data("unknown command \(command)\n".utf8))
    exit(2)
}
