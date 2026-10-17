# Murmur

Murmur is a macOS desktop speech-to-text app. Press a shortcut, speak, and
Murmur pastes the transcription into the active application.

Murmur sends audio to a cloud transcription service using a session already on
the computer. Codex with ChatGPT is the default. On macOS, Gemini through the
local Antigravity session is available as an experimental alternative. Neither
option requires an API key or local speech-recognition model.

## Requirements

- macOS 10.15 or later
- A working microphone
- Codex signed in with a ChatGPT account, or Antigravity signed in on macOS
- Accessibility permission for global shortcuts and text insertion
- For meetings with system audio: macOS 14.4 or later on Apple silicon, and
  the System Audio Recording permission
- Network access while transcribing
- Optional: Apple Intelligence for on-device AI cleanup. It needs macOS 26 or
  later, a Mac that supports Apple Intelligence, Apple Intelligence turned on
  in System Settings, and a dictation language it supports. Without it, AI
  cleanup uses Codex when a Codex session is signed in, and otherwise pastes
  the raw transcript.

For Codex, Murmur reads the authentication cache from `CODEX_HOME/auth.json` or
`~/.codex/auth.json`. When the access token is about to expire, or when the
service rejects it, Murmur refreshes the session the same way the Codex CLI
does and writes the new tokens back to that file. It changes only the token
fields and `last_refresh`, keeps the file private (mode 600), and replaces it
in one step. If Codex updated the file while Murmur was refreshing, Murmur
keeps the Codex version. When the refresh token itself has expired, sign in to
Codex again.

For Gemini, Murmur starts a Murmur-owned language server from the verified
system Antigravity installation and lets that server access its own session.
Murmur does not read or copy the Antigravity token.

### Private interfaces

Both integrations use internal services rather than public, documented APIs:
the ChatGPT transcription endpoint that the Codex apps use, and the
Antigravity language server. Neither service promises stability, so a Codex,
ChatGPT, or Antigravity update can break transcription without notice until
Murmur is updated. Murmur identifies itself to the transcription endpoint the
way the Codex desktop app does. Use of these services stays subject to the
provider's terms for your account.

## Install

