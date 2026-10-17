// C ABI over a Core Audio process tap, for the system audio track of a
// meeting. Status codes are mirrored in src/meetings/capture/system.rs.
//
// The flow follows Apple's "Capturing system audio with Core Audio taps" and
// the open-source AudioCap app: a global tap of every process but Murmur, a
// private aggregate device that carries it, and an IOProc on that device.
// Process taps need macOS 14.4; every use sits behind `#available`, so the
// symbols are weak and the app still launches on older macOS.

import CoreAudio
import Foundation

private let ok: Int32 = 0
private let osTooOld: Int32 = 1
private let alreadyRunning: Int32 = 2
private let tapFailed: Int32 = 3
private let aggregateFailed: Int32 = 4
private let formatUnsupported: Int32 = 5
private let deviceFailed: Int32 = 6

public typealias MurmurSamplesCallback =
    @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<Float>?, Int32, Double) -> Void

@_cdecl("murmur_system_audio_support")
public func murmurSystemAudioSupport() -> Int32 {
    if #available(macOS 14.4, *) { return ok }
    return osTooOld
}

/// The on-device analysis helper (FluidAudio) needs macOS 14.
@_cdecl("murmur_meeting_analysis_support")
public func murmurMeetingAnalysisSupport() -> Int32 {
    if #available(macOS 14.0, *) { return ok }
    return osTooOld
}

private let sessionLock = NSLock()
private var session: AnyObject?

/// Starts tapping system audio and calls `onSamples` with mono Float32 frames
/// from a private queue until `murmur_system_tap_stop` returns.
@_cdecl("murmur_system_tap_start")
public func murmurSystemTapStart(
    _ context: UnsafeMutableRawPointer?,
    _ onSamples: MurmurSamplesCallback
) -> Int32 {
    guard #available(macOS 14.4, *) else { return osTooOld }
    sessionLock.lock()
    defer { sessionLock.unlock() }
    if session != nil { return alreadyRunning }
    let tap = SystemTap(context: context, callback: onSamples)
    let status = tap.start()
    if status == ok { session = tap }
    return status
}

/// Stops the tap. No callback runs once this returns.
@_cdecl("murmur_system_tap_stop")
public func murmurSystemTapStop() {
    guard #available(macOS 14.4, *) else { return }
    sessionLock.lock()
    let tap = session as? SystemTap
    session = nil
    sessionLock.unlock()
    tap?.stop()
}

@available(macOS 14.4, *)
private final class SystemTap: @unchecked Sendable {
    private let context: UnsafeMutableRawPointer?
    private let callback: MurmurSamplesCallback
    /// Runs the IOProc. Separate from `control`, so stopping the device from
    /// `control` never waits on itself.
    private let ioQueue = DispatchQueue(label: "murmur.meeting.system-audio.io", qos: .userInitiated)
    /// Serializes start, stop and output-device changes.
    private let control = DispatchQueue(label: "murmur.meeting.system-audio.control")

    private var tapID = AudioObjectID(kAudioObjectUnknown)
    private var tapUID = ""
    private var aggregateID = AudioObjectID(kAudioObjectUnknown)
    private var procID: AudioDeviceIOProcID?
    private var outputListener: AudioObjectPropertyListenerBlock?
    private var stopped = false
    private var mono: [Float] = []

    init(context: UnsafeMutableRawPointer?, callback: MurmurSamplesCallback) {
        self.context = context
        self.callback = callback
    }

    func start() -> Int32 {
        control.sync {
            let description = CATapDescription(stereoGlobalTapButExcludeProcesses: ownProcessObjects())
            description.uuid = UUID()
            description.isPrivate = true
            description.muteBehavior = .unmuted
            var tap = AudioObjectID(kAudioObjectUnknown)
            guard AudioHardwareCreateProcessTap(description, &tap) == noErr,
                  tap != kAudioObjectUnknown
            else { return tapFailed }
            tapID = tap
            tapUID = description.uuid.uuidString
            let status = startDevice()
            if status != ok {
                destroyTap()
                return status
            }
            listenForOutputChanges()
            return ok
        }
    }

