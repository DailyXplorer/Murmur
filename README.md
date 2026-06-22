# Handy

Handy is a native macOS speech-to-text app. Press a shortcut, speak, and Handy pastes the transcription into the active app.

This repository is an experimental native Swift rewrite of [cjpais/Handy](https://github.com/cjpais/Handy), the original open-source Handy project. The goal is to keep the same direct speech-to-text workflow while making the app feel at home on macOS: SwiftUI/AppKit UI, Core ML local transcription, Keychain-backed credentials, native shortcuts, and a small reproducible SwiftPM build.

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
- Optional Apple Speech transcription
- Global shortcuts, push-to-talk, recording overlay, history, audio feedback, and post-processing
- Local ZIP/DMG packaging script for personal builds

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
/tmp/handy-native-dist/Handy.app
```

For an isolated smoke launch:

```bash
HANDY_PORTABLE_SMOKE=1 ./script/build_and_run.sh --verify
```

## Local Packaging

Create local ZIP and DMG artifacts:

```bash
HANDY_PORTABLE_SMOKE=1 ./script/build_and_run.sh --release-readiness
```

Artifacts are written under:

```text
/tmp/handy-native-dist/archive/
```

Local builds are ad-hoc signed by default. Developer ID signing and notarization are only needed if you distribute prebuilt binaries to other people.

## Native Readiness Checks

```bash
swift test -debug-info-format none
HANDY_PORTABLE_SMOKE=1 ./script/build_and_run.sh --verify
HANDY_PORTABLE_SMOKE=1 ./script/build_and_run.sh --readiness
HANDY_PORTABLE_SMOKE=1 ./script/build_and_run.sh --release-readiness
```

`--readiness` writes JSON reports under:

```text
/tmp/handy-native-dist/readiness/
```

## Architecture

- `Sources/HandyNative/` - native app source
- `tests/HandyNativeTests/` - XCTest coverage
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
/Applications/Handy.app/Contents/MacOS/Handy --toggle-transcription
/Applications/Handy.app/Contents/MacOS/Handy --toggle-post-process
/Applications/Handy.app/Contents/MacOS/Handy --cancel
/Applications/Handy.app/Contents/MacOS/Handy --start-hidden
/Applications/Handy.app/Contents/MacOS/Handy --no-tray
/Applications/Handy.app/Contents/MacOS/Handy --debug
```

## Permissions

Handy needs:

- Microphone access to record speech
- Accessibility access to paste or type the transcription into other apps
- Speech Recognition access only when using Apple Speech

## Contributing

Contributions are welcome, especially around transcription quality, model handling, macOS permissions, packaging, accessibility, and measured performance. Please keep changes focused on the native macOS app.

See [CONTRIBUTING.md](CONTRIBUTING.md) and [BUILD.md](BUILD.md).
