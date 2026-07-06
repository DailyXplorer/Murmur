# Contributing

Thanks for working on Murmur.

This repository contains the native macOS app only. Keep changes aligned with that scope.

## Setup

```bash
xcode-select --install
swift test -debug-info-format none
./script/build_and_run.sh
```

## Before A Pull Request

Run:

```bash
swift test -debug-info-format none
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --verify
```

For packaging or release-related changes, also run:

```bash
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --readiness
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --release-readiness
```

## Scope

Good changes:

- Native macOS app fixes
- WhisperKit/Core ML transcription fixes
- API transcription fixes
- Apple Speech fixes
- Packaging, signing, readiness, and test improvements
- Documentation updates for the native macOS app

Out of scope:

- Reintroducing deleted cross-platform project surfaces
- Non-macOS platform support
- Compatibility shims for removed local engines

## Style

- Keep changes focused.
- Prefer clear Swift over clever abstractions.
- Add tests when behavior changes.
- Do not commit local build output or generated smoke data.

## Commits

Use conventional commit prefixes:

- `feat:`
- `fix:`
- `docs:`
- `refactor:`
- `test:`
- `chore:`
