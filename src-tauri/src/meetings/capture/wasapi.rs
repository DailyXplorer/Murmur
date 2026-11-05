//! The remote participants' track on Windows: everything the PC plays,
//! captured with WASAPI loopback.
//!
//! The preferred stream is a process loopback that leaves Murmur's own
//! process tree out, so its chimes stay off the track. Where Windows cannot
//! activate it (before Windows 10 build 20348, or the activation fails) the
//! default output's classic loopback records everything, Murmur's sounds
//! included. Either stream follows the default output: when it changes, or
//! the device goes away, the stream is built again, and the track keeps the
//! gap as silence (`TrackWriter` places every packet by its capture time).
//!
//! Nothing here needs a permission: Windows has no consent for loopback.

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use log::{info, warn};
use windows::core::{implement, Interface, Ref, Result as WinResult, GUID, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, E_FAIL, HANDLE, PROPERTYKEY};
use windows::Win32::Media::Audio::{
    eConsole, eRender, ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    PKEY_AudioEndpoint_FormFactor, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, BLOB, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::Variant::{VT_BLOB, VT_LPWSTR, VT_UI4};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

use super::loopback_logic::{
    downmix_to_mono, is_float32, route_facts, units_from_ticks, ClockAnchor, DeviceWatch, Step,
    WATCH_INTERVAL,
};
use super::route::RouteFacts;
use super::Feeder;

/// `PKEY_Device_EnumeratorName`: the bus that enumerated the device.
const ENUMERATOR_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 24,
};

const EXTENSIBLE_TAG: u16 = 0xFFFE;
const FLOAT_TAG: u16 = 3;
/// What the process loopback is asked for: it has no mix format to read.
const PROCESS_LOOPBACK_RATE: u32 = 48_000;
const PROCESS_LOOPBACK_CHANNELS: u16 = 2;
/// Longest wait for the stream's event, so a stop or a device change is seen
/// within it while nothing plays and no packet arrives.
const EVENT_WAIT_MS: u32 = 100;
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(5);
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// COM on the calling thread, balanced on drop. A thread that already chose
/// another apartment keeps it (and its COM), and releases nothing.
struct Com {
    owned: bool,
}

impl Com {
    fn enter() -> Self {
        // SAFETY: plain COM initialisation of this thread.
        let owned = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        Self { owned }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: balances the successful CoInitializeEx in `enter`.
            unsafe { CoUninitialize() };
        }
    }
}

/// An auto-reset event the stream signals when a packet is ready, closed on drop.
struct Event(HANDLE);

