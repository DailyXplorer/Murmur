# AGENTS.md

Guidance for AI coding assistants working in this repository.

## Project Shape

Handy is now a native macOS SwiftPM app. Do not reintroduce the deleted cross-platform project surfaces.

Keep the app focused on:

- SwiftUI/AppKit native macOS UI
- Local Whisper transcription through WhisperKit/Core ML
- API transcription providers such as Mistral and OpenAI-compatible endpoints
- Optional Apple Speech

## Commands

```bash
swift test -debug-info-format none
./script/build_and_run.sh
HANDY_PORTABLE_SMOKE=1 ./script/build_and_run.sh --verify
HANDY_PORTABLE_SMOKE=1 ./script/build_and_run.sh --readiness
HANDY_PORTABLE_SMOKE=1 ./script/build_and_run.sh --release-readiness
```

## Structure

- `Sources/HandyNative/` - app source
- `tests/HandyNativeTests/` - XCTest coverage
- `ThirdParty/ArgmaxWhisperKit/` - vendored WhisperKit dependency
- `Resources/` - packaging resources copied into the app bundle
- `script/build_and_run.sh` - build, sign, launch, archive, readiness, and notarization helper

## Code Style

- Prefer small Swift types with explicit error handling.
- Keep UI in SwiftUI unless AppKit is needed for macOS integration.
- Keep resource paths owned by `Resources/` and the build script.
- Avoid compatibility scaffolding for removed engines or deleted cross-platform code.
- Add focused XCTest coverage for behavior changes.

## GitHub Workflow

Before opening a PR or issue, read the matching template under `.github/`.

Use conventional commit prefixes:

- `feat:`
- `fix:`
- `docs:`
- `refactor:`
- `test:`
- `chore:`
