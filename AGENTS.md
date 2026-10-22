# AGENTS.md

This file provides guidance to AI coding assistants working with code in this repository.

## Development Commands

**Prerequisites:**

- [Rust](https://rustup.rs/) (latest stable)
- [Bun](https://bun.sh/) package manager

**Core Development:**

```bash
# Install dependencies
bun install

# Run in development mode
bun run tauri dev
# If cmake error on macOS:
CMAKE_POLICY_VERSION_MINIMUM=3.5 bun run tauri dev

# Build for production
bun run tauri build

# Frontend only development
bun run dev        # Start Vite dev server
bun run build      # Build frontend (TypeScript + Vite)
bun run preview    # Preview built frontend
```

**Checks (run before pushing):**

```bash
bun run check             # CI code-quality job: audit, translations, macOS guard, versions, ESLint, format (Prettier + cargo fmt --check), build, test typecheck. Clippy on macOS.
bun run test:playwright   # UI changes. This is the Playwright CI job.
```

From `src-tauri` on macOS, Rust behavior changes also need `cargo test --locked`. The macOS CI job runs that, plus Clippy with `-D warnings`.

**Model Setup:** Transcription uses the Codex / ChatGPT session already on the machine (`~/.codex/auth.json`). No local model download is required.

For macOS build setup, code signing, and notarization, see [BUILD.md](BUILD.md).

## Architecture Overview

Murmur is a macOS desktop speech-to-text application built with Tauri 2.x (Rust backend + React/TypeScript frontend).

### Backend Structure (src-tauri/src/)

- `lib.rs` - Main entry point, Tauri setup, manager initialization
- `managers/` - Core business logic:
  - `audio.rs` - Audio recording and device management
  - `transcription.rs` - Speech-to-text processing pipeline
  - `history.rs` - Transcription history storage
- `audio_toolkit/` - Low-level audio processing:
  - `audio/` - Device enumeration, recording, resampling
- `commands/` - Tauri command handlers for frontend communication
- `cli.rs` - CLI argument definitions (clap derive)
- `shortcut/` - Global keyboard shortcut handling
- `settings.rs` - Application settings management
- `overlay.rs` - Native macOS recording overlay window
- `signal_handle.rs` - `send_transcription_input()` reusable function
- `cleanup/` - AI cleanup prompt and the Codex responses client
- `dictionary/` - Learned words and the post-paste correction watcher
- `engine/` - Which engine transcribes a dictation: a pure `route`/`reroute` table (Local or Cloud, one fallback at most, History records why) and `local/`, Parakeet on this Mac. `local/session.rs` loads the model when a recording starts, runs one request at a time and releases it after 10 idle minutes (never while recording); long dictations are cut at pauses (`audio_toolkit/pauses.rs`) into pieces of one pass. Apple silicon runs it in the meetings helper's long-lived `serve` mode (`local/fluid.rs`, one WAV per request in `app_cache/local-stt`), Intel Macs on the CPU with transcribe.cpp (`local/cpu.rs`, the only `[target.'cfg(…)']` section `check:macos-only` allows). Debug builds fail the local run with `MURMUR_FORCE_LOCAL_FAILURE`
- `local_models/` - The on-device speech models shared by dictation and meetings: a manifest pinned to Hugging Face commits with every file's SHA-256 (`pinned.rs`, written by `scripts/generate-model-manifest.ts`), a resumable downloader, and the `Missing → Downloading → Preparing → Ready` state behind the `*_local_model*` commands. Apple silicon keeps them in `app_data/meetings/models` as FluidAudio expects and loads them once with the helper's `prepare`; folders an earlier version downloaded are checked file by file
- `meetings/` - Meeting recording, on by default (`meetings_enabled`, switch in Advanced): microphone and system audio (Core Audio process tap in `swift/meeting_audio.swift`) streamed to two WAVs, the 3-hour cap, a low-disk warning at start (`coordinator.rs`), and the `meetings.db` library. A system track of pure digital zeros while the mic has speech and another app played sound (polled from Core Audio's per-process output state, `system_played`) flags the meeting (`system_audio_silent`), since that is what a denied System Audio Recording permission produces. A denied microphone is read from macOS at start (`microphone_denied`, alert `MicrophoneDenied`), never guessed from zeros, which a muted input gives too; each processing run logs its stage timings (`Meeting N timings:`). After a meeting stops, `processor.rs` splits each track at pauses (`chunking.rs`), sends the pieces to Codex (`transcribe.rs`, at most 3 at a time, resumable), and interleaves both tracks into the transcript (`spread.rs`, `merge.rs`). Once the transcript is complete, Codex writes the title and summary (`summary.rs`, people as `[[key]]` markers so a rename needs no rewrite), and the tracks are compressed into one `audio.m4a` (`archive.rs`, `swift/meeting_archive.swift`). Meanwhile `murmur-meetings-helper` (`swift/MurmurMeetingsHelper`, FluidAudio, a separate process; `helper.rs`, `protocol.rs`, the shared `local_models`, fetched with a meeting unless declined, `models.rs`) times the words heard on device and tells speakers apart, on system audio when the remote side was heard speaking there, else on the microphone (`analysis.rs`); `align/` places the Codex words at those times, `speakers.rs` gives each word its speaker and `bleed.rs` drops remote voices the microphone echoed. Without the models, or on Intel, each track keeps one speaker
- `window_names.rs`, `accessibility.rs`, `field_excerpt.rs` - Name hints and the text around the cursor, read from the frontmost window for cleanup
- `utils.rs` - Tray, window, and application helpers

### Frontend Structure (src/)

- `App.tsx` - Main component with onboarding flow
- `components/` - React UI components:
  - `settings/` - Settings UI
  - `onboarding/` - First-run experience
  - `overlay/` - Recording overlay UI
  - `update-checker/` - App update notifications
  - `shared/`, `ui/`, `icons/`, `footer/` - Shared components
- `hooks/useSettings.ts` - Settings state management hook
- `stores/settingsStore.ts` - Zustand store for settings
- `bindings.ts` - Auto-generated Tauri type bindings (via tauri-specta)
- `overlay/` - Recording overlay window entry point
- `lib/types.ts` - Shared TypeScript type definitions

### Key Architecture Patterns

**Manager Pattern:** Core functionality organized into audio, transcription, and history managers initialized at startup and managed via Tauri state.

**Command-Event Architecture:** Frontend → Backend via Tauri commands; Backend → Frontend via events.

**Pipeline Processing:** Audio → quiet-speech gain (`audio_toolkit/gain.rs`) → local or Codex transcription (`engine/`) → dictionary → AI cleanup through the Codex session (`cleanup/`, raw text on any failure) → Clipboard/Paste → correction learning (`dictionary/`)

**State Flow:** Zustand → Tauri Command → Rust State → Persistence (tauri-plugin-store)

### Technology Stack

**Core Libraries:**

- Codex cloud transcription via the local ChatGPT/Codex session
- `cpal` - Cross-platform audio I/O
- `tauri-plugin-global-shortcut` - Global keyboard shortcuts
- `rubato` - Audio resampling
- `rodio` - Audio playback for feedback sounds

### Application Flow

1. **Initialization:** App starts minimized to tray, loads settings, initializes managers
2. **Recording:** Global shortcut triggers audio recording
3. **Processing:** Audio sent to Codex cloud transcription
4. **Output:** Text pasted to active application via system clipboard

### Settings System

Settings are stored using Tauri's store plugin with reactive updates:

- Keyboard shortcuts (configurable; the recording shortcut is hold-to-talk or double-tap hands-free, see `dictation_gesture.rs`)
- Audio devices (microphone/output selection)
- Codex / ChatGPT session for cloud transcription
- Audio feedback and overlay options

### Single Instance Architecture

The app enforces single instance behavior — launching when already running brings the settings window to front rather than creating a new process. Remote control flags (`--toggle-transcription`, etc.) work by launching a second instance that sends args to the running instance via `tauri_plugin_single_instance`, then exits.

## Internationalization (i18n)

All user-facing strings must use i18next translations. ESLint enforces this (no hardcoded strings in JSX).

**Adding new text:**

1. Add key to `src/i18n/locales/en/translation.json`
2. Use in component: `const { t } = useTranslation(); t('key.path')`

**File structure:**

```
src/i18n/
├── index.ts           # i18n setup
├── languages.ts       # Language metadata
└── locales/
    ├── en/translation.json  # English (source)
    └── fr/, es/, de/, it/, pt/
```

For translation contribution guidelines, see [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md).

## Code Style

**Rust:**

- Run `cargo fmt` and `cargo clippy` before committing
- Handle errors explicitly (avoid unwrap in production)
- Use descriptive names, add doc comments for public APIs

**TypeScript/React:**

- Strict TypeScript, avoid `any` types
- Functional components with hooks
- Tailwind CSS for styling
- Path aliases: `@/` → `./src/`

## CLI Parameters

Murmur supports command-line parameters for integration with scripts and autostart configurations.

**Implementation:** `cli.rs` (definitions), `main.rs` (parsing), `lib.rs` (applying), `signal_handle.rs` (shared logic)

| Flag                     | Description                                                |
| ------------------------ | ---------------------------------------------------------- |
| `--toggle-transcription` | Toggle recording on/off on a running instance              |
| `--cancel`               | Cancel the current operation on a running instance         |
| `--start-hidden`         | Launch without showing the main window (tray icon visible) |
| `--no-tray`              | Launch without system tray (closing window quits the app)  |
| `--debug`                | Enable debug mode with verbose (Trace) logging             |

**Key design decisions:**

- CLI flags are runtime-only overrides — they do NOT modify persisted settings
- Remote control flags work via `tauri_plugin_single_instance`: second instance sends args, then exits
- `send_transcription_input()` in `signal_handle.rs` is shared between signal handlers and CLI

## Debug Mode

Access debug features with `Cmd+Shift+D`.

## Platform Notes

- Accessibility permission is required for keyboard shortcuts and text insertion

## Troubleshooting

See the [Troubleshooting](BUILD.md#troubleshooting) section in BUILD.md.

## No-go

- Do not edit `src/bindings.ts` by hand. It is generated.
- Do not remove the `compile_error!` at the top of `src-tauri/src/lib.rs`. `main` is macOS-only. Windows work stays on `beta`.
- Do not add a Windows target, a `target_os` cfg, or a Windows CI job on `main`.
- Do not create GitHub labels.
- Do not silence a check with `#[allow]`, `eslint-disable`, a skipped test, or `--no-verify` unless the pull request explains why.
- Do not ask CodeRabbit to push commits. Autofix, fix-ci, and the "Prompt for AI Agents" block are disabled.
- A version bump updates `package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, and `src/content/release-notes/<version>.md` together. Do not update one of them alone.
- Do not reformat or refactor files the change does not need.

## Checks before pushing

Run `bun run check` before pushing. It is the CI `code quality` job. On macOS it also runs Clippy.

For a UI change, also run `bun run test:playwright`. For Rust behavior, from `src-tauri` run `cargo test --locked`.

`bun install` sets `core.hooksPath` to `.githooks`. The pre-commit hook runs the fast subset. Let it run. If you skip it, say why in the pull request.

## Pull requests

- Title: Conventional Commits (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`), focused on why. Branch the same way: `feat/short-description`.
- Body: `## Summary` (why, and the issue link), `## Changes`, `## Testing` (`bun run check` and anything else you ran). Visible UI changes include screenshots or recordings.
- Open an issue before a large pull request (a new feature, or a change across several modules).
- User-facing JSX text goes through i18next. Add the English key and keep the other locales in sync.
- Use the [bug report form](.github/ISSUE_TEMPLATE/bug_report.yml) and the [feature request form](.github/ISSUE_TEMPLATE/feature_request.yml). Translation rules are in [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md). The human workflow is [CONTRIBUTING.md](CONTRIBUTING.md).

## Review rounds

Address every review comment in one pass, then push once. A real bug fix in that pass includes a test that fails before the fix. A style comment does not need a commit.

A completed round is a review, then your fix. After two completed rounds, stop when the next review arrives. Summarize what is still open and wait for a maintainer. If the same comment comes back after a fix, reply on the thread instead of applying it again.

The `review loop` workflow may comment and apply the `review-loop` label. That label is created the first time it is needed. It means a maintainer decides. Do not start another fix round because of it.
