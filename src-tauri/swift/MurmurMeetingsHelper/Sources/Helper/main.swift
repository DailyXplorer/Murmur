// murmur-meetings-helper: on-device analysis of a recorded meeting.
//
//   prepare  --models-dir <dir>
//   analyze  --models-dir <dir> [--system <wav>] [--mic <wav>]
//   diarize  --models-dir <dir> (--system <wav> | --mic <wav>)
//   serve    --models-dir <dir>
//
// Murmur downloads the models itself (`local_models`). `prepare` loads them
// once, so Core ML compiles them for this Mac; `analyze` prints the words
// heard on each track, `diarize` who spoke when on one track: Murmur picks
// it from those words. `serve` keeps Parakeet loaded for dictation and
// transcribes each WAV named on stdin, boosting the terms named with it when
// the optional CTC model is downloaded. Prints one JSON object per line on
// stdout (see `meetings/protocol.rs`). None of them downloads: missing
// models are reported as an error.

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

struct ServeReady: Encodable {
    let type = "ready"
}

struct ServeText: Encodable {
    let type = "text"
    let id: UInt64
    let text: String
}

struct ServeFailure: Encodable {
    let type = "error"
    let id: UInt64
    let stage = "transcribe"
    let code: String
    let message: String
}

struct ServeRequest: Decodable {
    let type: String
    let id: UInt64
    let wav: String
    /// Words to listen for. Absent when there are none.
    let terms: [String]?
}

let output = FileHandle.standardOutput
let encoder = JSONEncoder()

func emit<T: Encodable>(_ message: T) {
    guard var line = try? encoder.encode(message) else { return }
    line.append(0x0A)
    output.write(line)
}

func code(of error: Error) -> String {
    if case DownloadError.modelMissing = error {
        return "model_missing"
    } else if case DownloadError.networkDisabled = error {
        return "model_missing"
    }
    return "io"
}

func fail(_ stage: String, _ error: Error) {
    emit(Failure(stage: stage, code: code(of: error), message: String(describing: error)))
}

func value(of flag: String, in arguments: [String]) -> String? {
    guard let index = arguments.firstIndex(of: flag), index + 1 < arguments.count else {
        return nil
    }
    return arguments[index + 1]
}

/// The share of `prepare` done, from `from` to `to`.
func compileProgress(from: Double, to: Double) -> ProgressHandler {
    { progress in
        emit(
            Progress(
                phase: "compile", track: nil,
                fraction: from + (to - from) * min(max(progress.fractionCompleted, 0), 1)))
    }
}

func asrDirectory(_ models: URL) -> URL {
    models.appendingPathComponent(Repo.parakeetV3.folderName, isDirectory: true)
}

