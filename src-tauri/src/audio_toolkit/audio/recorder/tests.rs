use super::{
    append_recording_frame_with_limit, is_microphone_access_denied, is_no_input_device_error,
    run_consumer, AudioChunk, AudioRecorder, Capture, Cmd, ReleaseTailRequest, StreamHealth, STALL,
    ZERO_REOPEN_EVERY,
};
use crate::audio_toolkit::release_tail::TailConfig;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[test]
fn unopened_recorder_does_not_need_reopen() {
    // No worker has been spawned yet, so there is nothing to reap. Guards
    // against inverting the "no worker" case, which would make every first
    // open() take the rebuild path.
    let recorder = AudioRecorder::new().expect("recorder");
    assert!(!recorder.needs_reopen());
}

#[test]
fn stream_error_requires_reopen() {
    let mut recorder = AudioRecorder::new().expect("recorder");
    let health = Arc::new(StreamHealth::new(Instant::now()));
    health.failed.store(true, Ordering::Relaxed);
    recorder.health = Some(health);
    assert!(recorder.needs_reopen());
}

#[test]
fn a_stream_whose_callbacks_stopped_counts_as_stalled() {
    let opened = Instant::now();
    let health = StreamHealth::new(opened);
    assert!(!health.stalled(opened + STALL), "just opened");
    assert!(
        health.stalled(opened + STALL + Duration::from_millis(1)),
        "never called back"
    );

    health.mark(opened + Duration::from_secs(10));
    assert!(!health.stalled(opened + Duration::from_secs(10) + STALL));
    assert!(health.stalled(opened + Duration::from_secs(11) + STALL));
}

#[test]
fn a_stalled_stream_requires_reopen() {
    // A device opened as the system default can go away without cpal
    // reporting anything: its callbacks just stop.
    let mut recorder = AudioRecorder::new().expect("recorder");
    let opened = Instant::now().checked_sub(STALL * 2).expect("uptime");
    recorder.health = Some(Arc::new(StreamHealth::new(opened)));
    assert!(recorder.needs_reopen());

    recorder.health.as_ref().unwrap().mark(Instant::now());
    assert!(!recorder.needs_reopen());
}

#[test]
fn a_stream_on_the_default_reopens_once_another_microphone_is_the_default() {
    let mut recorder = AudioRecorder::new().expect("recorder");
    recorder.default_name = Some("AirPods Pro".to_string());
    assert!(!recorder.left_default(|| Some("AirPods Pro".to_string())));
    // The AirPods went back in their case: macOS made the built-in
    // microphone the default.
    assert!(recorder.left_default(|| Some("MacBook Pro Microphone".to_string())));
    // No input device is left.
    assert!(recorder.left_default(|| None));
}

#[test]
fn a_stream_on_a_named_microphone_ignores_the_default() {
    let recorder = AudioRecorder::new().expect("recorder");
    assert!(!recorder.left_default(|| Some("MacBook Pro Microphone".to_string())));
}

#[test]
fn shutdown_is_processed_without_audio_samples() {
    let (sample_tx, sample_rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        run_consumer(
            48_000,
            sample_rx,
            cmd_rx,
            None,
            Arc::new(AtomicBool::new(false)),
            Instant::now(),
        );
        let _ = done_tx.send(());
    });

    cmd_tx.send(Cmd::Shutdown).expect("send shutdown");
    let stopped = done_rx.recv_timeout(Duration::from_secs(1));

    // Unblocks a consumer stuck waiting for a sample, so a failing test
    // still exits cleanly.
    drop(sample_tx);
    worker.join().expect("join consumer");
    assert!(stopped.is_ok(), "shutdown waited for an audio sample");
}

