# Murmur

Murmur is a cross-platform desktop speech-to-text app for macOS, Windows, and
Linux. Press a shortcut, speak, and Murmur pastes the transcription into the
active application.

Murmur sends audio to the cloud transcription service used by Codex and
authenticates with the ChatGPT session already present on the computer. Setup
requires no API key or local speech-recognition model.

## Requirements

- A working microphone
- Codex installed and signed in with a ChatGPT account
- Accessibility permission when the operating system requires it for global
  shortcuts or text insertion
- Network access while transcribing

Murmur reads the Codex authentication cache from `CODEX_HOME/auth.json` or
`~/.codex/auth.json`. It never writes to that file. The transcription endpoint
is an internal ChatGPT service rather than a public, documented API, so a
future Codex or ChatGPT change may require a Murmur update.

## Features

- Global record, push-to-talk, and cancel shortcuts
- Automatic language detection or explicit language selection
- Microphone and output-device selection
- Optional filler-word removal and custom-word correction
- Transcription history with saved recordings
- Recording overlay, audio feedback, tray controls, and automatic updates
- Portable mode on Windows

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

See [BUILD.md](BUILD.md) for platform-specific dependencies and packaging.

## Architecture

Murmur uses Tauri 2 with a Rust backend and a React/TypeScript frontend. The
runtime pipeline is:

```text
microphone -> WAV/audio samples -> ChatGPT session transcription -> cleanup -> clipboard/paste
```

The backend owns audio capture, authentication-cache reading, transcription,
history, shortcuts, and platform integration. The frontend owns onboarding,
settings, history, and the recording overlay.

## Privacy and security

Audio sent for transcription leaves the computer and is handled by ChatGPT's
cloud service. Do not describe Murmur as offline or local-only.

The Codex authentication cache is sensitive. Murmur reads only the access token
and account identifier needed for a request, does not log them, and does not
persist a copy.

## License

MIT. See [LICENSE](LICENSE). The original copyright notice is preserved as
required by the license.