    func stop() {
        control.sync {
            stopped = true
            if let listener = outputListener {
                var address = defaultOutputAddress
                AudioObjectRemovePropertyListenerBlock(
                    AudioObjectID(kAudioObjectSystemObject), &address, control, listener)
                outputListener = nil
            }
            stopDevice()
            destroyTap()
        }
        // Let an IOProc call already queued finish before the caller frees
        // the context.
        ioQueue.sync {}
    }

    /// Builds the aggregate device around the current default output, which
    /// clocks the tap, and starts reading from it.
    private func startDevice() -> Int32 {
        guard let outputUID = defaultOutputUID() else { return aggregateFailed }
        let description: [String: Any] = [
            kAudioAggregateDeviceNameKey: "Murmur meeting audio",
            kAudioAggregateDeviceUIDKey: UUID().uuidString,
            kAudioAggregateDeviceMainSubDeviceKey: outputUID,
            kAudioAggregateDeviceIsPrivateKey: true,
            kAudioAggregateDeviceIsStackedKey: false,
            kAudioAggregateDeviceTapAutoStartKey: true,
            kAudioAggregateDeviceSubDeviceListKey: [[kAudioSubDeviceUIDKey: outputUID]],
            kAudioAggregateDeviceTapListKey: [
                [kAudioSubTapDriftCompensationKey: true, kAudioSubTapUIDKey: tapUID],
            ],
        ]
        var aggregate = AudioObjectID(kAudioObjectUnknown)
        guard AudioHardwareCreateAggregateDevice(description as CFDictionary, &aggregate) == noErr
        else { return aggregateFailed }
        aggregateID = aggregate

        guard let format = tapFormat(),
              format.mFormatID == kAudioFormatLinearPCM,
              format.mFormatFlags & kAudioFormatFlagIsFloat != 0,
              format.mBitsPerChannel == 32,
              format.mChannelsPerFrame > 0
        else {
            stopDevice()
            return formatUnsupported
        }
        let channels = Int(format.mChannelsPerFrame)
        let interleaved = format.mFormatFlags & kAudioFormatFlagIsNonInterleaved == 0
        let sampleRate = format.mSampleRate

        var proc: AudioDeviceIOProcID?
        let status = AudioDeviceCreateIOProcIDWithBlock(&proc, aggregate, ioQueue) {
            [weak self] _, input, _, _, _ in
            self?.deliver(input, channels: channels, interleaved: interleaved, sampleRate: sampleRate)
        }
        guard status == noErr, let proc else {
            stopDevice()
            return deviceFailed
        }
        procID = proc
        guard AudioDeviceStart(aggregate, proc) == noErr else {
            stopDevice()
            return deviceFailed
        }
        return ok
    }

    private func stopDevice() {
        if let proc = procID {
            AudioDeviceStop(aggregateID, proc)
            AudioDeviceDestroyIOProcID(aggregateID, proc)
            procID = nil
        }
        if aggregateID != kAudioObjectUnknown {
            AudioHardwareDestroyAggregateDevice(aggregateID)
            aggregateID = AudioObjectID(kAudioObjectUnknown)
        }
    }

    private func destroyTap() {
        if tapID != kAudioObjectUnknown {
            AudioHardwareDestroyProcessTap(tapID)
            tapID = AudioObjectID(kAudioObjectUnknown)
        }
    }