/// A consumer at the target rate, fed by the test in place of cpal's
/// callback.
fn spawn_consumer() -> (
    mpsc::Sender<AudioChunk>,
    mpsc::Sender<Cmd>,
    Arc<AtomicBool>,
    thread::JoinHandle<()>,
) {
    let (sample_tx, sample_rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let consumer_flag = Arc::clone(&stop_flag);
    let worker = thread::spawn(move || {
        run_consumer(
            16_000,
            sample_rx,
            cmd_rx,
            None,
            consumer_flag,
            Instant::now(),
        );
    });
    (sample_tx, cmd_tx, stop_flag, worker)
}

/// Starts a recording and returns once its first chunk is captured.
fn record_one_chunk(sample_tx: &mpsc::Sender<AudioChunk>, cmd_tx: &mpsc::Sender<Cmd>) {
    let (ready_tx, ready_rx) = mpsc::channel();
    cmd_tx
        .send(Cmd::Start(Instant::now(), ready_tx))
        .expect("start");
    sample_tx
        .send(AudioChunk::Samples(vec![0.1; 4_800]))
        .expect("chunk");
    ready_rx.recv().expect("the chunk was captured");
}

fn stop(cmd_tx: &mpsc::Sender<Cmd>) -> mpsc::Receiver<Capture> {
    let (reply_tx, reply_rx) = mpsc::channel();
    cmd_tx.send(Cmd::Stop(reply_tx, None)).expect("stop");
    reply_rx
}

#[test]
fn a_microphone_lost_mid_recording_is_reported_with_the_audio_before_it() {
    let (sample_tx, cmd_tx, _, worker) = spawn_consumer();
    record_one_chunk(&sample_tx, &cmd_tx);

    // The device went away: its callback never runs again, so the stop
    // gets no EndOfStream.
    let capture = stop(&cmd_tx).recv().expect("reply");

    assert!(capture.mic_lost, "the stop says the microphone was lost");
    assert_eq!(capture.samples.len(), 4_800, "the audio before it is kept");
    cmd_tx.send(Cmd::Shutdown).expect("shutdown");
    drop(sample_tx);
    worker.join().expect("join consumer");
}

#[test]
fn a_recording_stopped_by_the_user_reports_no_lost_microphone() {
    let (sample_tx, cmd_tx, stop_flag, worker) = spawn_consumer();
    record_one_chunk(&sample_tx, &cmd_tx);

    let reply = stop(&cmd_tx);
    // As cpal's callback does: the next call after the stop flag is set
    // ends the stream.
    while !stop_flag.load(Ordering::Relaxed) {
        thread::yield_now();
    }
    sample_tx.send(AudioChunk::EndOfStream).expect("end");
    let capture = reply.recv().expect("reply");

    assert!(!capture.mic_lost);
    assert_eq!(capture.samples.len(), 4_800);
    cmd_tx.send(Cmd::Shutdown).expect("shutdown");
    drop(sample_tx);
    worker.join().expect("join consumer");
}

/// A consumer that found a recording's start and stop both queued when
/// it first ran.
struct Stopping {
    sample_tx: mpsc::Sender<AudioChunk>,
    cmd_tx: mpsc::Sender<Cmd>,
    reply: mpsc::Receiver<Capture>,
    stop_flag: Arc<AtomicBool>,
    worker: thread::JoinHandle<()>,
}

impl Stopping {
    /// `in_flight` is already sent by the callback when the consumer
    /// starts.
    fn with(in_flight: Option<Vec<f32>>) -> Self {
        let (sample_tx, sample_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ready_tx, _ready_rx) = mpsc::channel();
        cmd_tx
            .send(Cmd::Start(Instant::now(), ready_tx))
            .expect("start");
        let reply = stop(&cmd_tx);
        if let Some(samples) = in_flight {
            sample_tx.send(AudioChunk::Samples(samples)).expect("chunk");
        }
        let stop_flag = Arc::new(AtomicBool::new(false));
        let consumer_flag = Arc::clone(&stop_flag);
        let worker = thread::spawn(move || {
            run_consumer(
                16_000,
                sample_rx,
                cmd_rx,
                None,
                consumer_flag,
                Instant::now(),
            );
        });
        Self {
            sample_tx,
            cmd_tx,
            reply,
            stop_flag,
            worker,
        }
    }

    fn shut_down(self) {
        self.cmd_tx.send(Cmd::Shutdown).expect("shutdown");
        drop(self.sample_tx);
        self.worker.join().expect("join consumer");
    }
}

#[test]
fn a_microphone_lost_after_a_first_chunk_the_stop_took_is_reported() {
    let consumer = Stopping::with(Some(vec![0.1; 4_800]));

    // The stop takes the stream's first chunk, then the device goes
    // away: no EndOfStream follows.
    let capture = consumer.reply.recv().expect("reply");

    assert_eq!(capture.samples.len(), 4_800);
    assert!(capture.mic_lost, "the stop says the microphone was lost");
    consumer.shut_down();
}

#[test]
fn a_microphone_lost_after_a_first_chunk_drained_by_the_stop_is_reported() {
    let consumer = Stopping::with(None);

    // The stop waits for the stream's end; the first chunk comes in
    // meanwhile, then the device goes away.
    while !consumer.stop_flag.load(Ordering::Relaxed) {
        thread::yield_now();
    }
    consumer
        .sample_tx
        .send(AudioChunk::Samples(vec![0.1; 4_800]))
        .expect("chunk");
    let capture = consumer.reply.recv().expect("reply");

    assert_eq!(capture.samples.len(), 4_800);
    assert!(capture.mic_lost, "the stop says the microphone was lost");
    consumer.shut_down();
}

const CHUNK: usize = 320;

/// What cpal's callback does once the stop flag is up: end the stream.
/// Until then it delivers a 20 ms chunk at `level(index)`.
fn deliver_until_stopped(
    sample_tx: mpsc::Sender<AudioChunk>,
    stop_flag: Arc<AtomicBool>,
    level: impl Fn(usize) -> f32 + Send + 'static,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut index = 0;
        while !stop_flag.load(Ordering::Relaxed) {
            if sample_tx
                .send(AudioChunk::Samples(vec![level(index); CHUNK]))
                .is_err()
            {
                return;
            }
            index += 1;
            thread::sleep(Duration::from_millis(1));
        }
        let _ = sample_tx.send(AudioChunk::EndOfStream);
    })
}