impl Event {
    fn new() -> WinResult<Self> {
        // SAFETY: an unnamed auto-reset event with default security.
        unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.map(Self)
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateEventW and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn enumerator() -> WinResult<IMMDeviceEnumerator> {
    // SAFETY: COM is initialised on this thread by the caller.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

fn default_render(devices: &IMMDeviceEnumerator) -> Option<IMMDevice> {
    // SAFETY: a plain call on a live COM interface.
    unsafe { devices.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()
}

fn device_id(device: &IMMDevice) -> Option<String> {
    // SAFETY: GetId returns a CoTaskMem string, freed below once read.
    unsafe {
        let id = device.GetId().ok()?;
        let text = id.to_string().ok();
        CoTaskMemFree(Some(id.0.cast_const().cast::<c_void>()));
        text
    }
}

fn property_u32(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<u32> {
    // SAFETY: the variant is read by its tag and cleared once.
    unsafe {
        let mut value = store.GetValue(key).ok()?;
        let inner = &*value.Anonymous.Anonymous;
        let number = (inner.vt == VT_UI4).then_some(inner.Anonymous.ulVal);
        let _ = PropVariantClear(&mut value);
        number
    }
}

fn property_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    // SAFETY: the variant is read by its tag and cleared once.
    unsafe {
        let mut value = store.GetValue(key).ok()?;
        let inner = &*value.Anonymous.Anonymous;
        let text = if inner.vt == VT_LPWSTR {
            inner.Anonymous.pwszVal.to_string().ok()
        } else {
            None
        };
        let _ = PropVariantClear(&mut value);
        text
    }
}

/// The default output as `OutputRoute::of` reads it: its form factor and the
/// bus it sits on. `None` when there is no output or Windows does not say.
pub fn output_route_facts() -> Option<RouteFacts> {
    let _com = Com::enter();
    let device = default_render(&enumerator().ok()?)?;
    // SAFETY: a plain call on a live COM interface.
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }.ok()?;
    let form_factor = property_u32(&store, &PKEY_AudioEndpoint_FormFactor);
    let bus = property_string(&store, &ENUMERATOR_NAME).unwrap_or_default();
    route_facts(form_factor, &bus)
}

/// The performance counter next to `Instant::now()`, which is the same
/// counter on Windows.
fn clock_anchor() -> Option<ClockAnchor> {
    let (mut ticks, mut frequency) = (0i64, 0i64);
    // SAFETY: both pointers are to locals that outlive the calls.
    unsafe {
        QueryPerformanceCounter(&mut ticks).ok()?;
        QueryPerformanceFrequency(&mut frequency).ok()?;
    }
    Some(ClockAnchor::new(
        Instant::now(),
        units_from_ticks(ticks, frequency),
    ))
}

/// A running capture stream and what is needed to read its packets.
struct Stream {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: Event,
    channels: usize,
    rate: u32,
    anchor: Option<ClockAnchor>,
    /// The default output this stream was built on.
    device: String,
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: a plain call on a live COM interface.
        let _ = unsafe { self.client.Stop() };
    }
}

impl Stream {
    /// Hands every queued packet to the track as mono. An error means the
    /// stream is gone (a device removed gives `AUDCLNT_E_DEVICE_INVALIDATED`)
    /// and must be built again.
    fn drain(&self, feeder: &Feeder) -> WinResult<()> {
        loop {
            // SAFETY: plain calls on live COM interfaces; the buffer
            // `GetBuffer` returns holds `frames` frames of `channels`
            // 32-bit floats until `ReleaseBuffer`, and is copied before.
            let (samples, stamp) = unsafe {
                if self.capture.GetNextPacketSize()? == 0 {
                    return Ok(());
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let (mut frames, mut flags, mut position) = (0u32, 0u32, 0u64);
                self.capture.GetBuffer(
                    &mut data,
                    &mut frames,
                    &mut flags,
                    None,
                    Some(&mut position),
                )?;
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                let stamp =
                    (flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 == 0).then_some(position);
                let samples = if silent || data.is_null() {
                    vec![0.0; frames as usize]
                } else {
                    let bytes =
                        std::slice::from_raw_parts(data, frames as usize * self.channels * 4);
                    let interleaved: Vec<f32> = bytes
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        .collect();
                    downmix_to_mono(&interleaved, self.channels)
                };
                self.capture.ReleaseBuffer(frames)?;
                (samples, stamp)
            };
            if samples.is_empty() {
                return Ok(());
            }
            let now = Instant::now();
            let captured = match &self.anchor {
                Some(anchor) => anchor.packet_end(stamp, samples.len(), self.rate, now),
                None => now,
            };
            feeder.push_at(samples, self.rate, captured);
        }
    }
}

/// Starts `client` on `wave` in shared, event-driven mode.
fn begin(
    client: IAudioClient,
    wave: *const WAVEFORMATEX,
    flags: u32,
    channels: usize,
    rate: u32,
    device: String,
) -> WinResult<Stream> {
    let event = Event::new()?;
    // SAFETY: plain calls on a live COM interface; `wave` points at a valid
    // format for the duration of Initialize, which copies it.
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            flags | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            0,
            0,
            wave,
            None,
        )?;
        client.SetEventHandle(event.0)?;
    }
    // SAFETY: plain calls on a live COM interface.
    let capture: IAudioCaptureClient = unsafe { client.GetService() }?;
    // SAFETY: as above.
    unsafe { client.Start() }?;
    Ok(Stream {
        client,
        capture,
        event,
        channels,
        rate,
        anchor: clock_anchor(),
        device,
    })
}

/// Called by Windows when the activation of the process loopback finishes.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated(mpsc::Sender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> WinResult<()> {
        let _ = self.0.send(());
        Ok(())
    }
}

