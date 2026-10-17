// C ABI to keep a meeting's audio small: its 16 kHz mono WAV tracks go into
// one AAC file (channel 0 the first track, channel 1 the second), and come
// back out as WAVs to transcribe the meeting again. Status codes are mirrored
// in src/meetings/archive.rs. ExtAudioFile does the AAC work and trims the
// encoder's priming frames, so the decoded tracks keep their timing.

import AudioToolbox
import Foundation

private let ok: Int32 = 0
private let openFailed: Int32 = 1
private let createFailed: Int32 = 2
private let ioFailed: Int32 = 3
private let channelMismatch: Int32 = 4

private let sampleRate: Double = 16_000
private let blockFrames = 4_096
/// About 29 MB an hour with two channels.
private let bitRatePerChannel: UInt32 = 32_000

private func floatFormat(channels: UInt32) -> AudioStreamBasicDescription {
    AudioStreamBasicDescription(
        mSampleRate: sampleRate, mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
        mBytesPerPacket: 4 * channels, mFramesPerPacket: 1, mBytesPerFrame: 4 * channels,
        mChannelsPerFrame: channels, mBitsPerChannel: 32, mReserved: 0)
}

private func url(_ path: UnsafePointer<CChar>) -> CFURL {
    URL(fileURLWithPath: String(cString: path)) as CFURL
}

private func setClientFormat(_ file: ExtAudioFileRef, channels: UInt32) -> Bool {
    var format = floatFormat(channels: channels)
    return ExtAudioFileSetProperty(
        file, kExtAudioFileProperty_ClientDataFormat,
        UInt32(MemoryLayout<AudioStreamBasicDescription>.size), &format) == noErr
}

private func fileChannels(_ file: ExtAudioFileRef) -> UInt32? {
    var format = AudioStreamBasicDescription()
    var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    guard ExtAudioFileGetProperty(file, kExtAudioFileProperty_FileDataFormat, &size, &format)
        == noErr
    else { return nil }
    return format.mChannelsPerFrame
}

/// Opens `path` to read Float32 frames at 16 kHz with `channels` channels.
private func openReader(_ path: UnsafePointer<CChar>, channels: UInt32) -> ExtAudioFileRef? {
    var file: ExtAudioFileRef?
    guard ExtAudioFileOpenURL(url(path), &file) == noErr, let file else { return nil }
    guard setClientFormat(file, channels: channels) else {
        ExtAudioFileDispose(file)
        return nil
    }
    return file
}

/// Creates `path` (replacing it) to write Float32 frames into `format`.
private func createWriter(
    _ path: UnsafePointer<CChar>, type: AudioFileTypeID, format: AudioStreamBasicDescription,
    channels: UInt32
) -> ExtAudioFileRef? {
    var format = format
    var file: ExtAudioFileRef?
    guard ExtAudioFileCreateWithURL(
        url(path), type, &format, nil, AudioFileFlags.eraseFile.rawValue, &file) == noErr,
        let file
    else { return nil }
    guard setClientFormat(file, channels: channels) else {
        ExtAudioFileDispose(file)
        return nil
    }
    return file
}

/// Reads up to `buffer.count / channels` frames; 0 at the end, -1 on error.
private func read(_ file: ExtAudioFileRef, into buffer: inout [Float], channels: UInt32) -> Int {
    var frames = UInt32(buffer.count) / channels
    let status = buffer.withUnsafeMutableBytes { bytes -> OSStatus in
        var list = AudioBufferList(
            mNumberBuffers: 1,
            mBuffers: AudioBuffer(
                mNumberChannels: channels, mDataByteSize: UInt32(bytes.count),
                mData: bytes.baseAddress))
        return ExtAudioFileRead(file, &frames, &list)
    }
    return status == noErr ? Int(frames) : -1
}

private func write(
    _ file: ExtAudioFileRef, _ buffer: inout [Float], frames: Int, channels: UInt32
) -> Bool {
    buffer.withUnsafeMutableBytes { bytes -> OSStatus in
        var list = AudioBufferList(
            mNumberBuffers: 1,
            mBuffers: AudioBuffer(
                mNumberChannels: channels, mDataByteSize: UInt32(frames) * 4 * channels,
                mData: bytes.baseAddress))
        return ExtAudioFileWrite(file, UInt32(frames), &list)
    } == noErr
}

