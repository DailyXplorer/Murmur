# Murmur

Murmur is a macOS desktop speech-to-text app. Press a shortcut, speak, and
Murmur pastes the transcription into the active application.

Murmur sends audio to a cloud transcription service using a session already on
the computer. Codex with ChatGPT is the default. On macOS, Gemini through the
local Antigravity session is available as an experimental alternative. Murmur
can also use Muse Voice Transcribe through the official Meta Model API. Codex
and Gemini require no API key or local speech-recognition model. An experimental
Meta AI app mode can instead drive Meta AI's global dictation without a key
while keeping Murmur's overlay visible. The Meta Model API requires an API key
and may incur usage charges under Meta's current pricing.

## Requirements

- A working microphone
- Codex signed in with a ChatGPT account, Antigravity signed in on macOS, Meta
  AI for Mac running in the menu bar with Dictation set to Hold Fn, or a Meta
  Model API key
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

The separate experimental Meta AI app mode does not use that API. Open Meta AI
once, enable Dictation with Hold Fn, close its main window, and leave it running
in the menu bar. Murmur refuses to start if Meta has a visible window or owns an
existing dictation. It then holds Meta's global Fn shortcut and places Meta's
small dictation indicator behind Murmur's non-activating overlay. Meta AI
captures the microphone and types directly into the focused app. Murmur never
reads Meta credentials or its private network protocol. Because Murmur does not
receive the transcript, history, language, microphone, text-processing, and
output settings do not apply in this mode.

## Features

- Global record, push-to-talk, and cancel shortcuts
- Automatic language detection or explicit language selection
- Codex, experimental Gemini, Meta Model API, or experimental Meta AI app
  dictation selection on macOS
- Microphone and output-device selection
- Optional filler-word removal and custom-word correction for Codex and the
  Meta Model API
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

The experimental Meta AI app path is intentionally separate:

```text
Murmur shortcut -> Meta AI global dictation -> focused app
                -> Murmur overlay only
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

In Meta AI app mode, Meta AI owns microphone capture and remote transcription.
Murmur never launches, activates, or hides Meta automatically. It posts the
configured Fn shortcut only after confirming that Meta has no visible main
window and that the target app still has focus. It uses macOS Accessibility to
repeat those checks and position Meta's dictation indicator behind the Murmur
overlay. It does not read Meta's Keychain items, session token, or private
WebSocket traffic.

## Origin

Murmur is a fork of [Handy](https://github.com/cjpais/Handy), created by CJ
Pais. It keeps parts of Handy's Tauri foundation, but most of Handy's original
transcription engine and local-model stack have been removed. Murmur now
transcribes through cloud services using sessions already present on the
computer, Meta AI's direct app dictation, or a user-provided Meta Model API key.

## License

MIT. See [LICENSE](LICENSE). The original copyright notice is preserved as
required by the license.
