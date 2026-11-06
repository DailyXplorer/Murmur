# Meetings on Windows

## Product rule

**Meetings on Windows must reach the SAME quality as on macOS; reuse `main`'s code wherever it is platform-neutral.**

Meetings stay hidden on the Windows beta (no sidebar entry, no tray item, `meetings_enabled` forced off) until every row below is delivered and the enabling change lands. The pieces land first as tested code that no screen reaches.

## Parity checklist

Status: `Not yet` is not built. `Built, hidden` is built and tested but no screen reaches it.

| #   | Feature on macOS                                                                   | Windows                                            | Status                                                                           |
| --- | ---------------------------------------------------------------------------------- | -------------------------------------------------- | -------------------------------------------------------------------------------- |
| 1   | Start/stop a meeting from the tray item, the shortcut or the Meetings page         | Code on `beta`, enabled last                       | Not yet                                                                          |
| 2   | Record the microphone ("Me")                                                       | The microphone path of dictation (cpal, WASAPI)    | Not yet                                                                          |
| 3   | Record system or app audio ("Others"), Murmur's own sounds left out                | WASAPI process loopback, classic loopback fallback | Built, hidden (`meetings/capture/wasapi.rs`)                                     |
| 4   | Capture mode: both, system only, microphone only                                   | Settings exist                                     | Not yet                                                                          |
| 5   | Dropped-audio warning, low disk, 3 h cap, crash recovery, sleep notice, keep-awake | Code on `beta`; keep-awake is new                  | Partly: keep-awake built, hidden (`meetings/power_windows.rs`). The rest Not yet |
| 6   | Transcript through Codex, offline fallback to the local model                      | Code on `beta`                                     | Not yet                                                                          |
| 7   | Speaker separation, rename speakers                                                | In-process analyzer, at most 4 speakers            | Built, hidden (`meetings/analysis_cpp.rs`)                                       |
| 8   | Word-aligned merge, echo removal, headphone awareness                              | Neutral code; the output route is new              | Partly: route built, hidden (`loopback_logic::route_facts`). The rest Not yet    |
| 9   | AI title and summary, retry                                                        | Code on `beta`                                     | Not yet                                                                          |
| 10  | Compressed audio (`audio.m4a`), re-transcribe, retention, delete audio             | AAC through Media Foundation                       | Built, hidden (`meetings/archive/`)                                              |
| 11  | Export and copy Markdown (save dialog)                                             | `IFileSaveDialog`                                  | Built, hidden (`meetings/save_panel.rs`); copy Not yet                           |
| 12  | Meetings list, detail, search, delete                                              | UI and FTS5 search on `beta`                       | Not yet                                                                          |
| 13  | Live transcript, "Catching up", paste-new-text shortcut                            | Port of the live transcript                        | Not yet                                                                          |
| 14  | Call detected, prompt to record (Zoom, Teams, Slack, Webex, Discord, browser Meet) | Microphone consent store, shared state machine     | Built, hidden (`meetings/call_probe_windows.rs`)                                 |
| 15  | "Call ended" prompt                                                                | Port                                               | Built, hidden (`meetings/call_watch.rs`)                                         |
| 16  | Transcript-ready pill, open from the notice                                        | Port                                               | Built, hidden (`overlay/meeting_notice.rs`)                                      |
| 17  | MCP meeting tools                                                                  | Flip the Windows switch                            | Built, hidden (`mcp/`, the same five tools). Settings copy: at enable            |
| 18  | Models fetched with a meeting unless declined, licences dialog                     | New speaker model source                           | Partly: speaker model source built, hidden (`pinned::WINDOWS`). The rest Not yet |
| 19  | Settings: enable, call prompt, live transcript, retention, capture mode            | Port with Windows copy                             | Partly: the call prompt row has its Windows text. The rest Not yet               |
| 20  | FaceTime calls                                                                     | Not applicable on Windows                          | Not applicable                                                                   |
| 21  | Windows 10 before build 20348: system audio includes Murmur's own sounds           | Known limit, see below                             | Documented                                                                       |

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
process (`meetings/analysis_cpp.rs`). Two models run on the CPU through
transcribe.cpp:

