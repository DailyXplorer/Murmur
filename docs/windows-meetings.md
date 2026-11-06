# Meetings on Windows

## Product rule

**Meetings on Windows must reach the SAME quality as on macOS; reuse `main`'s code wherever it is platform-neutral.**

Meetings stay hidden on the Windows beta (no sidebar entry, no tray item, `meetings_enabled` forced off) until every row below is delivered and the enabling change lands. The pieces land first as tested code that no screen reaches.

## Parity checklist

Status: `Not yet` is not built. `Built, hidden` is built and tested but no screen reaches it.

| #   | Feature on macOS                                                                   | Windows                                                            | Status                                                                                                     |
| --- | ---------------------------------------------------------------------------------- | ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| 1   | Start/stop a meeting from the tray item, the shortcut or the Meetings page         | Code on `beta`, enabled last                                       | Not yet                                                                                                    |
| 2   | Record the microphone ("Me")                                                       | The microphone path of dictation (cpal, WASAPI)                    | Not yet                                                                                                    |
| 3   | Record system or app audio ("Others"), Murmur's own sounds left out                | WASAPI process loopback, classic loopback fallback                 | Built, hidden (`meetings/capture/wasapi.rs`)                                                               |
| 4   | Capture mode: both, system only, microphone only                                   | Settings exist                                                     | Not yet                                                                                                    |
| 5   | Dropped-audio warning, low disk, 3 h cap, crash recovery, sleep notice, keep-awake | Code on `beta`; keep-awake is new                                  | Partly: keep-awake built, hidden (`meetings/power_windows.rs`). The rest Not yet                           |
| 6   | Transcript through Codex, offline fallback to the local model                      | Code on `beta`                                                     | Not yet                                                                                                    |
| 7   | Speaker separation, rename speakers                                                | In-process analyzer, pyannote Community-1 (any number of speakers) | Built, hidden (`meetings/analysis_cpp.rs`, `meetings/diarize/`)                                            |
| 8   | Word-aligned merge, echo removal, headphone awareness                              | Neutral code; the output route is new                              | Partly: route built, hidden (`loopback_logic::route_facts`). The rest Not yet                              |
| 9   | AI title and summary, retry                                                        | Code on `beta`                                                     | Not yet                                                                                                    |
| 10  | Compressed audio (`audio.m4a`), re-transcribe, retention, delete audio             | AAC through Media Foundation                                       | Built, hidden (`meetings/archive/`)                                                                        |
| 11  | Export and copy Markdown (save dialog)                                             | `IFileSaveDialog`                                                  | Built, hidden (`meetings/save_panel.rs`); copy Not yet                                                     |
| 12  | Meetings list, detail, search, delete                                              | UI and FTS5 search on `beta`                                       | Not yet                                                                                                    |
| 13  | Live transcript, "Catching up", paste-new-text shortcut                            | Port of the live transcript                                        | Built, hidden (`meetings/live/`, `engine/local/cpu.rs`)                                                    |
| 14  | Call detected, prompt to record (Zoom, Teams, Slack, Webex, Discord, browser Meet) | Microphone consent store, shared state machine                     | Built, hidden (`meetings/call_probe_windows.rs`)                                                           |
| 15  | "Call ended" prompt                                                                | Port                                                               | Built, hidden (`meetings/call_watch.rs`)                                                                   |
| 16  | Transcript-ready pill, open from the notice                                        | Port                                                               | Built, hidden (`overlay/meeting_notice.rs`)                                                                |
| 17  | MCP meeting tools                                                                  | Flip the Windows switch                                            | Built, hidden (`mcp/`, the same five tools). Settings copy: at enable                                      |
| 18  | Models fetched with a meeting unless declined, licences dialog                     | New speaker model source                                           | Partly: speaker model source (ONNX files) and licences built, hidden (`pinned::WINDOWS`). The rest Not yet |
| 19  | Settings: enable, call prompt, live transcript, retention, capture mode            | Port with Windows copy                                             | Partly: the call prompt row has its Windows text. The rest Not yet                                         |
| 20  | FaceTime calls                                                                     | Not applicable on Windows                                          | Not applicable                                                                                             |
| 21  | Windows 10 before build 20348: system audio includes Murmur's own sounds           | Known limit, see below                                             | Documented                                                                                                 |