/// Compresses one or two WAV tracks into the AAC file `output`. The shorter
/// track is padded with silence.
@_cdecl("murmur_encode_m4a")
public func murmurEncodeM4a(
    _ first: UnsafePointer<CChar>, _ second: UnsafePointer<CChar>?,
    _ output: UnsafePointer<CChar>
) -> Int32 {
    let channels: UInt32 = second == nil ? 1 : 2
    var inputs: [ExtAudioFileRef] = []
    defer { inputs.forEach { ExtAudioFileDispose($0) } }
    for path in [first, second].compactMap({ $0 }) {
        guard let input = openReader(path, channels: 1) else { return openFailed }
        inputs.append(input)
    }

    var aac = AudioStreamBasicDescription()
    aac.mFormatID = kAudioFormatMPEG4AAC
    aac.mSampleRate = sampleRate
    aac.mChannelsPerFrame = channels
    guard let out = createWriter(output, type: kAudioFileM4AType, format: aac, channels: channels)
    else { return createFailed }
    var disposed = false
    defer { if !disposed { ExtAudioFileDispose(out) } }

    // Best effort: the encoder keeps its default rate when it refuses this.
    var converter: AudioConverterRef?
    var size = UInt32(MemoryLayout<AudioConverterRef?>.size)
    if ExtAudioFileGetProperty(out, kExtAudioFileProperty_AudioConverter, &size, &converter)
        == noErr, let converter
    {
        var bitRate = bitRatePerChannel * channels
        if AudioConverterSetProperty(
            converter, kAudioConverterEncodeBitRate, UInt32(MemoryLayout<UInt32>.size), &bitRate)
            == noErr
        {
            var config: CFPropertyList?
            ExtAudioFileSetProperty(
                out, kExtAudioFileProperty_ConverterConfig,
                UInt32(MemoryLayout<CFPropertyList?>.size), &config)
        }
    }

    var tracks = inputs.map { _ in [Float](repeating: 0, count: blockFrames) }
    var finished = inputs.map { _ in false }
    var mixed = [Float](repeating: 0, count: blockFrames * Int(channels))
    while finished.contains(false) {
        var frames = 0
        for index in inputs.indices {
            var count = 0
            if !finished[index] {
                count = read(inputs[index], into: &tracks[index], channels: 1)
                if count < 0 { return ioFailed }
                finished[index] = count == 0
            }
            for frame in count..<blockFrames { tracks[index][frame] = 0 }
            frames = max(frames, count)
        }
        if frames == 0 { continue }
        for frame in 0..<frames {
            for index in inputs.indices {
                mixed[frame * Int(channels) + index] = tracks[index][frame]
            }
        }
        guard write(out, &mixed, frames: frames, channels: channels) else { return ioFailed }
    }
    disposed = true
    return ExtAudioFileDispose(out) == noErr ? ok : ioFailed
}

/// Decodes an AAC file back into one 16 kHz mono 16-bit WAV per channel:
/// `first` for channel 0 and `second` for channel 1. The file must have as
/// many channels as paths are given.
@_cdecl("murmur_decode_m4a")
public func murmurDecodeM4a(
    _ input: UnsafePointer<CChar>, _ first: UnsafePointer<CChar>,
    _ second: UnsafePointer<CChar>?
) -> Int32 {
    let channels: UInt32 = second == nil ? 1 : 2
    var source: ExtAudioFileRef?
    guard ExtAudioFileOpenURL(url(input), &source) == noErr, let source else {
        return openFailed
    }
    defer { ExtAudioFileDispose(source) }
    guard fileChannels(source) == channels else { return channelMismatch }
    guard setClientFormat(source, channels: channels) else { return openFailed }

    let pcm = AudioStreamBasicDescription(
        mSampleRate: sampleRate, mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked,
        mBytesPerPacket: 2, mFramesPerPacket: 1, mBytesPerFrame: 2,
        mChannelsPerFrame: 1, mBitsPerChannel: 16, mReserved: 0)
    var outputs: [ExtAudioFileRef] = []
    var disposed = false
    defer { if !disposed { outputs.forEach { ExtAudioFileDispose($0) } } }
    for path in [first, second].compactMap({ $0 }) {
        guard let output = createWriter(path, type: kAudioFileWAVEType, format: pcm, channels: 1)
        else { return createFailed }
        outputs.append(output)
    }

    var mixed = [Float](repeating: 0, count: blockFrames * Int(channels))
    var track = [Float](repeating: 0, count: blockFrames)
    while true {
        let frames = read(source, into: &mixed, channels: channels)
        if frames < 0 { return ioFailed }
        if frames == 0 { break }
        for (index, output) in outputs.enumerated() {
            for frame in 0..<frames { track[frame] = mixed[frame * Int(channels) + index] }
            guard write(output, &track, frames: frames, channels: 1) else { return ioFailed }
        }
    }
    disposed = true
    let statuses = outputs.map { ExtAudioFileDispose($0) }
    return statuses.allSatisfy { $0 == noErr } ? ok : ioFailed
}

/// The length of an audio file in milliseconds, or -1 when it cannot be
/// read.
@_cdecl("murmur_audio_duration_ms")
public func murmurAudioDurationMs(_ path: UnsafePointer<CChar>) -> Int64 {
    var file: ExtAudioFileRef?
    guard ExtAudioFileOpenURL(url(path), &file) == noErr, let file else { return -1 }
    defer { ExtAudioFileDispose(file) }
    var format = AudioStreamBasicDescription()
    var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    var frames: Int64 = 0
    var framesSize = UInt32(MemoryLayout<Int64>.size)
    guard ExtAudioFileGetProperty(file, kExtAudioFileProperty_FileDataFormat, &size, &format)
        == noErr,
        ExtAudioFileGetProperty(file, kExtAudioFileProperty_FileLengthFrames, &framesSize, &frames)
        == noErr,
        format.mSampleRate > 0
    else { return -1 }
    return Int64(Double(frames) * 1000 / format.mSampleRate)
}