fn tail_request(abort: bool) -> ReleaseTailRequest {
    ReleaseTailRequest {
        config: TailConfig::new(0, 0),
        abort: Arc::new(move || abort),
    }
}

/// Records a 300 ms clip of speech, releases with a tail, and returns how
/// many 20 ms chunks `level` delivered into the clip after the release.
fn tail_chunks(
    request: ReleaseTailRequest,
    first_chunk: usize,
    level: impl Fn(usize) -> f32 + Send + 'static,
) -> usize {
    let (sample_tx, cmd_tx, stop_flag, worker) = spawn_consumer();
    let (ready_tx, ready_rx) = mpsc::channel();
    cmd_tx
        .send(Cmd::Start(Instant::now(), ready_tx))
        .expect("start");
    sample_tx
        .send(AudioChunk::Samples(vec![0.1; first_chunk]))
        .expect("chunk");
    ready_rx.recv().expect("the chunk was captured");

    let (reply_tx, reply_rx) = mpsc::channel();
    cmd_tx
        .send(Cmd::Stop(reply_tx, Some(request)))
        .expect("stop");
    // The consumer takes the stop within its 50 ms poll, then waits for
    // the tail's audio.
    thread::sleep(Duration::from_millis(120));
    let feeder = deliver_until_stopped(sample_tx.clone(), stop_flag, level);
    let capture = reply_rx.recv().expect("reply");
    feeder.join().expect("join feeder");

    cmd_tx.send(Cmd::Shutdown).expect("shutdown");
    drop(sample_tx);
    worker.join().expect("join consumer");
    assert!(!capture.mic_lost);
    (capture.samples.len() - first_chunk) / CHUNK
}

const SPEECH_SAMPLES: usize = 4_800;

#[test]
fn a_silent_user_is_recorded_for_the_base_tail_only() {
    // 200 ms is 10 chunks; a chunk or two may be in flight at the end.
    let chunks = tail_chunks(tail_request(false), SPEECH_SAMPLES, |_| 0.0);
    assert!((10..=13).contains(&chunks), "{chunks} chunks");
}

#[test]
fn a_release_after_a_pause_ends_the_tail_early() {
    let (sample_tx, cmd_tx, stop_flag, worker) = spawn_consumer();
    let (ready_tx, ready_rx) = mpsc::channel();
    cmd_tx
        .send(Cmd::Start(Instant::now(), ready_tx))
        .expect("start");
    // 300 ms of speech, then 200 ms of silence before the release.
    let mut clip = vec![0.1_f32; SPEECH_SAMPLES];
    clip.extend(vec![0.0_f32; 3_200]);
    sample_tx.send(AudioChunk::Samples(clip)).expect("chunk");
    ready_rx.recv().expect("the chunk was captured");

    let (reply_tx, reply_rx) = mpsc::channel();
    cmd_tx
        .send(Cmd::Stop(reply_tx, Some(tail_request(false))))
        .expect("stop");
    thread::sleep(Duration::from_millis(120));
    let feeder = deliver_until_stopped(sample_tx.clone(), stop_flag, |_| 0.0);
    let capture = reply_rx.recv().expect("reply");
    feeder.join().expect("join feeder");
    cmd_tx.send(Cmd::Shutdown).expect("shutdown");
    drop(sample_tx);
    worker.join().expect("join consumer");

    // 80 ms is 4 chunks; a chunk or two may be in flight at the end.
    let chunks = (capture.samples.len() - SPEECH_SAMPLES - 3_200) / CHUNK;
    assert!((4..=7).contains(&chunks), "{chunks} chunks");
}