## System audio on Windows

- **Where.** `meetings/capture/wasapi.rs` (the Windows calls), `meetings/capture/loopback_logic.rs` (everything that needs no Windows API, tested on any platform), `meetings/capture/system.rs` (the shared entry points), `meetings/power_windows.rs` (keep-awake).
- **Two streams.** The preferred one is a process loopback of everything the PC plays except Murmur's own process tree (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`, exclude mode), so the start and stop chimes stay off the track. Where Windows cannot activate it (before Windows 10 build 20348, or an activation error), the classic loopback of the default output records everything, Murmur's sounds included. The log says which one runs, with fixed words.
- **Same clock as the microphone.** Each packet is stamped from the packet's performance-counter position, mapped onto `Instant` (the same counter on Windows), and handed to the same `Feeder::push_at` as on macOS. `TrackWriter` places it by that time and keeps the silence while nothing plays: WASAPI sends no packet then.
- **Follows the default output.** The capture looks at the default output once a second, and builds the stream again when it changed, the stream stopped delivering or the device was removed (`AUDCLNT_E_DEVICE_INVALIDATED` is an ordinary error there, never a panic). After 5 s without a stream the track reports itself interrupted, which the coordinator turns into the usual notice.
- **No permission.** Windows has no consent for loopback capture. `permission()` is granted, `system_audio_denied()` is false, and there is nothing to open in Settings.
- **Output route.** The default output's form factor (`PKEY_AudioEndpoint_FormFactor`) and bus (`PKEY_Device_EnumeratorName`) become the `RouteFacts` that `route::OutputRoute::of` reads on macOS. Headphones and headsets are headphones, also on Bluetooth or USB. Speakers, displays, optical and line outputs, and Bluetooth or USB speakers are speakers. Anything else is unknown, which counts as speakers.
- **Silent system track.** The macOS check for a denied System Audio Recording permission (zeros while another app played) has no Windows cause, so `SystemCapture::finish` reports "no" there and the flag stays off.
- **Keep awake.** `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` from a thread the guard owns, cleared on drop. A closed lid still sleeps the PC.

## Call detection on Windows

- **Where.** `meetings/call_probe_windows.rs` reads the system (and holds the parsers and decisions, which run on any platform in the tests), `meetings/call_watch.rs` polls it once a second through the `Probe` trait, `meetings/calls.rs` is the state machine and the app tables that macOS shares, `overlay.rs` builds the prompt window.
- **Signal.** The microphone consent store, `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone`. A packaged app has a subkey named by its package family name. A desktop app has one under `NonPackaged`, named by its executable's path with `#` for `\`. Each holds `LastUsedTimeStart` and `LastUsedTimeStop`: an app is on the microphone while Start is set and Stop is 0. It works from Windows 10 1903, needs no administrator rights and no COM. While nothing holds the microphone, a poll reads only the registry.
- **Murmur is skipped**, by its executable's file name: its own dictation holds the microphone too.
- **Dead apps.** Windows leaves Stop at 0 when an app dies with the microphone open. A desktop app counts only while its executable is in the process list, which is read only when a desktop app is on the microphone. A packaged app has no executable to look for, so a stale entry of one stays until the app opens the microphone again.
- **Apps.** Zoom (`Zoom.exe`), Teams (`ms-teams.exe`, `Teams.exe` and the packaged `MSTeams`), Slack (`slack.exe`), Webex (`webex.exe`, `CiscoCollabHost.exe`, `webexmta.exe`, `atmgr.exe`) and Discord (`Discord.exe`) are calls as soon as they hold the microphone for 3 s. The browsers are Chrome, Edge, Brave, Firefox, Vivaldi, Opera, Chromium, Arc, Helium and Comet. The tables are `WINDOWS_EXECUTABLES` and `WINDOWS_PACKAGES` in `calls.rs`; Chrome's and Edge's Beta and Canary channels share their executable, so they count as one browser.
- **Browsers** are a call only on a meeting page. First the titles of the browser's visible windows are read (Google Meet names its tab "Meet - abc-defg-hij"). If none shows a meeting, the address bar is read through UI Automation, the toolbar only and never the page, at most every 5 s per browser and within 750 ms. The address matches a Meet code, Teams on the web, the Zoom web client, Webex and a Discord channel. A call found stays while the browser holds the microphone, so it survives a switch to another tab.
- **Not detected.** FaceTime (no Windows version), an app that is not in the tables, and a web app that is not one of the pages above.
- **Prompt and answers** are the macOS ones: "Record this meeting?" with Record and Not now, then "Call ended." with Stop and transcribe and Keep recording, each answer acting only on the prompt it answers. The panel opens beside the orb, takes clicks and never the focus, and its window exists only while a prompt shows. Every call asks, and Murmur never starts or stops a meeting by itself.
- **Setting.** `meeting_call_prompt` turns it on and off, with the macOS text minus FaceTime. The watcher runs only while `meetings_enabled` is on, which Windows keeps off for now, so nothing of this runs yet.