- **Parakeet Ultra**, the model dictation already uses, with word timestamps.
  Each track is cut at pauses into pieces of at most 20 s
  (`engine::local::MAX_PIECE`), the pieces without speech are skipped, and
  each word is placed at its time on the track.
- **Sortformer v2.1** (`diar_streaming_sortformer_4spk-v2.1`, Q8_0, 139 MB),
  in its offline operating point, on the track `analysis::diarized_track`
  picks: system audio when the remote side was heard speaking there, else the
  microphone.

The decisions (pieces, times, which track, turns named `S1`…`S4` in a stable
order, how a failure ends the run) are in `meetings/analysis_local.rs`, which
has no model in it and is tested with fakes. `analysis_cpp.rs` holds the
models, the cancel token, a `catch_unwind` around them and a time limit that
grows with the audio.

**The speaker model.** It is a second, optional source of the Windows
manifest (`local_models/pinned.rs`, folder `sortformer`), pinned to a
Hugging Face commit and a SHA-256 like the others. Dictation never waits for
it, and a models folder ready before it was added stays ready. It is fetched
when a meeting starts (unless the user deleted the models or cancelled their
download) and, for a user who has Meetings on, after a dictation download
and at launch. Until Meetings are on, nothing fetches it. The analysis of a
meeting waits for that fetch to end.

**When it degrades.** Without the speaker model, or when the file is missing
or damaged, the words are still timed and each track keeps one speaker, as on
an Intel Mac. The analysis is then not kept as final, so it runs again once
the model is there. A failure in either model, a track that cannot be read or
a run past its time limit ends the analysis the same way. It never fails the
meeting.

**Limits.**

- Sortformer tells up to **4 voices** apart on the track it is given. A fifth
  remote speaker is merged into another. The Mac models have no such limit.
- It was trained on **English** speech. Other languages are timed by
  Parakeet as usual, but the voices are told apart less reliably.
- A track is read whole into memory for the speaker model: about 230 MB of
  samples per hour of audio, and about as much again for its features. The
  longest meeting (3 hours) peaks near 1.3 GB while it is analysed (an
  estimate from the sizes, not a measurement).
- The models use half of the cores, so a dictation made during the analysis
  still runs. A slow processor takes longer; the time limit is twice the audio
  it reads plus 10 minutes.

**Tests.** `analysis_local.rs` runs everywhere. The Windows `cpu model` job
speaks a three-utterance dialogue with two of the runner's own voices, runs
the analysis on it and checks that word times are in order and inside the
clip, that at least 80 % of the words said are timed, and that two speakers
are found with the change of speaker within 1.5 s of the splice. It cannot
prove accuracy on real calls (accents, crosstalk, more than four speakers) or
speed on a weak processor.

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

- The pure logic runs in the normal Windows `rust` job and on macOS: downmix, packet clock, pause-to-silence on the real `TrackWriter`, default-output follow state machine, route table, keep-awake guard (Windows only).
- The archive tests encode two tones with the real encoder and decode them: length, size an hour, channels kept apart (each tone ten times louder than the other on its own track), a single track, a missing channel, a track that cannot be read, the pipeline in `processor.rs`. They need the AAC encoder: the `rust` job runs `.github/scripts/windows-media-foundation.ps1`, which installs `Server-Media-Foundation` on Server images that lack it. The sample-rate filter, the PCM packing and the clock run without it. The save dialog is compile-checked only.
- `wasapi_loopback_hears_a_tone` and `wasapi_process_loopback_leaves_out_the_own_process` need a sound output. They are `#[ignore]`d, and the `meetings-audio` job of `windows-beta.yml` runs them with `--ignored`. They skip with a logged reason when the machine has no output. Run them by hand with `cargo test --lib wasapi_ -- --ignored --nocapture`.
