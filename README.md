# Murmur

Murmur is a macOS desktop speech-to-text app. Press a shortcut, speak, and
Murmur pastes the transcription into the active application.

Murmur sends audio to a cloud transcription service using a session already on
the computer. Codex with ChatGPT is the default. On macOS, Gemini through the
local Antigravity session is available as an experimental alternative. Neither
option requires an API key or local speech-recognition model.

## Requirements

- A working microphone
- Codex signed in with a ChatGPT account, or Antigravity signed in on macOS
- Accessibility permission for global shortcuts and text insertion
- Network access while transcribing

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

## Features

- Global record, push-to-talk, and cancel shortcuts
- Automatic language detection for both services, with explicit language selection for Codex
- Codex or experimental Gemini transcription selection on macOS
- Microphone and output-device selection
- Optional filler-word removal for Codex and custom-word correction for both services
- Transcription history with saved recordings
- Recording overlay, audio feedback, tray controls, and automatic updates

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
microphone -> WAV/audio samples -> selected cloud transcription -> cleanup -> clipboard/paste
```

The backend owns audio capture, the Codex session and its refresh, transcription,
history, shortcuts, and macOS integration. The frontend owns onboarding,
settings, history, and the recording overlay.

## Privacy and security

Audio sent for transcription leaves the computer and is handled by ChatGPT or
Google, depending on the selected service. Do not describe Murmur as offline
or local-only.

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