## Speakers and word times

After a meeting, the transcript shows who spoke (Speaker 1, 2, 3… on the
remote side, "You" on the microphone) and every word sits at the time it was
said. The pipeline after the analysis is the Mac one: `Analysis::absorb`,
`align/`, `speakers.rs` and `bleed.rs` read the same `protocol` messages.

**Where it runs.** In the app, in a background thread, with no helper
process (`meetings/analysis_cpp.rs`). Two models run on the CPU:

- **Parakeet Ultra**, the model dictation already uses, through
  transcribe.cpp with word timestamps. Each track is cut at pauses into
  pieces of at most 20 s (`engine::local::MAX_PIECE`), the pieces without
  speech are skipped, and each word is placed at its time on the track.
- **pyannote Community-1**, the speaker model of the Mac, through ONNX
  Runtime, on the track `analysis::diarized_track` picks: system audio when
  the remote side was heard speaking there, else the microphone.

The decisions (pieces, times, which track, turns named `S1`, `S2`… with no
maximum in a stable order, how a failure ends the run) are in
`meetings/analysis_local.rs`, which has no model in it and is tested with
fakes. `analysis_cpp.rs` holds the models, the cancel token, a `catch_unwind`
around them and a time limit that grows with the audio.

**The speaker pipeline** (`meetings/diarize/`). `main` runs the offline
diarization of FluidAudio 0.17.7 (Apache-2.0) in the Swift helper, through Core
ML. Windows runs the same pipeline, re-implemented in Rust from that source,
with the same weights and the same values (`config.rs` names each one with its
line in `OfflineDiarizerTypes.swift`; none is tuned):

| Stage                                                              | File           | Mac value kept                                                                                                                                                                   |
| ------------------------------------------------------------------ | -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows of 10 s every 2 s; 589 frames, 3 local speakers (powerset) | `segmentation` | `Segmentation.community`; a network, ONNX Runtime                                                                                                                                |
| 80 mel bins, 25 ms / 10 ms, Hamming, per-window mean               | `fbank`        | the `FBank` graph, read from its MIL: same values within 1.4e-5                                                                                                                  |
| Embedding per window and local speaker, overlapped frames left out | `embedding`    | `Embedding.community`: pooling weighted by the speaker's frames, so a plain filter of frames is not the same (other variance denominator); the ResNet is a network, ONNX Runtime |
| 256 to 128 values                                                  | `plda`         | the same two JSON files; output equal to the Core ML graph up to the sign of some dimensions, which VBx does not see                                                             |
| Agglomerative clustering, cut at 0.6                               | `ahc`          | `Clustering.community`                                                                                                                                                           |
| VBx, Fa 0.07, Fb 0.8, 20 iterations, tolerance 1e-4                | `vbx`          | `Clustering.community`, `VBx.community`                                                                                                                                          |
| Centroids, constrained assignment                                  | `assign`       | two local speakers of a window never share a cluster                                                                                                                             |
| Votes per frame, one speaker at a time, gaps under 0.1 s closed    | `reconstruct`  | `PostProcessing.community`                                                                                                                                                       |