/// The `PROPVARIANT` that hands `params` to `ActivateAudioInterfaceAsync`. Its
/// blob points at the caller's stack, not at COM memory, so it must never be
/// cleared: `PROPVARIANT` frees a `VT_BLOB` with `CoTaskMemFree` when it drops,
/// which on this pointer corrupts the heap. `ManuallyDrop` keeps the destructor
/// from running on any path, `?` returns included.
fn loopback_variant(params: &mut AUDIOCLIENT_ACTIVATION_PARAMS) -> ManuallyDrop<PROPVARIANT> {
    let mut variant = ManuallyDrop::new(PROPVARIANT::default());
    // SAFETY: tags a VT_BLOB variant that borrows `params`; the caller keeps
    // `params` alive for as long as it uses the variant.
    unsafe {
        let inner = &mut *variant.Anonymous.Anonymous;
        inner.vt = VT_BLOB;
        inner.Anonymous.blob = BLOB {
            cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            pBlobData: std::ptr::from_mut(params).cast::<u8>(),
        };
    }
    variant
}

/// The audio client of a loopback of everything but `pid`'s process tree.
fn activate_process_loopback(pid: u32) -> WinResult<IAudioClient> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let variant = loopback_variant(&mut params);
    let (sender, receiver) = mpsc::channel();
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(sender).into();
    // SAFETY: `variant`, `params` and `handler` outlive the call.
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*variant as *const PROPVARIANT),
            &handler,
        )
    }?;
    receiver
        .recv_timeout(ACTIVATION_TIMEOUT)
        .map_err(|_| windows::core::Error::from(E_FAIL))?;
    let mut result = windows::core::HRESULT(0);
    let mut activated: Option<windows::core::IUnknown> = None;
    // SAFETY: both pointers are to locals that outlive the call.
    unsafe { operation.GetActivateResult(&mut result, &mut activated) }?;
    result.ok()?;
    activated
        .ok_or_else(|| windows::core::Error::from(E_FAIL))?
        .cast::<IAudioClient>()
}

fn open_process_loopback(pid: u32, device: String) -> WinResult<Stream> {
    let client = activate_process_loopback(pid)?;
    let block_align = PROCESS_LOOPBACK_CHANNELS * 4;
    let wave = WAVEFORMATEX {
        wFormatTag: FLOAT_TAG,
        nChannels: PROCESS_LOOPBACK_CHANNELS,
        nSamplesPerSec: PROCESS_LOOPBACK_RATE,
        nAvgBytesPerSec: PROCESS_LOOPBACK_RATE * u32::from(block_align),
        nBlockAlign: block_align,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    begin(
        client,
        &wave,
        AUDCLNT_STREAMFLAGS_LOOPBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
        usize::from(PROCESS_LOOPBACK_CHANNELS),
        PROCESS_LOOPBACK_RATE,
        device,
    )
}

fn open_classic_loopback(device: &IMMDevice, id: String) -> WinResult<Stream> {
    // SAFETY: plain calls on live COM interfaces.
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }?;
    // SAFETY: GetMixFormat returns a CoTaskMem block holding the format,
    // freed below. The extensible tail is read only when the tag says it is
    // there.
    let mix = unsafe { client.GetMixFormat() }?;
    let (wave, subformat) = unsafe {
        let wave = std::ptr::read_unaligned(mix);
        let subformat = (wave.wFormatTag == EXTENSIBLE_TAG).then(|| {
            let extensible = std::ptr::read_unaligned(mix.cast::<WAVEFORMATEXTENSIBLE>());
            let guid = extensible.SubFormat;
            guid.to_u128()
        });
        (wave, subformat)
    };
    let (tag, bits, channels, rate) = (
        wave.wFormatTag,
        wave.wBitsPerSample,
        wave.nChannels,
        wave.nSamplesPerSec,
    );
    let started = if is_float32(tag, bits, subformat) && channels > 0 {
        begin(
            client,
            mix.cast_const(),
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            usize::from(channels),
            rate,
            id,
        )
    } else {
        Err(windows::core::Error::from(E_FAIL))
    };
    // SAFETY: the block came from GetMixFormat and is freed once.
    unsafe { CoTaskMemFree(Some(mix.cast_const().cast::<c_void>())) };
    started
}

