# Murmur for Mac

Murmur for Mac is a native macOS speech-to-text app. Press a shortcut, speak, and Murmur pastes the transcription into the active app.

This repository, `Murmur`, is an experimental native Swift rewrite of [cjpais/Handy](https://github.com/cjpais/Handy), the original open-source Handy project. The goal is to keep the same direct speech-to-text workflow while making the app feel at home on macOS: SwiftUI/AppKit UI, Core ML local transcription, Keychain-backed credentials, native shortcuts, and a small reproducible SwiftPM build.

Compared with the original app, this fork also adds configurable API transcription models, OpenAI-compatible provider settings, Mistral Voxtral support, color themes, a native model screen, and a more Mac-focused packaging/readiness workflow.

I am publishing this version to gather feedback from macOS users and developers. Issues, bug reports, performance notes, and focused contributions are welcome.

This repository is intentionally macOS-only. It does not contain the previous Tauri, React, Rust, Bun, or Nix project surfaces.

## Why Native Mac

The previous cross-platform tree mixed a Tauri shell, Rust backend, React UI, JavaScript/TypeScript tooling, Bun, Vite, and platform packaging layers. This snapshot is focused on a Mac-native implementation that can be built with Swift Package Manager and Xcode Command Line Tools.

Current local publication metrics:

- Native app bundle: `13 MB`
- Main executable: `11 MB`
- App bundle files: `25`
- XCTest coverage: `275` tests
- Published source snapshot: `229` selected app/build/test/resource files
- Swift files in the published app tree: `198`
- Rust files in the published app tree: `0`
- JavaScript/TypeScript files in the published app tree: `0`

Compared with the original cross-platform source tree in this repository before the native rewrite, the published snapshot removes the old Tauri/Rust/React toolchain surface: `49` Rust files, `131` JavaScript/TypeScript files, and `289` legacy cross-platform/build files are no longer part of the app source needed to rebuild this Mac version.

These numbers describe the repository and local app footprint on my machine. They are not a universal benchmark; real transcription speed and memory use depend on the Mac, model, accelerator setting, language, and audio length.

## Features

- Native SwiftUI/AppKit macOS app
- Local Whisper transcription through WhisperKit and Core ML
- API transcription providers, including Mistral Voxtral and OpenAI-compatible endpoints
- Custom API model entries, so you can add provider model IDs without changing the app code
- Optional Apple Speech transcription
- Color themes for the native macOS interface
- Global shortcuts, push-to-talk, recording overlay, history, audio feedback, and post-processing
- Local ZIP/DMG packaging script for personal builds

## Screenshots

| Model onboarding | General settings |
| --- | --- |
| ![Choose a transcription model during onboarding](docs/screenshots/onboarding-models.png) | ![Configure shortcuts, language, microphone, and audio feedback](docs/screenshots/general-settings.png) |

## Requirements

- macOS 14 or newer
- Xcode Command Line Tools

Install the tools with:

```bash
xcode-select --install
```

## Build And Run

```bash
swift test -debug-info-format none
./script/build_and_run.sh
```

The build script creates and launches:

```text
/tmp/murmur-native-dist/Murmur.app
```

For an isolated smoke launch:

```bash
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --verify
```

## Local Packaging

Create local ZIP and DMG artifacts:

```bash
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --release-readiness
```

Artifacts are written under:

```text
/tmp/murmur-native-dist/archive/
```

Local builds are ad-hoc signed by default. Developer ID signing and notarization are only needed if you distribute prebuilt binaries to other people.

## Native Readiness Checks

```bash
swift test -debug-info-format none
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --verify
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --readiness
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --release-readiness
```

`--readiness` writes JSON reports under:

```text
/tmp/murmur-native-dist/readiness/
```

## Architecture

- `Sources/MurmurNative/` - native app source
- `tests/MurmurNativeTests/` - XCTest coverage
- `ThirdParty/ArgmaxWhisperKit/` - vendored WhisperKit dependency
- `Resources/` - app icon, entitlements, fonts, logos, and sounds used by the packaging script
- `script/build_and_run.sh` - build, sign, launch, archive, readiness, and notarization helper

Main transcription paths:

- Local Whisper: `WhisperKitTranscriptionService`
- API transcription: `APITranscriptionService`
- Apple Speech: `AppleSpeechTranscriptionService`

## CLI Flags

The app binary supports these runtime flags:

```bash
/Applications/Murmur.app/Contents/MacOS/Murmur --toggle-transcription
/Applications/Murmur.app/Contents/MacOS/Murmur --toggle-post-process
/Applications/Murmur.app/Contents/MacOS/Murmur --cancel
/Applications/Murmur.app/Contents/MacOS/Murmur --start-hidden
/Applications/Murmur.app/Contents/MacOS/Murmur --no-tray
/Applications/Murmur.app/Contents/MacOS/Murmur --debug
```

## Permissions

Murmur needs:

- Microphone access to record speech
- Accessibility access to paste or type the transcription into other apps
- Speech Recognition access only when using Apple Speech

## Contributing

Contributions are welcome, especially around transcription quality, model handling, macOS permissions, packaging, accessibility, and measured performance. Please keep changes focused on the native macOS app.

See [CONTRIBUTING.md](CONTRIBUTING.md) and [BUILD.md](BUILD.md).