The number of speakers is whatever the clustering finds: the 3 of the model
are the local speakers of one window, not a cap. There is no maximum anywhere
in the pipeline, the turns, the keys (`s1`…), the names or the colours.

**Runtime.** The `ort` crate (`=2.0.0-rc.15`), the CPU provider only (no
DirectML, CUDA or other provider), under `cfg(all(windows, x86_64))` like
`transcribe-cpp`, so macOS and Intel builds are unchanged.

- It uses `download-binaries`: the build downloads ONNX Runtime's prebuilt CPU
  binaries from the crate's CDN, checks them against the hash `ort-sys`
  pins, and links them statically. No `onnxruntime.dll` ships beside the app
  and none can be picked up by name from `System32`. The other choice,
  `load-dynamic` with a pinned DLL, needs a second download in the build, a
  place for the DLL in the installer and an absolute-path load to keep the
  system copy out; it stays the fallback if the static link fails on a runner.
- The binary needs AVX2. `Pyannote` checks it before the first call and a
  processor without it keeps one speaker per track (the log says `the speaker
model cannot run`).
- The arena allocator is off (it never gives memory back) and the encoder
  runs one window at a time, which is also the fastest per window on a CPU.
- A native error or panic is a `Failure`, never a crash: every call is inside
  `catch_unwind` (`analysis_cpp.rs`), and the messages carry fixed words, never
  a path or text.

**The speaker model files.** Three optional sources of the Windows manifest
(`local_models/pinned.rs`, folder `pyannote`, 32.7 MB together), pinned to
Hugging Face commits and SHA-256 like the others: the segmentation network
(5.9 MB), the embedding encoder split before its statistics pooling (21.3 MB)
with the weight and bias of its last layer (5.2 MB), and the two PLDA files
(0.27 MB, the files the Mac pins). Dictation never waits for them, and a
models folder ready before they were added stays ready. They are fetched when
a meeting starts (unless the user deleted the models or cancelled their
download) and, for a user who has Meetings on, after a dictation download and
at launch. Until Meetings are on, nothing fetches them. The analysis of a
meeting waits for that fetch to end. The analysis hashes the six files when it
opens them, so a file that is missing, partial or damaged is "no speaker
model".

**Sortformer is gone.** The first Windows speaker model (Sortformer v2.1:
at most 4 voices, trained on English) was a stepping stone and is not Mac
quality. Its source, accessor, code, licence lines and CI step are removed.
Nobody had fetched it: it was only downloaded for a user with Meetings on, and
they are hidden on Windows. A `sortformer` folder that an earlier build left
is deleted once the new optional files are in place.

**When it degrades.** Without the speaker model, or when a file is missing or
damaged, the words are still timed and each track keeps one speaker, as on an
Intel Mac. The analysis is then not kept as final, so it runs again once the
model is there. A runtime that cannot start (no AVX2), a failure in either
model, a track that cannot be read or a run past its time limit ends the
analysis the same way. It never fails the meeting.

**Limits.**

- There is no maximum number of speakers and the speaker model is not tied to
  English. How well it does on French, on crosstalk and on a headset is a
  real-PC check (`OWNER_CHECKS.md` of the lane).
- Memory (measured, Linux x86_64, release, see below): a 2.9-hour track peaks at
  1.13 GB of working set, 0.68 GB of which is the track itself, read whole
  into memory as before. The pipeline adds about 0.45 GB at that length: the
  embeddings (about 4.5 KB each, at most three per window) and the speaker
  weights of 5,300 windows kept for the reconstruction. Everything else is
  freed window group by group: 32 windows of features and encoder output are
  50 MB, and one encoder run holds under 100 MB. A 6-minute track peaks at
  0.37 GB. Nothing grows with the track but the embeddings and the weights.