#[test]
fn a_user_still_speaking_is_recorded_until_quiet() {
    // 10 chunks of speech, then quiet: 200 ms of speech plus 160 ms.
    let chunks = tail_chunks(tail_request(false), SPEECH_SAMPLES, |index| {
        if index < 10 {
            0.1
        } else {
            0.0
        }
    });
    assert!((18..=21).contains(&chunks), "{chunks} chunks");
}

#[test]
fn a_user_who_never_stops_is_cut_at_the_cap() {
    let chunks = tail_chunks(tail_request(false), SPEECH_SAMPLES, |_| 0.1);
    assert!((30..=33).contains(&chunks), "{chunks} chunks");
}

#[test]
fn an_aborted_tail_ends_at_once() {
    let chunks = tail_chunks(tail_request(true), SPEECH_SAMPLES, |_| 0.1);
    assert!(chunks <= 3, "{chunks} chunks");
}

#[test]
fn a_tap_gets_no_tail() {
    // 100 ms of audio is not a dictation.
    let chunks = tail_chunks(tail_request(false), 1_600, |_| 0.1);
    assert!(chunks <= 3, "{chunks} chunks");
}

/// An open recorder wired to a consumer, for what a stop does to it.
fn recorder_with_consumer() -> (
    AudioRecorder,
    mpsc::Sender<AudioChunk>,
    Arc<AtomicBool>,
    thread::JoinHandle<()>,
) {
    let (sample_tx, cmd_tx, stop_flag, worker) = spawn_consumer();
    let mut recorder = AudioRecorder::new().expect("recorder");
    recorder.cmd_tx = Some(cmd_tx);
    recorder.health = Some(Arc::new(StreamHealth::new(Instant::now())));
    (recorder, sample_tx, stop_flag, worker)
}

/// Records `level` for 100 ms through a real stop and returns whether the
/// stream then needs rebuilding.
fn needs_reopen_after_clip_of(level: f32) -> bool {
    let (recorder, sample_tx, stop_flag, worker) = recorder_with_consumer();
    let ready = recorder.start().expect("start");
    sample_tx
        .send(AudioChunk::Samples(vec![level; 1_600]))
        .expect("chunk");
    ready.recv().expect("the chunk was captured");
    let feeder = deliver_until_stopped(sample_tx.clone(), stop_flag, move |_| level);
    let capture = recorder.stop().expect("stop");
    feeder.join().expect("join feeder");
    assert!(!capture.samples.is_empty());

    let needs_reopen = recorder.needs_reopen();
    recorder
        .cmd_tx
        .as_ref()
        .unwrap()
        .send(Cmd::Shutdown)
        .expect("shutdown");
    drop(sample_tx);
    worker.join().expect("join consumer");
    needs_reopen
}

#[test]
fn a_clip_of_only_zeros_asks_for_the_stream_to_be_reopened() {
    assert!(needs_reopen_after_clip_of(0.0));
}

#[test]
fn a_clip_with_any_signal_leaves_the_stream_alone() {
    // One least significant bit of a 16-bit microphone is not zero.
    assert!(!needs_reopen_after_clip_of(1.0 / 32_768.0));
    assert!(!needs_reopen_after_clip_of(0.1));
}

#[test]
fn zero_clips_ask_for_a_reopen_at_most_every_half_minute() {
    let zeros = Capture {
        samples: vec![0.0; 1_600],
        ..Capture::default()
    };
    let signal = Capture {
        samples: vec![0.1; 1_600],
        ..Capture::default()
    };
    let fresh = || AudioRecorder {
        health: Some(Arc::new(StreamHealth::new(Instant::now()))),
        ..AudioRecorder::new().expect("recorder")
    };
    let start = Instant::now();
    let recorder = fresh();
    recorder.note_clip_at(&zeros, start);
    assert!(recorder.needs_reopen());

    // The rebuilt stream is still dead, as a muted input is: not again
    // at once ...
    let health = |recorder: AudioRecorder| AudioRecorder {
        health: Some(Arc::new(StreamHealth::new(Instant::now()))),
        ..recorder
    };
    let recorder = health(recorder);
    recorder.note_clip_at(&zeros, start + Duration::from_secs(10));
    assert!(!recorder.needs_reopen());
    // ... but it is tried again later, for a device still held then.
    recorder.note_clip_at(&zeros, start + ZERO_REOPEN_EVERY);
    assert!(recorder.needs_reopen());

    // A clip with signal clears the wait.
    let recorder = health(recorder);
    recorder.note_clip_at(&signal, start + Duration::from_secs(31));
    recorder.note_clip_at(&zeros, start + Duration::from_secs(32));
    assert!(recorder.needs_reopen());
}

