// echo-probe: measures what macOS voice processing does to the microphone
// of a meeting held on speakers, before Murmur records with it. Not shipped.
//
// Another process (`afplay`, standing in for the call app) plays a voice on
// the default output while the microphone records:
//   1. raw, with nothing else running;
//   2. with voice processing on in this process, while a child process
//      records the raw microphone at the same moment;
//   3. as 2, while you read a sentence aloud over the voice (double talk).
// It reports the echo removed (ERLE), how much quieter the other app's sound
// got (ducking), and, given the meetings helper and its models, how many of
// your words each recording of the double talk keeps.
//
// Build: swiftc -O scripts/echo-probe/main.swift -o /tmp/echo-probe
// Run:   /tmp/echo-probe [--out DIR] [--helper PATH --models-dir DIR]

import AVFoundation
import CoreAudio
import Foundation

let frameSeconds = 0.02
/// What you read aloud during the double talk.
let sentence =
    "The quarterly budget review moves to Thursday afternoon, and the new hires start on Monday morning."
/// What the other side says, about twenty seconds.
let remoteText = """
    Thanks for joining. Before we start, a quick update on the schedule. The release candidate \
    is ready, the last tests run this week, and support gets the notes on Friday. We still need \
    someone for the second talk at the conference, and the booth needs two demo laptops.
    """

func value(of flag: String, in arguments: [String]) -> String? {
    guard let index = arguments.firstIndex(of: flag), index + 1 < arguments.count else { return nil }
    return arguments[index + 1]
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("echo-probe: \(message)\n".utf8))
    exit(1)
}

// MARK: Recording

/// Records the default input, mono, from the first channel, which carries
/// the processed signal when voice processing is on.
final class Recorder: @unchecked Sendable {
    let engine = AVAudioEngine()
    private let lock = NSLock()
    private(set) var samples: [Float] = []
    private(set) var sampleRate: Double = 0
    /// Host time of the first sample, in seconds, to line up two recordings.
    private(set) var startedAt: Double?

    init(voiceProcessing: Bool) throws {
        let input = engine.inputNode
        if voiceProcessing {
            try input.setVoiceProcessingEnabled(true)
            if #available(macOS 14.0, *) {
                input.voiceProcessingOtherAudioDuckingConfiguration =
                    AVAudioVoiceProcessingOtherAudioDuckingConfiguration(
                        enableAdvancedDucking: false, duckingLevel: .min)
            }
            // Voice processing runs input and output as one unit.
            _ = engine.mainMixerNode
        }
        let format = input.outputFormat(forBus: 0)
        sampleRate = format.sampleRate
        input.installTap(onBus: 0, bufferSize: 4096, format: format) { [weak self] buffer, time in
            guard let self, let channel = buffer.floatChannelData?[0] else { return }
            let frames = Int(buffer.frameLength)
            self.lock.lock()
            if self.startedAt == nil, time.isHostTimeValid {
                self.startedAt = AVAudioTime.seconds(forHostTime: time.hostTime)
            }
            self.samples.append(contentsOf: UnsafeBufferPointer(start: channel, count: frames))
            self.lock.unlock()
        }
    }

    func start() throws { try engine.start() }

    func stop() {
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
    }

    func write(to url: URL) throws {
        lock.lock()
        defer { lock.unlock() }
        try writeWav(samples, sampleRate: sampleRate, to: url)
    }
}

func writeWav(_ samples: [Float], sampleRate: Double, to url: URL) throws {
    guard let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1),
          let buffer = AVAudioPCMBuffer(
            pcmFormat: format, frameCapacity: AVAudioFrameCount(max(samples.count, 1)))
    else { fail("cannot make an audio buffer") }
    buffer.frameLength = AVAudioFrameCount(samples.count)
    samples.withUnsafeBufferPointer { source in
        buffer.floatChannelData![0].update(from: source.baseAddress!, count: samples.count)
    }
    let file = try AVAudioFile(forWriting: url, settings: format.settings)
    try file.write(from: buffer)
}

func readWav(_ url: URL) -> (samples: [Float], sampleRate: Double) {
    guard let file = try? AVAudioFile(forReading: url),
          let buffer = AVAudioPCMBuffer(
            pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)),
          (try? file.read(into: buffer)) != nil,
          let channel = buffer.floatChannelData?[0]
    else { fail("cannot read \(url.path)") }
    return (Array(UnsafeBufferPointer(start: channel, count: Int(buffer.frameLength))),
            file.processingFormat.sampleRate)
}

func run(_ path: String, _ arguments: [String]) -> Process {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: path)
    process.arguments = arguments
    do { try process.run() } catch { fail("cannot run \(path): \(error)") }
    return process
}

/// Child mode: records the raw microphone for `seconds` into `path`, and
/// prints the host time of its first sample.
func recordRaw(to path: String, seconds: Double) -> Never {
    do {
        let recorder = try Recorder(voiceProcessing: false)
        try recorder.start()
        Thread.sleep(forTimeInterval: seconds)
        recorder.stop()
        try recorder.write(to: URL(fileURLWithPath: path))
        print(recorder.startedAt ?? 0)
        exit(0)
    } catch {
        fail("raw recording failed: \(error)")
    }
}

