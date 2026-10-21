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

**Linting and Formatting (run before committing):**

```bash
bun run lint              # ESLint for frontend
bun run lint:fix          # ESLint with auto-fix
bun run format            # Prettier + cargo fmt
bun run format:check      # Check formatting without changes
bun run format:frontend   # Prettier only
bun run format:backend    # cargo fmt only
```

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
- `meetings/` - Meeting recording, on by default (`meetings_enabled`, switch in Advanced): microphone and system audio (Core Audio process tap in `swift/meeting_audio.swift`) streamed to two WAVs, the 3-hour cap, a low-disk warning at start (`coordinator.rs`), and the `meetings.db` library. A system track of pure digital zeros while the mic has speech and another app played sound (polled from Core Audio's per-process output state, `system_played`) flags the meeting (`system_audio_silent`), since that is what a denied System Audio Recording permission produces. A denied microphone is read from macOS at start (`microphone_denied`, alert `MicrophoneDenied`), never guessed from zeros, which a muted input gives too; each processing run logs its stage timings (`Meeting N timings:`). After a meeting stops, `processor.rs` splits each track at pauses (`chunking.rs`), sends the pieces to Codex (`transcribe.rs`, at most 3 at a time, resumable), and interleaves both tracks into the transcript (`spread.rs`, `merge.rs`). Once the transcript is complete, Codex writes the title and summary (`summary.rs`, people as `[[key]]` markers so a rename needs no rewrite), and the tracks are compressed into one `audio.m4a` (`archive.rs`, `swift/meeting_archive.swift`). Meanwhile `murmur-meetings-helper` (`swift/MurmurMeetingsHelper`, FluidAudio, a separate process; `helper.rs`, `protocol.rs`, models in `models.rs`) times the words heard on device and tells speakers apart, on system audio when the remote side was heard speaking there, else on the microphone (`analysis.rs`); `align/` places the Codex words at those times, `speakers.rs` gives each word its speaker and `bleed.rs` drops remote voices the microphone echoed. Without the models, or on Intel, each track keeps one speaker
- `window_names.rs`, `accessibility.rs` - Name hints read from the frontmost window
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

**Pipeline Processing:** Audio → quiet-speech gain (`audio_toolkit/gain.rs`) → Codex transcription → dictionary → AI cleanup through the Codex session (`cleanup/`, raw text on any failure) → Clipboard/Paste → correction learning (`dictionary/`)

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
    ├── de/, es/, fr/, ja/, ru/, zh/, ...
    └── ...
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

## Contributor workflow

- Pull requests should explain the change, its boundaries, and the exact checks run. Include screenshots or recordings for visible changes.
- Use [`.github/ISSUE_TEMPLATE/bug_report.md`](.github/ISSUE_TEMPLATE/bug_report.md) for bug reports.
- Use [`.github/ISSUE_TEMPLATE/feature_request.md`](.github/ISSUE_TEMPLATE/feature_request.md) for a feature whose outcome and done-when are already clear.
- Follow [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md) for translation changes and [CONTRIBUTING.md](CONTRIBUTING.md) for the general workflow.
- Use conventional commit prefixes (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`). Focus the message on why the change is needed.

## Checks before pushing

Run the tests your change adds or modifies, plus a build: `bun run build` for frontend changes, and from `src-tauri` `cargo test <your tests>` for Rust changes. CI runs the full suite (lint, formatting, translations, Clippy, all Rust tests, Playwright) and is the safety net.

`bun install` sets `core.hooksPath` to `.githooks`, whose pre-commit hook runs the fast checks. Let the hook run. If you skip it, say why in the pull request.

## Pull requests

- Title: Conventional Commits (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`), focused on why.
- Body sections: `## Summary`, `## Changes`, `## Testing`. Visible UI changes include screenshots or recordings.
- Keep the change focused. Leave lockfiles, generated `src/bindings.ts`, and unrelated refactors alone.
- `main` is macOS-only. Windows work stays on the `beta` branch. Keep the `compile_error!` guard in `lib.rs`.
- User-facing JSX text goes through i18next. Add the English key and keep the other locales in sync.
- A version bump updates `package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, and `src/content/release-notes/<version>.md` together.
- Fix a failing check. A `#[allow]`, `eslint-disable`, skipped test, or `--no-verify` needs an explanation in the pull request.

## Review rounds

Review and fix rounds continue until a review comes back clean. There is no round limit. Only real bugs (correctness, security, behavior) block a merge; style never does.

- Fix each real bug with a test that fails before the fix.
- If the same comment comes back after a fix, do not apply it again: check whether the fix works or the comment is wrong, and reply on the thread.
- If one file has had bugs in three rounds, simplify that part (reuse a proven approach) instead of patching it again.
- After 20 rounds, stop and ask a maintainer.

The `review loop` workflow comments and applies the `review-loop` label in those three cases, so a maintainer or the lead agent takes a look. The label is created the first time it is needed. It never means "stop fixing bugs".

Do not ask CodeRabbit to push the fixes. Autofix, fix-ci, and the "Prompt for AI Agents" block are disabled so a review cannot commit onto the branch.