#[test]
fn an_empty_clip_is_not_digital_silence() {
    // A stop that captured nothing says nothing about the stream.
    let recorder = AudioRecorder {
        health: Some(Arc::new(StreamHealth::new(Instant::now()))),
        ..AudioRecorder::new().expect("recorder")
    };
    recorder.note_clip(&Capture::default());
    assert!(!recorder.needs_reopen());
}

#[cfg(target_os = "windows")]
#[test]
fn a_closed_callback_channel_preserves_the_recording_and_reports_microphone_loss() {
    let (sample_tx, sample_rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let in_callback = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    let level_cb = {
        let in_callback = Arc::clone(&in_callback);
        let resume = Arc::clone(&resume);
        Arc::new(move |_: Vec<f32>| {
            in_callback.wait();
            resume.wait();
        }) as Arc<dyn Fn(Vec<f32>) + Send + Sync>
    };
    let worker = thread::spawn(move || {
        run_consumer(
            16_000,
            sample_rx,
            cmd_rx,
            Some(level_cb),
            Arc::new(AtomicBool::new(false)),
            Instant::now(),
        );
    });
    let (ready_tx, _ready_rx) = mpsc::channel();
    cmd_tx
        .send(Cmd::Start(Instant::now(), ready_tx))
        .expect("start");
    sample_tx
        .send(AudioChunk::Samples(vec![0.1; 4_800]))
        .expect("chunk");
    in_callback.wait();
    // WASAPI drops the data callback when its stream thread exits on an
    // error. Queue Stop while this chunk is still being processed, then
    // close its producer before the consumer receives again.
    let reply = stop(&cmd_tx);
    drop(sample_tx);
    resume.wait();
    let capture = reply.recv();
    let _ = cmd_tx.send(Cmd::Shutdown);
    worker.join().expect("join consumer");
    let capture = capture.expect("the closed callback must not discard the recording");
    assert_eq!(capture.samples.len(), 4_800);
    assert!(capture.mic_lost);
}

#[test]
fn a_microphone_cpal_reported_gone_before_any_audio_is_reported_lost() {
    let (sample_tx, cmd_tx, _, worker) = spawn_consumer();
    let mut recorder = AudioRecorder::new().expect("recorder");
    recorder.cmd_tx = Some(cmd_tx);
    let health = Arc::new(StreamHealth::new(Instant::now()));
    recorder.health = Some(Arc::clone(&health));

    // A Bluetooth microphone can take seconds to deliver its first
    // chunk: a stop before it is no sign of a lost device.
    recorder.start().expect("start");
    let slow = recorder.stop().expect("stop");
    assert!(slow.samples.is_empty());
    assert!(!slow.mic_lost, "a slow microphone is not a lost one");

    // cpal's disconnect listener fired before the first chunk.
    recorder.start().expect("start");
    health.failed.store(true, Ordering::Relaxed);
    let lost = recorder.stop().expect("stop");
    assert!(lost.samples.is_empty());
    assert!(lost.mic_lost);

    recorder.close().expect("close");
    drop(sample_tx);
    worker.join().expect("join consumer");
}

#[test]
fn detects_access_is_denied() {
    assert!(is_microphone_access_denied("Access is denied"));
}

#[test]
fn detects_permission_denied() {
    assert!(is_microphone_access_denied("permission denied"));
}

#[test]
fn does_not_match_unrelated_errors() {
    assert!(!is_microphone_access_denied("device not found"));
}

#[test]
fn detects_no_input_device() {
    assert!(is_no_input_device_error("No input device found"));
}

#[test]
fn detects_coreaudio_config_error() {
    assert!(is_no_input_device_error(
        "Failed to fetch preferred config: A backend-specific error has occurred: An unknown error unknown to the coreaudio-rs API occurred"
    ));
}

#[test]
fn does_not_match_other_errors_for_no_device() {
    assert!(!is_no_input_device_error("permission denied"));
    assert!(!is_no_input_device_error("device not found"));
}

#[test]
fn recording_frames_never_exceed_the_sample_limit() {
    let mut samples = vec![1.0, 2.0];
    assert!(append_recording_frame_with_limit(
        &[3.0, 4.0],
        true,
        &mut samples,
        3
    ));
    assert_eq!(samples, vec![1.0, 2.0, 3.0]);

    assert!(append_recording_frame_with_limit(
        &[5.0],
        true,
        &mut samples,
        3
    ));
    assert_eq!(samples, vec![1.0, 2.0, 3.0]);
}