- Time (measured on 2 threads of a server CPU, see below): 0.057 s per second of audio on a
  quiet machine (352 s of audio in 20 s on 2 threads), 0.076 on a 2.9-hour
  track while other jobs loaded the machine (799 s). The encoder is 100 ms
  per window of 10 s taken every 2 s, the segmentation 15 to 25 ms, the
  fbank 1.3 ms; clustering 16,000 embeddings takes about 30 s on one thread
  and 67 MB. A processor 5 times slower than that server takes about 1 h for
  a 3-hour meeting.
- The models use half of the cores, so a dictation made during the analysis
  still runs. A slow processor takes longer; the time limit is twice the audio
  it reads plus 10 minutes, so that a 3-hour meeting still
  ends on a processor 20 times slower than measured (the limit for two
  tracks of 3 hours is 18 h 10 min, and a hung model still ends it). The
  limit is unchanged; `a_three_hour_meeting_on_a_very_slow_processor_fits_the_limit`
  holds the arithmetic.

**Tests.**

- `meetings/diarize/` runs on every host: each stage against the Swift
  source's formulas with hand-checkable values; clustering on fixture
  embeddings (2, 3, 5 and 8 speakers recovered, one speaker over many windows
  stays one, two local speakers of a window never share a cluster, a stray
  embedding makes no speaker, same input same labels, empty and short input);
  the PLDA transform and the fbank against the Core ML graphs' constants.
- The Windows `rust` job opens the speaker model with missing and damaged
  files. `analysis_local.rs` covers turns for 12 speakers and a failing,
  missing or silent speaker model.
- The Windows `cpu model` job speaks a three-utterance dialogue with two of
  the runner's own voices (and in French when the runner has French voices),
  runs the analysis on it and checks that word times are in order and inside
  the clip, that at least 80 % of the words said are timed, and that two
  speakers are found with the change of speaker within 1.5 s of the splice,
  through ONNX Runtime and the files the app downloads.
- On a Linux machine, the real pipeline and the real models were run on a
  7-speaker recording made of public-domain read speech with known turns
  (not committed): 7 speakers
  in 18 turns over 352 s were recovered with 99.3 % of the frames agreeing
  with the known speaker after the best mapping, at 0.057 times real time and
  0.37 GB; the same recording repeated to 2.9 hours gave 7 speakers
  (799 s, 1.13 GB). This is accuracy against known turns on English read
  speech, not parity with the Mac and not French.
- The comparison with the Mac's own answer on the same audio
  (`diarize/parity.rs`: same number of speakers, change points within 1 s,
  at least 90 % of frames agreeing after the best mapping) waits for the
  file the Mac makes: the `meetings helper` workflow keeps the dialogue and
  the turns of a manual run, and `OWNER_CHECKS.md` says how to commit them.

CI cannot prove accuracy on real calls (accents, crosstalk, more than four
speakers), French accuracy when the runner has no French voice, or speed and
memory on a weak laptop.

## Audio file and export on Windows

- **Where.** `meetings/archive.rs` (the shared flow, as on macOS), `meetings/archive/media_foundation.rs` (encoding), `meetings/archive/aac.rs` (length, full decode, restoring the tracks), `meetings/archive/resample.rs` (16 kHz to 48 kHz and back), `meetings/save_panel.rs` (export).
- **Same file as macOS.** One AAC-LC stream in an MP4 container, `audio.m4a`: channel 1 the microphone, channel 2 system audio, one channel when only one track was recorded. A shorter track is padded with silence. Restoring writes the same 16 kHz mono WAVs, so re-transcription, retention and "delete audio" run the shared code unchanged.
- **Size.** About 43 MB an hour with two tracks, against about 29 MB on macOS: the AAC encoder of Windows takes 44.1 or 48 kHz only and goes no lower than 96 kbps. Two WAVs are about 230 MB an hour. The 16 kHz tracks are brought to 48 kHz by a windowed-sinc filter and back by another; both keep the exact length, with no delay.
- **No edit list.** The Windows MP4 sink writes none, so the encoder's priming and padding frames stay: a restored track starts and ends with a few tens of milliseconds of silence more. Both tracks shift together.
- **Never linked.** `mfplat.dll` and `mfreadwrite.dll` are loaded when the first archive runs, from the system folder. A link-time import would stop the whole app from starting on N and KN editions without the Media Feature Pack, and on Server without Media Foundation. There, or with no AAC encoder, `encode` fails with a fixed message and the WAV tracks stay.
- **Safe order.** The encoder writes `audio.m4a.part`. Before it counts as done, the whole file is decoded and its length compared with the longest track (within 1 s), and `compress` reads the container's length too. Only then is the part file renamed, and only then does `archive` delete the WAVs. After 3 failed attempts the WAVs stay for good and are no longer requeued.
- **Export.** `IFileSaveDialog` owned by the active window, suggesting the file name and its extension, asking before it replaces a file, remembering the folder like any Windows app. Cancelling returns no path.
- **Delete while compressing.** Windows keeps the part file open while the encoder writes, so deleting the audio in that window fails and the user tries again. On macOS the same click wins the race.