/// Plays `clip` from another process while recording: raw in this process,
/// or with voice processing here and raw in a child at the same moment.
/// Returns the recordings, aligned on the host clock.
func session(clip: URL, seconds: Double, voiceProcessing: Bool, out: URL, name: String)
    -> (raw: [Float], processed: [Float]?, sampleRate: Double)
{
    let rawPath = out.appendingPathComponent("\(name)-raw.wav")
    var child: (Process, Pipe)?
    if voiceProcessing {
        let pipe = Pipe()
        let process = Process()
        process.executableURL = URL(fileURLWithPath: CommandLine.arguments[0])
        process.arguments = ["record-raw", rawPath.path, String(seconds + 2)]
        process.standardOutput = pipe
        do { try process.run() } catch { fail("cannot start the raw recorder: \(error)") }
        child = (process, pipe)
        Thread.sleep(forTimeInterval: 0.5)
    }
    let recorder: Recorder
    do {
        recorder = try Recorder(voiceProcessing: voiceProcessing)
        try recorder.start()
    } catch {
        fail("the microphone did not start (voice processing \(voiceProcessing)): \(error)")
    }
    Thread.sleep(forTimeInterval: 0.5)
    let player = run("/usr/bin/afplay", [clip.path])
    player.waitUntilExit()
    Thread.sleep(forTimeInterval: 0.5)
    recorder.stop()
    guard let (process, pipe) = child else {
        try? recorder.write(to: rawPath)
        return (recorder.samples, nil, recorder.sampleRate)
    }
    try? recorder.write(to: out.appendingPathComponent("\(name)-vp.wav"))
    process.waitUntilExit()
    let printed = String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
    let rawStart = Double(printed.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 0
    let raw = readWav(rawPath)
    // Both on one timeline: the raw recording from where the processed one starts.
    let lead = Int(((recorder.startedAt ?? rawStart) - rawStart) * raw.sampleRate)
    let alignedRaw = lead >= 0 ? Array(raw.samples.dropFirst(lead)) : raw.samples
    let processed = resample(recorder.samples, from: recorder.sampleRate, to: raw.sampleRate)
    return (alignedRaw, processed, raw.sampleRate)
}

/// Nearest-sample resampling: enough for 20 ms levels.
func resample(_ samples: [Float], from: Double, to: Double) -> [Float] {
    guard from != to, from > 0 else { return samples }
    let count = Int(Double(samples.count) * to / from)
    return (0..<count).map { samples[min(Int(Double($0) * from / to), samples.count - 1)] }
}

// MARK: Measures

/// The power of each 20 ms frame.
func framePowers(_ samples: [Float], sampleRate: Double) -> [Double] {
    let size = Int(sampleRate * frameSeconds)
    return stride(from: 0, to: samples.count - size, by: size).map { start in
        samples[start..<start + size].reduce(0) { $0 + Double($1 * $1) } / Double(size)
    }
}

func decibels(_ power: Double) -> Double { 10 * log10(max(power, 1e-12)) }

/// The frames where the other app's voice is heard: 15 dB over the
/// recording's quietest fifth.
func voiced(_ powers: [Double]) -> [Int] {
    let sorted = powers.sorted()
    guard !sorted.isEmpty else { return [] }
    let floor = sorted[sorted.count / 5]
    return powers.indices.filter { decibels(powers[$0]) > decibels(floor) + 15 }
}

func meanDecibels(_ powers: [Double], at frames: [Int]) -> Double {
    let inside = frames.filter { $0 < powers.count }
    guard !inside.isEmpty else { return -120 }
    return decibels(inside.reduce(0) { $0 + powers[$1] } / Double(inside.count))
}

/// The words the meetings helper hears in `wav`, lowercased letters and digits.
func heardWords(_ wav: URL, helper: String, models: String) -> [String] {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: helper)
    process.arguments = ["analyze", "--models-dir", models, "--mic", wav.path]
    let input = Pipe(), output = Pipe()
    // The helper stops when its standard input closes.
    process.standardInput = input
    process.standardOutput = output
    do { try process.run() } catch { fail("cannot run the helper: \(error)") }
    let data = output.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    try? input.fileHandleForWriting.close()
    var words: [String] = []
    for line in String(decoding: data, as: UTF8.self).split(separator: "\n") {
        guard let json = try? JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any],
              json["type"] as? String == "words",
              let list = json["words"] as? [[String: Any]]
        else { continue }
        words += list.compactMap { $0["w"] as? String }.map(normalized)
    }
    return words
}

func normalized(_ word: String) -> String {
    word.lowercased().filter { $0.isLetter || $0.isNumber }
}

/// Share of the words of `sentence` found in `words`, in order.
func recall(of sentence: String, in words: [String]) -> Double {
    let wanted = sentence.split(separator: " ").map { normalized(String($0)) }
    var found = 0
    var rest = words[...]
    for word in wanted {
        if let index = rest.firstIndex(of: word) {
            found += 1
            rest = rest[(index + 1)...]
        }
    }
    return Double(found) / Double(wanted.count)
}

