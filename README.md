# Murmur

Murmur is a macOS desktop speech-to-text app. Press a shortcut, speak, and
Murmur pastes the transcription into the active application.

Murmur sends audio to a cloud transcription service using a session already on
the computer. Codex with ChatGPT is the default. On macOS, Gemini through the
local Antigravity session is available as an experimental alternative. Murmur
can also use Muse Voice Transcribe through the official Meta Model API. Codex
and Gemini require no API key or local speech-recognition model. Meta requires
an API key and charges $0.18 per audio hour; free-tier credits may apply.

## Requirements

- A working microphone
- Codex signed in with a ChatGPT account, Antigravity signed in on macOS, or a
  Meta Model API key
- Accessibility permission for global shortcuts and text insertion
- Network access while transcribing

For Codex, Murmur reads the authentication cache from `CODEX_HOME/auth.json` or
`~/.codex/auth.json`. It never writes to that file. For Gemini, Murmur starts or
reuses Antigravity's local language server and lets that server access its own
session. Murmur does not read or copy the Antigravity token. Both integrations
use internal services rather than public, documented APIs, so a Codex,
ChatGPT, or Antigravity update may require a Murmur update.

For Meta, Murmur sends 16 kHz WAV audio to the documented Muse Voice Transcribe
file API. The API key is stored in macOS Keychain and is never returned to the
frontend after it is saved.

## Features

- Global record, push-to-talk, and cancel shortcuts
- Automatic language detection or explicit language selection
- Codex, experimental Gemini, or Meta Muse Voice Transcribe selection on macOS
- Microphone and output-device selection
- Optional filler-word removal for Codex and Meta, plus custom-word correction
  for all services
- Transcription history with saved recordings
- Recording overlay, audio feedback, tray controls, and automatic updates

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

The backend owns audio capture, authentication-cache reading, transcription,
history, shortcuts, and macOS integration. The frontend owns onboarding,
settings, history, and the recording overlay.

## Privacy and security

Audio sent for transcription leaves the computer and is handled by ChatGPT,
Google, or Meta, depending on the selected service. Do not describe Murmur as
offline or local-only.

The Codex authentication cache is sensitive. Murmur reads only the access token
and account identifier needed for a request, does not log them, and does not
persist a copy.

For Gemini, Murmur sends audio to a loopback-only Antigravity service with an
ephemeral CSRF token. It never logs that token. A language server started by
Murmur stops after five minutes without a Gemini dictation. Murmur never stops
an Antigravity process it did not start.

For Meta, Murmur keeps the API key in macOS Keychain, never logs it, and sends
it only to Meta's documented Model API endpoint as a bearer credential.

## Origin

Murmur is a fork of [Handy](https://github.com/cjpais/Handy), created by CJ
Pais. It keeps parts of Handy's Tauri foundation, but most of Handy's original
transcription engine and local-model stack have been removed. Murmur now
transcribes through cloud services using sessions already present on the
computer or a user-provided Meta Model API key.

## License

MIT. See [LICENSE](LICENSE). The original copyright notice is preserved as
required by the license.
