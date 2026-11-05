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
bun run check             # CI code-quality job: audit, translations, platform guard, versions, ESLint, format (Prettier + cargo fmt --check), build, test typecheck. Clippy on macOS.
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
- `overlay.rs` - Native macOS recording overlay window. A dictation shows a 34-point orb, on both platforms: its ring wobbles with the voice while recording and runs an arc while it transcribes (no arc under reduced motion). Every notice (copy, mic lost, cancelled with Restore, error) opens from that circle and starts with a small orb glyph. One notice at a time: a copy pill that also lost the microphone shows first and the microphone warning follows once it goes (`Notice::Copy { mic_lost_next }`, `notice_after`). The call prompt and meeting pill are never shown on Windows
- `signal_handle.rs` - `send_transcription_input()` reusable function
- `recently_cancelled.rs` - Restore after a cancel. A user's cancel (shortcut, tray, overlay, `--cancel`) of a dictation with at least 1 s of audio, while recording or before its paste is claimed, keeps the audio, and the transcript if the engine had answered, in memory only for 60 s, never on disk. One slot: it is zeroed when the minute ends, when a new recording starts, when another cancel replaces it, and on quit (the slot closes when a quit begins, in `begin_shutdown` or at `RunEvent::Exit`, and refuses every keep after, so a pipeline the quit cancels cannot refill it). A short-tap discard, a quit and a cancel after the paste keep nothing. A keep takes the overlay epoch of the dictation it cancels (`DictationEpoch`, counted by `show_overlay_state`): one that lands after a newer dictation took the overlay leaves that overlay alone (`show_cancelled_notice`), while the cancelled dictation's own pill shows even before the coordinator hides its overlay. The overlay shows "Dictation cancelled" with Restore for 5 s (on macOS Escape dismisses it; on Windows the pill only times out, as the copy pill does, and Escape is never taken), the tray "Restore cancelled dictation (m:ss)" for the whole minute while idle. The coordinator admits Restore only while idle (`Command::RestoreCancelled`); it runs the normal pipeline (`actions::restore`: engine unless the transcript was kept, dictionary, cleanup with the window focused now, history) and pastes into the app focused at Restore time. No setting
- `cleanup/` - AI cleanup prompt and the Codex responses client
- `engine/` - Which engine transcribes a dictation: a pure `route`/`reroute` table (Local or Cloud, one fallback at most, History records why) and `local/`, Parakeet on this Mac. `local/session.rs` loads the model when a recording starts, runs one request at a time and releases it after 10 idle minutes (never while recording) or before the models are deleted, as Windows may refuse to delete the file the CPU model maps; long dictations are cut at pauses (`audio_toolkit/pauses.rs`) into pieces of one pass. Apple silicon runs it in the meetings helper's long-lived `serve` mode (`local/fluid.rs`, one WAV per request in `app_cache/local-stt`), Intel Macs on the CPU with transcribe.cpp (`local/cpu.rs`, the only `[target.'cfg(…)']` section `check:macos-only` allows). On Apple silicon the helper boosts the custom words with FluidAudio's CTC keyword spotting (`local/boost.rs`; the CTC model is an optional `local_models` source, fetched after the others are ready); Intel and Codex take no vocabulary. Debug builds fail the local run with `MURMUR_FORCE_LOCAL_FAILURE`
- `local_models/` - The on-device speech models shared by dictation and meetings: a manifest pinned to Hugging Face commits with every file's SHA-256 (`pinned.rs`, written by `scripts/generate-model-manifest.ts`), a resumable downloader, and the `Missing → Downloading → Preparing → Ready` state behind the `*_local_model*` commands. Apple silicon keeps them in `app_data/meetings/models` as FluidAudio expects and loads them once with the helper's `prepare`; folders an earlier version downloaded are checked file by file. The speech model is Parakeet Ultra (`parakeet-ultra`). A folder left ready with Parakeet TDT 0.6B v3 (`manifest::PREVIOUS`, recognised by its `.murmur-ready`) stays ready and in use while Ultra downloads beside it, with no state change and no UI; once Ultra is checked and prepared, the ready marker names it and the v3 folder is deleted (again at each launch, after a crash in between, or when Windows refused to delete a v3 file a loaded model still mapped). A failed fetch keeps v3 and tries again at the next launch. The helper's `--asr ultra|v3` and the GGUF path of Intel Macs and Windows follow `local_models::asr_in_use`. Ultra writes `<unk>` where its training text had characters its tokenizer cannot spell (quotes, brackets, dashes): `engine/local/unknown.rs` removes them from the text and the timed words alike at the helper protocol's parser, since FluidAudio keeps them; transcribe.cpp drops them itself on Intel Macs and Windows (`keep_special_tags` off). A delete during the background fetch of Ultra cancels it and deletes everything
- `meetings/` - Meeting recording, on by default (`meetings_enabled`, switch in Advanced): microphone and system audio (Core Audio process tap in `swift/meeting_audio.swift`) streamed to two WAVs (synced to disk every 10 s; audio a full writer queue drops is counted in `dropped_ms`), the 3-hour cap (`cap.rs`, warned 5 minutes before; the `previous_id` and `part` columns of an unreleased build stay unused), a low-disk warning at start and a clean stop when a write fails (`coordinator.rs`), a power assertion that keeps an idle Mac awake while recording (`power.rs`; a lid closed still sleeps it, which is noticed on waking, told in a notification and counted in `slept_ms`), a notification once the writers dropped more than 2 s, and the `meetings.db` library. What a reset or an orphan sweep set aside next to it (`meetings.orphaned-*`, `meetings.corrupt-*`, `meetings.db.corrupt-*`) is deleted after 30 days, read from the name, and `meeting_audio_retention` (off by default) deletes the audio of processed meetings after 7, 30 or 90 days, at launch and once a day (`retention.rs`). A system track of pure digital zeros while the mic has speech and another app played sound (polled from Core Audio's per-process output state, `system_played`) flags the meeting (`system_audio_silent`), since that is what a denied System Audio Recording permission produces. A denied microphone is read from macOS at start (`microphone_denied`, alert `MicrophoneDenied`), never guessed from zeros, which a muted input gives too; each processing run logs its stage timings (`Meeting N timings:`). After a meeting stops, `processor.rs` splits each track at pauses (`chunking.rs`), sends the pieces to Codex (`transcribe.rs`, at most 3 at a time, resumable; after about 5 minutes without a connection the pieces left go to the local model when it is ready, else the queue waits and checks the connection, `offline.rs`; a meeting a crash cut off is processed at the next launch), and interleaves both tracks into the transcript (`spread.rs`, `merge.rs`). Once the transcript is complete, Codex writes the title and summary (`summary.rs`, people as `[[key]]` markers so a rename needs no rewrite), and the tracks are compressed into one `audio.m4a` (`archive.rs`, `swift/meeting_archive.swift`, on Intel too; after 3 failures the WAVs stay and are no longer requeued). Meanwhile `murmur-meetings-helper` (`swift/MurmurMeetingsHelper`, FluidAudio, a separate process; `helper.rs`, `protocol.rs`, the shared `local_models`, fetched with a meeting unless declined, `models.rs`) times the words heard on device and tells speakers apart, on system audio when the remote side was heard speaking there, else on the microphone (`analysis.rs`); `align/` places the Codex words at those times, `speakers.rs` gives each word its speaker and `bleed.rs` drops remote voices the microphone echoed. Without the models, or on Intel, each track keeps one speaker. Without the models `bleed.rs` still runs, on the sentences as placed by the levels, with a stricter match. Where the meeting played sound is read from the default output each second (`capture/route.rs`, `output_route`): a meeting on headphones throughout keeps every microphone word. When a call app (Zoom, Teams, Slack, FaceTime, Webex, Discord, or a browser on a meeting page, read through Accessibility) holds the microphone for 3 s, a panel of its own beside the overlay asks whether to record it, until it is answered, a meeting starts, or the call has left the microphone for 15 s (`calls.rs`, pure; `call_watch.rs` polls Core Audio's per-process input each second, macOS 14 on Apple silicon; setting `meeting_call_prompt`). It never starts or stops a meeting by itself, and a dictation never hides it. `scripts/echo-probe` measures macOS voice processing on speakers (it plays sound; not shipped)
- `window_names.rs`, `accessibility.rs`, `field_excerpt.rs` - Name hints and the text around the cursor, read from the frontmost window for cleanup. On Windows, `uia.rs` (UI Automation, background threads only) stands in for `accessibility.rs`
- `leading_space.rs` - The rule for one space when a dictation is pasted right after a word (setting `leading_space`, on by default): `needs_leading_space` over the last two grapheme bases before the caret, pure and platform-neutral. `clipboard::paste` reads them for CtrlV and Direct pastes only, on a blocking thread with a 150 ms budget, and not at all when the setting is off. Any failure, timeout, secure field, non-empty selection, missing caret or truncated context adds no space. macOS reads the keyboard target through `accessibility.rs` (`field_excerpt::char_before`); Windows reads the foreground app's focused element through `uia.rs` (`char_before_caret`, TextPattern, `field_excerpt::char_before_selection`) and never in an app that exposes no caret
- `updates/` - Automatic update checks while `update_checks_enabled` is on: at launch, every 4 hours and after the Mac wakes (`schedule.rs`, a wake is wall-clock time `Instant` did not see), one check at a time (`checks.rs`). The result is the one update status the tray item and the settings footer show (`update-status` event). Nothing downloads until the user clicks the footer's "Update available"
- `utils.rs` - Tray, window, and application helpers
- `search.rs` - Search across meetings and dictations (`search_meetings`, `search_history`, 50 results). `fts_query` turns the user's words into an FTS5 query: each word a prefix, quoted phrases kept, every piece quoted so no input is FTS5 syntax, nothing under 2 characters. A meeting matches when every word is somewhere in it (title, summary or any turn, `fts_terms` checked one at a time), a dictation when its text holds every word. The FTS5 tables (`meetings_fts` on title and summary and `meeting_turns_fts` in `meetings.db`, `history_fts` on `transcription_text` in `history.db`, accents folded, prefix indexes for 2 and 3 letters) are external-content tables kept current by triggers only, never by call sites, so cascades, reprocessing and retention deletes update them; their migrations stay append-only like the others. `meetings::store::search` and `HistoryManager::search_with_conn` take any `&Connection`, so a read-only one works. Meetings rank by where they matched (title, summary, transcript), then relevance, then date; a query matching more than 2,000 turns skips the per-turn ranking and reads the newest turns (`search_bench`, ignored, measures it). Hits come back as plain text with highlight ranges in UTF-16 units, never HTML; summary snippets show speaker names for `[[key]]` markers. Speaker names and `raw_text` are not searched. `meeting_turns_fts` points at the implicit rowid of `meeting_turns`: a `VACUUM` of `meetings.db` must be followed by a `'rebuild'` of it. On Windows only History search is reachable: `history_fts` ships with `rusqlite`'s bundled SQLite (FTS5 included), its migration builds the index from existing rows when `history.db` opens, and the meetings half stays compiled but unused because Meetings are off there (no `meetings.db` exists, so none is created or migrated)

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