/// A stream on the default output now: the process loopback when Windows
/// allows it, else the classic loopback.
fn open(devices: &IMMDeviceEnumerator, own_pid: u32) -> Result<Stream, String> {
    let device =
        default_render(devices).ok_or_else(|| "There is no sound output device".to_string())?;
    let id = device_id(&device).ok_or_else(|| "Cannot read the sound output".to_string())?;
    match open_process_loopback(own_pid, id.clone()) {
        Ok(stream) => {
            info!("System audio: process loopback");
            return Ok(stream);
        }
        Err(error) => info!(
            "System audio: process loopback unavailable ({:#x}), using classic loopback",
            error.code().0
        ),
    }
    match open_classic_loopback(&device, id) {
        Ok(stream) => {
            info!("System audio: classic loopback");
            Ok(stream)
        }
        Err(error) => Err(format!(
            "Cannot capture system audio ({:#x})",
            error.code().0
        )),
    }
}

/// The capture thread: builds the stream, reads it, and builds it again when
/// the default output changes or the stream stops.
fn run(
    feeder: &Feeder,
    stop: &AtomicBool,
    interrupted: &AtomicBool,
    ready: &mpsc::Sender<Result<(), String>>,
) {
    let _com = Com::enter();
    let own_pid = std::process::id();
    let devices = match enumerator() {
        Ok(devices) => devices,
        Err(error) => {
            let _ = ready.send(Err(format!(
                "Cannot reach the sound system ({:#x})",
                error.code().0
            )));
            return;
        }
    };
    let first = match open(&devices, own_pid) {
        Ok(stream) => stream,
        Err(message) => {
            let _ = ready.send(Err(message));
            return;
        }
    };
    let mut watch = DeviceWatch::following(first.device.clone());
    let mut stream = Some(first);
    let _ = ready.send(Ok(()));

    let mut last_look = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        let mut delivering = true;
        match &stream {
            Some(running) => {
                // SAFETY: waits on an event the stream owns.
                let _ = unsafe { WaitForSingleObject(running.event.0, EVENT_WAIT_MS) };
                delivering = running.drain(feeder).is_ok();
            }
            None => std::thread::sleep(Duration::from_millis(u64::from(EVENT_WAIT_MS))),
        }
        let now = Instant::now();
        if delivering && now.saturating_duration_since(last_look) < WATCH_INTERVAL {
            continue;
        }
        last_look = now;
        let default = default_render(&devices).and_then(|device| device_id(&device));
        if watch.tick(now, default.as_deref(), delivering) == Step::Rebuild {
            // Stops and releases the old stream before the new one starts.
            stream = None;
            match open(&devices, own_pid) {
                Ok(rebuilt) => {
                    watch.built(rebuilt.device.clone());
                    stream = Some(rebuilt);
                }
                Err(_) => warn!("System audio: the output could not be captured again yet"),
            }
        }
        interrupted.store(watch.interrupted(now), Ordering::SeqCst);
    }
    if let Some(running) = &stream {
        let _ = running.drain(feeder);
    }
}

/// System audio capture on its own thread, until dropped.
pub struct LoopbackCapture {
    stop: Arc<AtomicBool>,
    interrupted: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LoopbackCapture {
    /// Returns once the first stream runs, or says why it cannot.
    pub fn start(feeder: Feeder) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let interrupted = Arc::new(AtomicBool::new(false));
        let (ready, started) = mpsc::channel();
        let thread = {
            let (stop, interrupted) = (Arc::clone(&stop), Arc::clone(&interrupted));
            std::thread::Builder::new()
                .name("meeting-system-audio".into())
                .spawn(move || run(&feeder, &stop, &interrupted, &ready))
                .map_err(|error| format!("Cannot start system audio capture: {error}"))?
        };
        let capture = Self {
            stop,
            interrupted,
            thread: Some(thread),
        };
        match started.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(capture),
            Ok(Err(message)) => Err(message),
            Err(_) => Err("System audio capture did not start".to_string()),
        }
    }

    /// No stream could be built for a few seconds after the output changed.
    pub fn interrupted(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }
}