/// The first load compiles the models for this Mac; later loads are fast.
func prepare(models: URL) async -> Int32 {
    ModelHub.offlineMode = true
    do {
        _ = try await AsrModels.load(
            from: asrDirectory(models), version: .v3,
            progressHandler: compileProgress(from: 0, to: 0.8))
        _ = try await OfflineDiarizerModels.load(
            from: models, progressHandler: compileProgress(from: 0.8, to: 1))
        // Optional: without it, dictation is not boosted.
        _ = await Booster.load(models: models)
        emit(Done())
        return 0
    } catch {
        fail("prepare", error)
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

/// The tracks named on the command line, system audio first.
func tracks(in arguments: [String]) -> [(String, URL)] {
    ["system", "mic"].compactMap { track in
        value(of: "--\(track)", in: arguments).map { (track, URL(fileURLWithPath: $0)) }
    }
}

func analyze(models: URL, arguments: [String]) async -> Int32 {
    // Everything the analysis needs is already on disk.
    ModelHub.offlineMode = true
    do {
        let asr = try await AsrModels.load(from: asrDirectory(models), version: .v3)
        for (track, url) in tracks(in: arguments) {
            await transcribe(url, track: track, models: asr)
        }
    } catch {
        fail("asr", error)
    }
    emit(Done())
    return 0
}

func diarize(models: URL, arguments: [String]) async -> Int32 {
    ModelHub.offlineMode = true
    if let (track, url) = tracks(in: arguments).first {
        await diarize(url, track: track, models: models)
    }
    emit(Done())
    return 0
}

/// The terms of one request, ready to rescore a transcript with.
struct Boost: Sendable {
    let terms: [String]
    let spotter: CtcKeywordSpotter
    let vocabulary: CustomVocabularyContext
    let rescorer: VocabularyRescorer

    /// The transcript with the terms boosted, or `nil` when nothing changed.
    func rescored(_ result: ASRResult, spotted: CtcKeywordSpotter.SpotKeywordsResult) -> String? {
        guard let timings = result.tokenTimings, !timings.isEmpty, !spotted.logProbs.isEmpty
        else { return nil }
        let tuned = ContextBiasingConstants.rescorerConfig(forVocabSize: vocabulary.terms.count)
        let output = rescorer.ctcTokenRescore(
            transcript: result.text, tokenTimings: timings, logProbs: spotted.logProbs,
            frameDuration: spotted.frameDuration, cbw: tuned.cbw,
            marginSeconds: ContextBiasingConstants.defaultMarginSeconds,
            minSimilarity: tuned.minSimilarity)
        return output.wasModified ? output.text : nil
    }
}

/// Custom-word boosting: FluidAudio's CTC keyword spotting rescores the
/// Parakeet transcript (FluidAudio's `Documentation/ASR/CustomVocabulary.md`).
/// The CTC model is an optional download, loaded from Murmur's folder rather
/// than FluidAudio's own cache.
final class Booster {
    let spotter: CtcKeywordSpotter
    let tokenizer: CtcTokenizer
    let directory: URL
    /// Building a rescorer reads the tokenizer again, so the last one is kept.
    var last: Boost?

    init(spotter: CtcKeywordSpotter, tokenizer: CtcTokenizer, directory: URL) {
        self.spotter = spotter
        self.tokenizer = tokenizer
        self.directory = directory
    }

    /// `nil` when the model is not downloaded or does not load: boosting is
    /// then off and dictation unchanged.
    static func load(models: URL) async -> Booster? {
        let directory = models.appendingPathComponent(
            Repo.parakeetCtc110m.folderName, isDirectory: true)
        guard CtcModels.modelsExist(at: directory) else { return nil }
        do {
            let ctc = try await CtcModels.loadDirect(from: directory)
            return Booster(
                spotter: CtcKeywordSpotter(models: ctc, blankId: ctc.vocabulary.count),
                tokenizer: try await CtcTokenizer.load(from: directory),
                directory: directory)
        } catch {
            FileHandle.standardError.write(Data("Custom-word boosting is off: \(error)\n".utf8))
            return nil
        }
    }

    /// `nil` when no term can be spotted or the rescorer cannot be built.
    func boost(_ terms: [String]) async -> Boost? {
        if let last, last.terms == terms { return last }
        let vocabulary = CustomVocabularyContext(
            terms: terms.compactMap { text in
                let ids = tokenizer.encode(text)
                return ids.isEmpty ? nil : CustomVocabularyTerm(text: text, ctcTokenIds: ids)
            })
        guard !vocabulary.terms.isEmpty else { return nil }
        // The acoustic rescue replaces far more words that were right.
        guard
            let rescorer = try? await VocabularyRescorer.create(
                spotter: spotter, vocabulary: vocabulary,
                config: VocabularyRescorer.Config(spotterRescueEnabled: false),
                ctcModelDirectory: directory)
        else { return nil }
        last = Boost(terms: terms, spotter: spotter, vocabulary: vocabulary, rescorer: rescorer)
        return last
    }
}

/// Loads Parakeet once, then answers each request read on stdin until
/// Murmur closes it. Pieces are at most 15 s, one pass of the encoder.
func serve(models: URL) async -> Int32 {
    ModelHub.offlineMode = true
    let manager = AsrManager(config: .default)
    do {
        try await manager.loadModels(
            try await AsrModels.load(from: asrDirectory(models), version: .v3))
    } catch {
        fail("load", error)
        return 1
    }
    let booster = await Booster.load(models: models)
    emit(ServeReady())
    let decoder = JSONDecoder()
    while let line = readLine() {
        guard let request = try? decoder.decode(ServeRequest.self, from: Data(line.utf8)),
            request.type == "transcribe"
        else { continue }
        do {
            let samples = try AudioConverter().resampleAudioFile(URL(fileURLWithPath: request.wav))
            var state = TdtDecoderState.make(decoderLayers: await manager.decoderLayerCount)
            let terms = request.terms ?? []
            guard !terms.isEmpty, let boost = await booster?.boost(terms) else {
                let result = try await manager.transcribe(samples, decoderState: &state)
                emit(ServeText(id: request.id, text: result.text))
                continue
            }
            // The CTC pass needs only the audio, so it runs beside Parakeet.
            async let spotting = boost.spotter.spotKeywordsWithLogProbs(
                audioSamples: samples, customVocabulary: boost.vocabulary)
            let result = try await manager.transcribe(samples, decoderState: &state)
            let boosted = (try? await spotting).flatMap { boost.rescored(result, spotted: $0) }
            emit(ServeText(id: request.id, text: boosted ?? result.text))
        } catch {
            emit(
                ServeFailure(
                    id: request.id, code: code(of: error), message: String(describing: error)))
        }
    }
    return 0
}

// Library logs below warnings could hold transcript text.
AppLogger.minimumLevel = .warning

let arguments = Array(CommandLine.arguments.dropFirst())
guard let command = arguments.first, let modelsPath = value(of: "--models-dir", in: arguments)
else {
    FileHandle.standardError.write(
        Data("usage: prepare|analyze|diarize|serve --models-dir <dir> …\n".utf8))
    exit(2)
}

// Murmur holds stdin open: end of file means Murmur quit or crashed, so the
// work stops instead of running on unseen. `serve` reads its requests there
// and stops at the same end of file.
if command != "serve" {
    Thread.detachNewThread {
        while !FileHandle.standardInput.availableData.isEmpty {}
        exit(1)
    }
}

let models = URL(fileURLWithPath: modelsPath, isDirectory: true)
switch command {
case "prepare":
    exit(await prepare(models: models))
case "analyze":
    exit(await analyze(models: models, arguments: arguments))
case "diarize":
    exit(await diarize(models: models, arguments: arguments))
case "serve":
    exit(await serve(models: models))
default:
    FileHandle.standardError.write(Data("unknown command \(command)\n".utf8))
    exit(2)
}