**Pipeline Processing:** Audio → quiet-speech gain (`audio_toolkit/gain.rs`) → local or Codex transcription (`engine/`) → dictionary (`audio_toolkit/text.rs`) → text passes (`audio_toolkit/text_passes/`) → AI cleanup through the Codex session (`cleanup/`, raw text on any failure) → Clipboard/Paste

**Text passes:** deterministic fixes on every dictation, English and French only. They run only when the language is known: the dictation setting when it is `en*` or `fr*`, in `auto` a stopword vote on the text (at least 3 hits and twice the other language), otherwise not at all, since fillers are real words elsewhere ("um" in Portuguese, "er" in German). `before_cleanup` (fillers, spoken line breaks) runs before cleanup; `raw_only` (spoken punctuation) runs only on text cleanup did not edit (off, skipped, failed), because cleanup reads those words in context ("trois virgule cinq"). A command never sits in a noun phrase: it keeps its words after a determiner ("a new line of products"), after "whole", "brand", "entire", "toute" or "véritable" ("a brand new line"), and, for the break, question, exclamation and full-stop commands only, before "of", "over", "about", "than" or the French "de", "du", "sur" ("a question mark over the launch"); the comma and "virgule" keep firing there ("yes comma of course"). The intensifier list is short on purpose, since an adjective also ends a list item ("big comma small comma") and "completely" or "tout" end the clause before a real command. Meetings do not use them. There is no switch; a new language comes in its own pull request with its own must-not-change corpus.