impl Drop for LoopbackCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::capture::TrackWriter;

    #[test]
    fn the_activation_variant_borrows_its_blob_and_is_never_freed() {
        let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: 4242,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
                },
            },
        };
        let address = std::ptr::from_mut(&mut params).cast::<u8>();
        // Dropping a plain PROPVARIANT here would hand a stack address to
        // CoTaskMemFree; the helper returns one that cannot be dropped.
        let variant = loopback_variant(&mut params);
        // SAFETY: reads the tags the helper just wrote.
        let inner = unsafe { &*variant.Anonymous.Anonymous };
        assert_eq!(inner.vt, VT_BLOB);
        // SAFETY: the variant was tagged VT_BLOB above.
        let blob = unsafe { inner.Anonymous.blob };
        assert_eq!(blob.pBlobData, address);
        assert_eq!(
            blob.cbSize as usize,
            std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>()
        );
        drop(variant);
    }

    /// Plays a 1 kHz tone on the default output for `seconds` seconds.
    fn play_tone(seconds: usize) -> Option<rodio::OutputStream> {
        let stream = rodio::OutputStreamBuilder::open_default_stream().ok()?;
        let rate = 48_000usize;
        let tone: Vec<f32> = (0..rate * seconds)
            .map(|i| (i as f32 * 1_000.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5)
            .collect();
        stream
            .mixer()
            .add(rodio::buffer::SamplesBuffer::new(1, rate as u32, tone));
        Some(stream)
    }

    /// The loudest sample of what a capture of the default output hears in
    /// about three seconds while a tone plays from this process. `None` when
    /// this machine has no output device, or `open` cannot build its stream.
    fn hear_tone(
        open: impl FnOnce(&IMMDeviceEnumerator, String) -> WinResult<Stream>,
    ) -> Option<f32> {
        let _com = Com::enter();
        let devices = enumerator().ok()?;
        let Some(id) = default_render(&devices).and_then(|device| device_id(&device)) else {
            eprintln!("skipped: this machine has no sound output device");
            return None;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system.wav");
        let origin = Instant::now();
        let writer = TrackWriter::create(&path, origin).unwrap();
        let feeder = writer.feeder();
        let stream = match open(&devices, id) {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!(
                    "skipped: the stream cannot be built ({:#x})",
                    error.code().0
                );
                return None;
            }
        };
        let Some(_playing) = play_tone(3) else {
            eprintln!("skipped: the tone cannot be played on this machine");
            return None;
        };
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(3) {
            // SAFETY: waits on an event the stream owns.
            let _ = unsafe { WaitForSingleObject(stream.event.0, EVENT_WAIT_MS) };
            stream.drain(&feeder).unwrap();
        }
        drop(stream);
        writer.finish(started.elapsed()).unwrap();
        let samples: Vec<i16> = hound::WavReader::open(&path)
            .unwrap()
            .into_samples()
            .map(Result::unwrap)
            .collect();
        let tail = &samples[samples.len() / 3..];
        Some(
            tail.iter()
                .map(|&sample| f32::from(sample).abs() / f32::from(i16::MAX))
                .fold(0.0, f32::max),
        )
    }

    #[test]
    #[ignore = "needs a sound output device: run by the meetings-audio job and by hand"]
    fn wasapi_loopback_hears_a_tone() {
        if let Some(peak) = hear_tone(|devices, id| {
            let device = default_render(devices).unwrap();
            open_classic_loopback(&device, id)
        }) {
            assert!(peak > 0.1, "the loopback track holds the tone: peak {peak}");
        }
    }

    #[test]
    #[ignore = "needs a sound output device and Windows 10 build 20348 or later"]
    fn wasapi_process_loopback_leaves_out_the_own_process() {
        if let Some(peak) = hear_tone(|_, id| open_process_loopback(std::process::id(), id)) {
            assert!(peak < 0.01, "the own tone stays off the track: peak {peak}");
        }
    }
}