## Meeting notices on Windows

- **Where.** `meetings/announce.rs` (what finished, batching, the open target; no platform code), `overlay/meeting_notice.rs` (the notice on the orb overlay), `settings_window.rs` (`MainAction::OpenMeetings`, `viewed_meeting`). These are the macOS files, taken over with `git cherry-pick -x`; the module sits beside beta's single `overlay.rs` as `overlay/meeting_notice.rs`.
- **When.** The processor tells the app once a meeting is ready or failed (`ProcessEvents::finished`, once per meeting, before the summary is written). The notice shows for 15 s. Several meetings that finish together become one line ("2 meetings ready"), with a second line when some failed.
- **Not shown** while a dictation shows or another notice is up (it waits and appears afterwards), when overlay style is off, and for a meeting the user already has open in a visible Settings window. A new recording takes the overlay from a notice that is showing; news that arrived during a dictation shows after it.
- **Actions.** Open shows the Settings window on that meeting, built again if it was closed (`show_main_window`, which builds it on a thread of its own on Windows). Copy puts the meeting's Markdown on the clipboard. Neither takes the focus: the overlay window is not focusable and only takes clicks while a notice shows. Escape is not taken on Windows; the notice times out like the copy pill.
- **Windows glue.** The overlay window reports its page loads and its destruction to the notice state (`create_recording_overlay`), so a notice survives a reloaded page. `viewed_meeting` reads the window's visibility and minimized state (Windows has no Spaces). `clipboard::copy_to_keep` is no longer macOS-only.
- **Dormant.** Meetings stay off on Windows, so nothing announces a meeting yet. The logic is tested without hardware.

## Live transcript on Windows

- **Where.** `meetings/live/` is the macOS code, unchanged and platform-neutral: the tap on each track writer (`LiveTap` in `capture/mod.rs`), the chunker, the seam joiner, the echo filter, the draft file `live.ndjson`, the worker, the paste action. Windows adds `CpuModel::transcribe_words` in `engine/local/cpu.rs` and the platform gate `PLATFORM_RUNS_LIVE` in `live/mod.rs`.
- **Who can have it.** Apple silicon, and Windows on x86_64, when the setting `meeting_live_transcript` is on (it is by default) and the speech model is ready. Otherwise the live view says why: setting off, model missing, downloading or preparing, or no live engine on this platform. Intel Macs stay without it, as before. The Windows model is the GGUF one of dictation, so no new download.
- **One engine, dictation first.** The live pieces go to the same `LocalEngine` session as dictation, one model copy. `Session::transcribe_background` answers `Busy` at once, without queuing, while a dictation records, transcribes or waits for the model, and the worker asks again 250 ms later. A dictation therefore waits for the piece running, at most 12 s of audio, never for the queue. Pieces under -60 dBFS are not sent. The meeting analysis after Stop loads a model copy of its own (`analysis_cpp.rs`) and is not part of this.
- **Priority.** The thread that runs a live piece goes below the normal priority for the run (`SetThreadPriority`, `THREAD_PRIORITY_BELOW_NORMAL`) and gets its old priority back afterwards, since the blocking pool reuses its threads. This lowers the thread that calls transcribe.cpp. The worker threads the native library starts for the matrix work keep their own priority, so the protection a dictation really has is the `Busy` rule above, and this priority only softens the piece running. How much it helps on a laptop is a real-PC check.
- **Word times.** A live piece is run with word timestamps (`TimestampKind::Word`), as the meeting analysis does; each word becomes `Word { w, s, e }` in seconds from the start of the piece (`timed_words`). A decode that stopped early (`OutputTruncated`, `OutputRepetition`) keeps the words it found. Any other error drops the loaded model, so the next piece loads it again; three failed pieces in a row stop the live text for the meeting.
- **Speed.** Over 30 s of speech waiting shows "Catching up". Over 120 s skips the oldest pieces with a gap entry; the final transcript has them. A PC slower than real time therefore shows "Catching up", recovers when the speech pauses, and never queues without limit.
- **The shortcut.** "Paste new meeting text" (`meeting_paste_new`) ships unbound. It holds its keys only while Meetings are on and yields them to an action that uses them (`shortcut::MEETING_BINDINGS`). It pastes through the same Windows paste path as "Paste last", as `You:` and `Others:` lines.
- **Draft.** The live text is appended to `live.ndjson` in the meeting's folder, shown as stopped after a crash, never saved as the meeting's transcript, and deleted when the processor saves the final turns. Once deleted it is sealed: a late sentence from the worker does not bring it back.
- **Not proven in CI.** The real-time factor on a laptop CPU during a call, the effect of the thread priority on a dictation made during a piece, and pasting into Word, Notepad and a Teams chat. `OWNER_CHECKS.md` of the lane lists them.