// MARK: Output route

func fourCC(_ code: UInt32) -> String {
    let bytes = [24, 16, 8, 0].map { UInt8((code >> $0) & 0xFF) }
    return String(decoding: bytes, as: UTF8.self)
}

/// The default output's transport and data source, never its name.
func outputRoute() -> String {
    var address = AudioObjectPropertyAddress(
        mSelector: kAudioHardwarePropertyDefaultOutputDevice,
        mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var device = AudioObjectID(kAudioObjectUnknown)
    var size = UInt32(MemoryLayout<AudioObjectID>.size)
    guard AudioObjectGetPropertyData(
        AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device) == noErr
    else { return "none" }
    func read(_ selector: AudioObjectPropertySelector, _ scope: AudioObjectPropertyScope) -> UInt32 {
        var address = AudioObjectPropertyAddress(
            mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
        var value: UInt32 = 0
        var size = UInt32(MemoryLayout<UInt32>.size)
        return AudioObjectGetPropertyData(device, &address, 0, nil, &size, &value) == noErr ? value : 0
    }
    let transport = read(kAudioDevicePropertyTransportType, kAudioObjectPropertyScopeGlobal)
    let source = read(kAudioDevicePropertyDataSource, kAudioObjectPropertyScopeOutput)
    return "transport '\(fourCC(transport))', source '\(source == 0 ? "none" : fourCC(source))'"
}

// MARK: Main

let arguments = Array(CommandLine.arguments.dropFirst())
if arguments.first == "record-raw", arguments.count == 3, let seconds = Double(arguments[2]) {
    recordRaw(to: arguments[1], seconds: seconds)
}

let out = URL(fileURLWithPath: value(of: "--out", in: arguments) ?? NSTemporaryDirectory())
    .appendingPathComponent("echo-probe-\(Int(Date().timeIntervalSince1970))")
try? FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
let clip = out.appendingPathComponent("remote.wav")
let said = run("/usr/bin/say", [
    "-r", "170", "-o", clip.path, "--file-format=WAVE", "--data-format=LEI16@48000", remoteText,
])
said.waitUntilExit()
guard said.terminationStatus == 0, let clipFile = try? AVAudioFile(forReading: clip) else {
    fail("say could not write the remote voice")
}
let seconds = Double(clipFile.length) / clipFile.processingFormat.sampleRate + 1.5

print("Output: \(outputRoute()). Use the built-in speakers at a normal volume, in a quiet room.")
print("Recordings go to \(out.path)")
print("1/3 Raw microphone while another app plays a voice (\(Int(seconds)) s)…")
let alone = session(clip: clip, seconds: seconds, voiceProcessing: false, out: out, name: "1")
print("2/3 The same, with voice processing on (and the raw microphone at the same time)…")
let processing = session(clip: clip, seconds: seconds, voiceProcessing: true, out: out, name: "2")
print("3/3 Double talk. Read this aloud, at your normal voice, as soon as the voice starts:")
print("\n    \(sentence)\n")
for count in [3, 2, 1] {
    print("\(count)…")
    Thread.sleep(forTimeInterval: 1)
}
let doubleTalk = session(clip: clip, seconds: seconds, voiceProcessing: true, out: out, name: "3")

let alonePowers = framePowers(alone.raw, sampleRate: alone.sampleRate)
let rawPowers = framePowers(processing.raw, sampleRate: processing.sampleRate)
let processedPowers = framePowers(processing.processed ?? [], sampleRate: processing.sampleRate)
let echoFrames = voiced(rawPowers)
let erle = meanDecibels(rawPowers, at: echoFrames) - meanDecibels(processedPowers, at: echoFrames)
let ducking = meanDecibels(alonePowers, at: voiced(alonePowers))
    - meanDecibels(rawPowers, at: echoFrames)

print("\nOutput route: \(outputRoute())")
print(String(format: "Echo removed (ERLE): %.1f dB (gate: at least 15 dB)", erle))
print(String(format: "Ducking of the other app's sound: %.1f dB (gate: at most 3 dB)", ducking))
var passes = erle >= 15 && ducking <= 3
if let helper = value(of: "--helper", in: arguments),
   let models = value(of: "--models-dir", in: arguments)
{
    let rawRecall = recall(
        of: sentence, in: heardWords(out.appendingPathComponent("3-raw.wav"), helper: helper, models: models))
    let processedRecall = recall(
        of: sentence, in: heardWords(out.appendingPathComponent("3-vp.wav"), helper: helper, models: models))
    let ratio = rawRecall > 0 ? processedRecall / rawRecall : 0
    print(String(
        format: "Your words kept in double talk: raw %.0f %%, voice processing %.0f %%, ratio %.0f %% (gate: at least 95 %%)",
        rawRecall * 100, processedRecall * 100, ratio * 100))
    passes = passes && ratio >= 0.95
} else {
    print("Double talk recall: not measured (pass --helper and --models-dir).")
}
print(passes ? "Gate: passes." : "Gate: fails.")