    /// Headphones plugged in or a Bluetooth device connected: the global tap
    /// keeps hearing everything, but the aggregate device is rebuilt around
    /// the new output. Rust fills the short gap with silence.
    private func listenForOutputChanges() {
        let listener: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
            guard let self, !self.stopped else { return }
            self.stopDevice()
            _ = self.startDevice()
        }
        var address = defaultOutputAddress
        if AudioObjectAddPropertyListenerBlock(
            AudioObjectID(kAudioObjectSystemObject), &address, control, listener) == noErr
        {
            outputListener = listener
        }
    }

    /// Mixes the tap's channels down to mono. The aggregate also carries the
    /// output device's own input streams when it has any (a headset), and
    /// the tap's streams come after them, so the tap is read from the end.
    private func deliver(
        _ input: UnsafePointer<AudioBufferList>,
        channels: Int,
        interleaved: Bool,
        sampleRate: Double
    ) {
        let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: input))
        let tapBuffers = interleaved ? 1 : channels
        guard buffers.count >= tapBuffers else { return }
        let tap = buffers.suffix(tapBuffers)
        guard let first = tap.first else { return }
        let samplesPerBuffer = Int(first.mDataByteSize) / MemoryLayout<Float>.size
        let frames = interleaved ? samplesPerBuffer / channels : samplesPerBuffer
        guard frames > 0 else { return }

        if mono.count < frames { mono = [Float](repeating: 0, count: frames) }
        let scale = 1 / Float(channels)
        if interleaved {
            guard let data = first.mData?.assumingMemoryBound(to: Float.self) else { return }
            for frame in 0..<frames {
                var sum: Float = 0
                for channel in 0..<channels { sum += data[frame * channels + channel] }
                mono[frame] = sum * scale
            }
        } else {
            for frame in 0..<frames { mono[frame] = 0 }
            for buffer in tap {
                guard let data = buffer.mData?.assumingMemoryBound(to: Float.self),
                      Int(buffer.mDataByteSize) / MemoryLayout<Float>.size >= frames
                else { return }
                for frame in 0..<frames { mono[frame] += data[frame] * scale }
            }
        }
        mono.withUnsafeBufferPointer { pointer in
            callback(context, pointer.baseAddress, Int32(frames), sampleRate)
        }
    }

    private func tapFormat() -> AudioStreamBasicDescription? {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioTapPropertyFormat,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain)
        var format = AudioStreamBasicDescription()
        var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
        guard AudioObjectGetPropertyData(tapID, &address, 0, nil, &size, &format) == noErr
        else { return nil }
        return format
    }
}

private var defaultOutputAddress = AudioObjectPropertyAddress(
    mSelector: kAudioHardwarePropertyDefaultOutputDevice,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain)

private func defaultOutputUID() -> String? {
    var address = defaultOutputAddress
    var device = AudioObjectID(kAudioObjectUnknown)
    var size = UInt32(MemoryLayout<AudioObjectID>.size)
    guard AudioObjectGetPropertyData(
        AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device) == noErr,
        device != kAudioObjectUnknown
    else { return nil }

    var uidAddress = AudioObjectPropertyAddress(
        mSelector: kAudioDevicePropertyDeviceUID,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain)
    var uid: Unmanaged<CFString>?
    size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
    guard AudioObjectGetPropertyData(device, &uidAddress, 0, nil, &size, &uid) == noErr,
          let uid
    else { return nil }
    return uid.takeRetainedValue() as String
}

/// Murmur's own audio object, so its sounds stay out of the meeting. Empty
/// when Murmur has not played anything yet and so has no object.
private func ownProcessObjects() -> [AudioObjectID] {
    var address = AudioObjectPropertyAddress(
        mSelector: kAudioHardwarePropertyTranslatePIDToProcessObject,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain)
    var pid = getpid()
    var object = AudioObjectID(kAudioObjectUnknown)
    var size = UInt32(MemoryLayout<AudioObjectID>.size)
    let status = AudioObjectGetPropertyData(
        AudioObjectID(kAudioObjectSystemObject), &address,
        UInt32(MemoryLayout<pid_t>.size), &pid, &size, &object)
    return status == noErr && object != kAudioObjectUnknown ? [object] : []
}