1. Download the DMG from the latest
   [GitHub release](https://github.com/DailyXplorer/Murmur/releases/latest):
   `aarch64` for Apple Silicon, `x64` for Intel.
2. Open the DMG and drag Murmur to Applications.
3. Open Murmur. Releases are signed with a stable self-signed certificate but
   are not notarized by Apple, so macOS blocks the first launch. Open System
   Settings → Privacy & Security and click **Open Anyway**. Since macOS 15,
   right-click → Open no longer bypasses this check.
4. Grant Microphone and Accessibility access when Murmur asks.

Murmur's built-in updater installs releases signed with the same certificate,
so macOS keeps both permissions and asks nothing again after an update. Murmur
does not store anything in the keychain.

## Features

- One recording shortcut (Option+Space by default): hold it to talk, or double-tap it to record hands-free and press it once to stop
- A cancel shortcut (Escape by default), active only while Murmur records or transcribes
- A shortcut (Control+Command+V by default) and tray item that paste the latest transcription again
- Automatic language detection for both services, with explicit language selection for Codex
- Codex or experimental Gemini transcription selection on macOS
- Microphone and output-device selection
- AI cleanup, on by default: removes fillers, applies self-corrections, fixes
  punctuation, and formats lists and spoken line breaks. The cleanup engine is
  selectable in Settings. Apple Intelligence on this Mac is the default. When
  it cannot run, Codex cleans the text if a Codex session is signed in. Any
  failure or timeout pastes the raw transcript.
- A dictionary that also learns names you correct right after pasting
- Names from the frontmost window as spelling hints for AI cleanup, read
  through Accessibility and never stored
- Automatic level boost for quiet speech, and a Whisper mode for very quiet speech
- Transcription history with saved recordings, keeping the raw and cleaned text; a failed
  or unsatisfying transcription can be sent again from its recording
- Insights: total words, words per minute over the last 30 days, current and
  longest streak with a 20-week heatmap, and usage by app category
- Meetings: record a call (Control+Option+Command+M or the menu bar), then
  get a transcript with speakers told apart and a summary with a title. See
  [Meetings](#meetings)
- A minimal waveform overlay at the top or bottom of the screen, which can be
  turned off
- Audio feedback, tray controls, and automatic updates

## Getting started

Grant microphone and Accessibility permissions, then choose a transcription
service. If it is unavailable, open Codex or Antigravity, sign in there, and
return to Murmur to refresh its status. The setup screen links to the
applications' download pages. The selected service must be configured before
setup can finish.

"Configured" means Murmur found a local session that is valid or that it can
refresh. A Codex session that expired and has no usable refresh token shows as
"Session expired". Configured does not prove that the cloud service will
accept the next request, because network or account limits can still prevent
transcription.

When a dictation fails, the recording overlay shows the reason for a few
seconds. If the overlay is turned off, Murmur posts a macOS notification
instead. With Codex, Murmur retries a dictation twice after a timeout, a dropped
connection, or a server error. It does not retry an expired session, a rate
limit, or another rejected request. A finished transcription is always kept
in History, even when pasting or saving the recording fails.

Recordings longer than about 13 minutes exceed 25 MB of audio. The Codex
transcription endpoint publishes no size limit, and the public OpenAI
transcription API rejects uploads above 25 MB, so Murmur warns before it
uploads a recording of that size. The upload timeout grows with the recording
length.

### Meetings

Meetings are on by default and can be turned off in Settings → Advanced.
Press Control+Option+Command+M, or choose Start Meeting in the menu bar, to
start recording; press it again to stop. You can pick other keys in Settings;
if another app already holds them, the shortcut field says so. A red pill
shows the elapsed time. A meeting stops by itself after 3 hours, with a
warning 5 minutes before.

- Murmur records two tracks: your microphone ("Me") and the system audio,
  which holds the other participants. System audio needs macOS 14.4 or later
  on Apple silicon, and the System Audio Recording permission, which macOS
  asks for at the first meeting. Without it, meetings record the microphone
  only. When another app played sound during a meeting but its system audio
  holds nothing but silence while you spoke, the meeting says so and links
  to the privacy settings.
- After you stop, Codex transcribes the tracks in pieces of up to 10 minutes,
  at most 3 at a time. Processing resumes after a quit, without sending a
  transcribed piece again.
- On Apple silicon, speaker detection runs on the Mac with
  [FluidAudio](https://github.com/FluidInference/FluidAudio). Its models
  (about 480 MB) download during the first meeting. Without them, each track
  keeps one speaker.
- Once the transcript is complete, Codex writes a title and a summary. The
  two tracks are then kept as one compressed `audio.m4a` (about 30 MB per
  hour), which you can delete while keeping the text.
- Murmur warns at the start of a meeting when less than 1.5 GB is free on
  the disk. Three hours take about 700 MB before compression.

### Permissions after updating to the stable signature

Releases are signed with a stable Murmur certificate, so macOS keeps the
Accessibility and microphone permissions from one update to the next. Earlier
releases were signed ad hoc. After the first update from an ad hoc release,
grant Accessibility and microphone access to Murmur once more. If the
Accessibility switch already looks enabled but Murmur keeps waiting, use the
repair button on the permissions screen, or follow the reset steps in
[BUILD.md](BUILD.md#macos-accessibility-remains-enabled-after-a-local-rebuild).

Gemini through Antigravity is experimental and detects the spoken language
automatically. A language selected for Codex is preserved when switching to
Gemini and applies again when switching back.

Cancel stops a transcription before its result is inserted. If the text has
already been pasted, cancellation still prevents a pending automatic Enter.
The pasted text and its history entry remain available.

## CLI

```bash
murmur --toggle-transcription
murmur --cancel
murmur --start-hidden
murmur --no-tray
murmur --debug
murmur --transcribe-file recording.wav --json
```

On Unix systems, `SIGUSR2` also toggles transcription for a running instance:

```bash
pkill -USR2 -n murmur
```

## Development

Prerequisites: the latest stable Rust toolchain and Bun.

```bash
bun install
bun run tauri dev
bun run build
bun run lint
bun run format:check
cargo test --manifest-path src-tauri/Cargo.toml
```

See [BUILD.md](BUILD.md) for macOS dependencies, packaging, code signing, and
notarization.

## Architecture

Murmur uses Tauri 2 with a Rust backend and a React/TypeScript frontend. The
runtime pipeline is:

```text
microphone -> quiet-speech gain -> selected cloud transcription -> dictionary -> AI cleanup (Apple Intelligence or Codex) -> clipboard/paste
```

The backend owns audio capture, the Codex session and its refresh, transcription,
AI cleanup, history, Insights, shortcuts, and macOS integration. The on-device
cleanup calls Apple's Foundation Models framework through a small Swift bridge.
The frontend owns onboarding, settings, history, Insights, and the recording
overlay.

## Privacy and security

Audio sent for transcription leaves the computer and is handled by ChatGPT or
Google, depending on the selected service. Do not describe Murmur as offline
or local-only.

AI cleanup with Apple Intelligence runs on the Mac, and the transcript does
not leave it for cleanup. The transcript is sent to ChatGPT for cleanup only
when Codex is the selected cleanup engine, or as a fallback when Apple
Intelligence cannot clean a dictation and a Codex session is signed in. A
cleanup request contains the transcript, your custom dictionary words, and the
names found in the frontmost window.

Those names are read on the Mac through Accessibility when a dictation starts,
only while AI cleanup and the window-names setting are both on. They are never
stored, and they leave the Mac only inside a Codex cleanup request. Password
fields and password managers are never read.

Meetings stay on the Mac, apart from two requests: the audio pieces sent to
Codex for transcription, and the transcript sent to Codex for the summary.
Speaker detection runs on the Mac. Its models are downloaded from Hugging
Face once.

History, saved recordings, learned words, and Insights stay on the Mac.
For Insights, Murmur records the frontmost app of each dictation and, in a
browser, the site's host name. Insights store no dictated text. Murmur checks GitHub for updates, which can be
turned off in Settings.

The Codex authentication cache is sensitive. Murmur reads the tokens needed
for a request or a refresh, never logs them, and keeps no copy of its own.
The only write is the token refresh described in
[Requirements](#requirements).

For Gemini, Murmur sends audio to a loopback-only Antigravity service with an
ephemeral CSRF token. It never logs that token. A language server started by
Murmur stops after five minutes without a Gemini dictation. Murmur never stops
an Antigravity process it did not start.

Guarded paste saves the clipboard before it places the transcription there.
By default, Murmur puts the saved items back, with every format they had, once
the 8-second paste window ends. If you copy something else during that window,
Murmur leaves your new clipboard alone. If insertion fails, Murmur keeps the
transcription on the clipboard for manual paste. The Keep Transcription option
leaves the transcription on the clipboard after insertion.

## Origin

Murmur is a fork of [Handy](https://github.com/cjpais/Handy), created by CJ
Pais. It keeps parts of Handy's Tauri foundation, but most of Handy's original
transcription engine and local-model stack have been removed. Murmur now
transcribes through cloud services using sessions already present on the
computer.

## License

MIT. See [LICENSE](LICENSE). The original copyright notice is preserved as
required by the license.
