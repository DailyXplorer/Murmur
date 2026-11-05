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
| 7   | Speaker separation, rename speakers                                                | In-process analyzer, at most 4 speakers            | Not yet                                                                          |
| 8   | Word-aligned merge, echo removal, headphone awareness                              | Neutral code; the output route is new              | Partly: route built, hidden (`loopback_logic::route_facts`). The rest Not yet    |
| 9   | AI title and summary, retry                                                        | Code on `beta`                                     | Not yet                                                                          |
| 10  | Compressed audio (`audio.m4a`), re-transcribe, retention, delete audio             | AAC through Media Foundation                       | Not yet                                                                          |
| 11  | Export and copy Markdown (save dialog)                                             | `IFileSaveDialog`                                  | Not yet                                                                          |
| 12  | Meetings list, detail, search, delete                                              | UI and FTS5 search on `beta`                       | Not yet                                                                          |
| 13  | Live transcript, "Catching up", paste-new-text shortcut                            | Port of the live transcript                        | Not yet                                                                          |
| 14  | Call detected, prompt to record (Zoom, Teams, Slack, Webex, Discord, browser Meet) | Microphone consent store, shared state machine     | Built, hidden (`meetings/call_probe_windows.rs`)                                 |
| 15  | "Call ended" prompt                                                                | Port                                               | Built, hidden (`meetings/call_watch.rs`)                                         |
| 16  | Transcript-ready pill, open from the notice                                        | Port                                               | Not yet                                                                          |
| 17  | MCP meeting tools                                                                  | Flip the Windows switch                            | Not yet                                                                          |
| 18  | Models fetched with a meeting unless declined, licences dialog                     | New speaker model source                           | Not yet                                                                          |
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

## Known limits

- Before Windows 10 build 20348, Murmur's own sounds can reach the system track. The echo filter tolerates it.
- Windows may play call audio on the default communications device, which is not the default device. The capture follows the default device only.
- Not proven in CI: real call audio, Bluetooth switching, headsets, an unplugged device mid-meeting. `OWNER_CHECKS.md` of the lane lists them.
- Not proven in CI: real Teams (new, classic, packaged), Zoom, Slack, Discord and Webex calls, a browser's address bar through UI Automation in each browser, and what an app does with the microphone while muted. `OWNER_CHECKS.md` of the lane lists them.
- A Windows machine without the consent store (before Windows 10 1903) has no call detection, and the setting is hidden.
- A call app Windows does not list yet, or one renamed by its maker, is no call until it is added to the tables. The debug log lists the apps on the microphone, by file name or package family, so a missing one can be found.

## Tests

- The pure logic runs in the normal Windows `rust` job and on macOS: downmix, packet clock, pause-to-silence on the real `TrackWriter`, default-output follow state machine, route table, keep-awake guard (Windows only).
- `wasapi_loopback_hears_a_tone` and `wasapi_process_loopback_leaves_out_the_own_process` need a sound output. They are `#[ignore]`d, and the `meetings-audio` job of `windows-beta.yml` runs them with `--ignored`. They skip with a logged reason when the machine has no output. Run them by hand with `cargo test --lib wasapi_ -- --ignored --nocapture`.
