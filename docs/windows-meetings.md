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
| 14  | Call detected, prompt to record (Zoom, Teams, Slack, Webex, Discord, browser Meet) | New probes, shared state machine                   | Not yet                                                                          |
| 15  | "Call ended" prompt                                                                | Port                                               | Not yet                                                                          |
| 16  | Transcript-ready pill, open from the notice                                        | Port                                               | Not yet                                                                          |
| 17  | MCP meeting tools                                                                  | Flip the Windows switch                            | Not yet                                                                          |
| 18  | Models fetched with a meeting unless declined, licences dialog                     | New speaker model source                           | Not yet                                                                          |
| 19  | Settings: enable, call prompt, live transcript, retention, capture mode            | Port with Windows copy                             | Not yet                                                                          |
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

## Known limits

- Before Windows 10 build 20348, Murmur's own sounds can reach the system track. The echo filter tolerates it.
- Windows may play call audio on the default communications device, which is not the default device. The capture follows the default device only.
- Not proven in CI: real call audio, Bluetooth switching, headsets, an unplugged device mid-meeting. `OWNER_CHECKS.md` of the lane lists them.

## Tests

- The pure logic runs in the normal Windows `rust` job and on macOS: downmix, packet clock, pause-to-silence on the real `TrackWriter`, default-output follow state machine, route table, keep-awake guard (Windows only).
- `wasapi_loopback_hears_a_tone` and `wasapi_process_loopback_leaves_out_the_own_process` need a sound output. They are `#[ignore]`d, and the `meetings-audio` job of `windows-beta.yml` runs them with `--ignored`. They skip with a logged reason when the machine has no output. Run them by hand with `cargo test --lib wasapi_ -- --ignored --nocapture`.
