import Foundation

enum WAVFileWriter {
    static func write(_ recording: AudioRecording, to url: URL) throws {
        let sampleRate = UInt32(recording.sampleRate.rounded())
        let channelCount: UInt16 = 1
        let bitsPerSample: UInt16 = 16
        let bytesPerSample = UInt16(bitsPerSample / 8)
        let blockAlign = channelCount * bytesPerSample
        let byteRate = sampleRate * UInt32(blockAlign)
        let dataByteCount = UInt32(recording.samples.count * Int(bytesPerSample))
        let riffChunkSize = UInt32(36) + dataByteCount

        var data = Data()
        data.reserveCapacity(44 + Int(dataByteCount))
        data.appendASCII("RIFF")
        data.appendUInt32LittleEndian(riffChunkSize)
        data.appendASCII("WAVE")
        data.appendASCII("fmt ")
        data.appendUInt32LittleEndian(16)
        data.appendUInt16LittleEndian(1)
        data.appendUInt16LittleEndian(channelCount)
        data.appendUInt32LittleEndian(sampleRate)
        data.appendUInt32LittleEndian(byteRate)
        data.appendUInt16LittleEndian(blockAlign)
        data.appendUInt16LittleEndian(bitsPerSample)
        data.appendASCII("data")
        data.appendUInt32LittleEndian(dataByteCount)

        for sample in recording.samples {
            let clamped = max(-1, min(1, sample))
            let pcmSample = Int16((clamped * Float(Int16.max)).rounded())
            data.appendInt16LittleEndian(pcmSample)
        }

        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try data.write(to: url, options: .atomic)
    }
}

private extension Data {
    mutating func appendASCII(_ string: String) {
        append(string.data(using: .ascii)!)
    }

    mutating func appendUInt16LittleEndian(_ value: UInt16) {
        var littleEndian = value.littleEndian
        append(Data(bytes: &littleEndian, count: MemoryLayout<UInt16>.size))
    }

    mutating func appendInt16LittleEndian(_ value: Int16) {
        var littleEndian = value.littleEndian
        append(Data(bytes: &littleEndian, count: MemoryLayout<Int16>.size))
    }

    mutating func appendUInt32LittleEndian(_ value: UInt32) {
        var littleEndian = value.littleEndian
        append(Data(bytes: &littleEndian, count: MemoryLayout<UInt32>.size))
    }
}