**Short dictations:** a dictation under 20 words (UAX #29 word count, so Chinese or Japanese count per word) skips the default AI cleanup and is pasted at once, recorded in History as `CleanupOutcome::ShortDictation` without a raw text, and logged as `AI cleanup skipped: short dictation ({n} words)`. Cleanup still runs on it when `cleanup_short_dictations` is on or the user chose a transformation: today only a Formal or Casual `dictation_style` (`actions.rs`, `cleanup_plan`). A future user-chosen transformation (custom prompt, translation, per-app prompt) must keep cleanup on short dictations too.

**State Flow:** Zustand → Tauri Command → Rust State → Persistence (`settings_store.json` through `durable::write`)

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

Settings live in `settings_store.json` in the app data folder, as `{"settings": …}` pretty-printed, the shape the store plugin of earlier versions wrote. `settings/file.rs` keeps the top-level map and writes changes through a background thread. A burst of changes lands as its latest value without a disk sync on the caller. Background saves and exit flush use the same drain loop, which writes the latest value until nothing is dirty. The first failed write stops the drain and keeps the value dirty for a later flush, without an endless retry loop. `update_settings_checked` waits for its write and reports failure. Parsing and normalization retain beta's Windows restrictions and its one-time window-context migration. Startup recognizes staged copies of the main file, `.bak`, and `.unreadable-<hash>` files. It removes them only when the filename's PID is proven dead. Live owners and unknown process status keep their files, because a headless instance can run alongside the GUI.

| Rule                                    | Enforcement                                                                    |
| --------------------------------------- | ------------------------------------------------------------------------------ |
| Use one background and exit save drain. | `Shared::write_pending` and `flush_drains_a_change_queued_after_its_snapshot`. |

Use `durable::write` for crash-safe replacement, never a truncating `fs::write`. It writes a unique temporary file in the same folder, flushes the bytes, closes the handle, and replaces the target. macOS keeps POSIX permissions and syncs the directory after rename. Windows uses `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`, without a copy fallback or directory fsync. Readers can briefly block replacement on Windows, so sharing-related failures retry the same rename for at most one second. Windows files inherit the folder's ACL, as the auth writer does, and read-only targets return an error without retry. Same-volume replacement protects against partial files after process termination. Survival of a power loss depends on the filesystem and drive honoring flushes and is not proven by process-kill tests. A symlink stays a link. A field this version cannot read falls back to its default after the original bytes are kept once as `settings_store.json.unreadable-<12 hex of its SHA-256>`, mode 0600 on macOS and with the folder's ACL on Windows.

Both paths passed to the Windows rename are absolute extended-length paths, including `\\?\UNC\` for a network share. Do not pass a plain UTF-16 encoding of the path to Win32. Rust's filesystem calls support long paths even when the app manifest has no `longPathAware` flag, and the unique staging suffix can push the source past MAX_PATH.

Panics on any thread go to `murmur.log` as one `error` record in `panic_log.rs`. The record has the thread, `file:line:col`, executable image base, and full backtrace. It never includes the panic message, which can quote dictated text. Windows reads the image base with `GetModuleHandleW(NULL)` rather than macOS's dyld. Windows symbolication needs the matching build's PDB, with the logged image base used to adjust frame addresses for ASLR. The release profile remains stripped with no debug information, and `windows-beta.yml` retains no PDB. This port does not add symbol packaging or the macOS dSYM upload.

The settings cover:

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
- Windows work stays on `beta`. Run `bun run check:platforms`. Keep native APIs behind the existing macOS and Windows cfg boundaries. The Windows beta keeps meetings and the updater disabled.

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

CI runs the Rust tests with `cargo nextest run --locked --profile ci`, one process per test. `.config/nextest.toml` terminates a test after two 60-second slow periods. Both macOS and Windows then run doctests with `cargo test --locked --doc`. Windows CPU-model tests also use nextest. `.github/scripts/check-nextest-timeout.ps1` proves that the real CI profile terminates a hung test and reports its name. `cargo test --locked` still works locally. Tests must not depend on another test's global state. Tests of the panic hook and logger run in a private child process.

For a change to how meetings record or process, also run the 3-hour soak on a Mac with the models downloaded: from `src-tauri`, `cargo test --locked meetings_soak -- --ignored --nocapture` (about half an hour, headless). It rewrites `src/meetings/soak-report.md`.

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