## Known limits

- Before Windows 10 build 20348, Murmur's own sounds can reach the system track. The echo filter tolerates it.
- Windows may play call audio on the default communications device, which is not the default device. The capture follows the default device only.
- Not proven in CI: real call audio, Bluetooth switching, headsets, an unplugged device mid-meeting. `OWNER_CHECKS.md` of the lane lists them.
- Not proven in CI: real Teams (new, classic, packaged), Zoom, Slack, Discord and Webex calls, a browser's address bar through UI Automation in each browser, and what an app does with the microphone while muted. `OWNER_CHECKS.md` of the lane lists them.
- A Windows machine without the consent store (before Windows 10 1903) has no call detection, and the setting is hidden.
- A call app Windows does not list yet, or one renamed by its maker, is no call until it is added to the tables. The debug log lists the apps on the microphone, by file name or package family, so a missing one can be found.
- The compressed audio is larger than on macOS (about 43 MB an hour), and N or KN editions without the Media Feature Pack keep WAV tracks (about 230 MB an hour) because there is no encoder.
- Not proven in CI: playing `audio.m4a` in the Meeting detail and in other Windows apps, the save dialog's look and remembered folder, long paths and OneDrive-redirected folders, an N edition.

## Tests

- The live transcript's pure parts run on both platforms: chunker, seam joiner, echo filter, draft file, the tap on a real `TrackWriter`, the state and paste rules, the platform gate, `Session::transcribe_background` against a fake model. The word conversion of the CPU model (`timed_words`) is pure; the real model run is not in the normal job.
- The pure logic runs in the normal Windows `rust` job and on macOS: downmix, packet clock, pause-to-silence on the real `TrackWriter`, default-output follow state machine, route table, keep-awake guard (Windows only).
- The archive tests encode two tones with the real encoder and decode them: length, size an hour, channels kept apart (each tone ten times louder than the other on its own track), a single track, a missing channel, a track that cannot be read, the pipeline in `processor.rs`. They need the AAC encoder: the `rust` job runs `.github/scripts/windows-media-foundation.ps1`, which installs `Server-Media-Foundation` on Server images that lack it. The sample-rate filter, the PCM packing and the clock run without it. The save dialog is compile-checked only.
- `wasapi_loopback_hears_a_tone` and `wasapi_process_loopback_leaves_out_the_own_process` need a sound output. They are `#[ignore]`d, and the `meetings-audio` job of `windows-beta.yml` runs them with `--ignored`. They skip with a logged reason when the machine has no output. Run them by hand with `cargo test --lib wasapi_ -- --ignored --nocapture`.
