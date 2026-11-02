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
- `recently_cancelled.rs` - Restore after a cancel. A user's cancel (shortcut, tray, overlay, `--cancel`) of a dictation with at least 1 s of audio, while recording or before its paste is claimed, keeps the audio, and the transcript if the engine had answered, in memory only for 60 s, never on disk. One slot: it is zeroed when the minute ends, when a new recording starts, when another cancel replaces it, and on quit (the slot closes when a quit begins, in `begin_shutdown` or at `RunEvent::Exit`, and refuses every keep after, so a pipeline the quit cancels cannot refill it). A short-tap discard, a quit and a cancel after the paste keep nothing. A keep takes the overlay epoch of the dictation it cancels (`DictationEpoch`, counted by `show_overlay_state`): one that lands after a newer dictation took the overlay leaves that overlay alone (`show_cancelled_notice`), while the cancelled dictation's own pill shows even before the coordinator hides its overlay. The overlay shows "Dictation cancelled" with Restore for 5 s (Escape dismisses it), the tray "Restore cancelled dictation (m:ss)" for the whole minute while idle. The coordinator admits Restore only while idle (`Command::RestoreCancelled`); it runs the normal pipeline (`actions::restore`: engine unless the transcript was kept, dictionary, cleanup with the window focused now, history) and pastes into the app focused at Restore time. No setting
- `cleanup/` - AI cleanup prompt and the Codex responses client
- `engine/` - Which engine transcribes a dictation: a pure `route`/`reroute` table (Local or Cloud, one fallback at most, History records why) and `local/`, Parakeet on this Mac. `local/session.rs` loads the model when a recording starts, runs one request at a time and releases it after 10 idle minutes (never while recording) or once the models are deleted; long dictations are cut at pauses (`audio_toolkit/pauses.rs`) into pieces of one pass. Apple silicon runs it in the meetings helper's long-lived `serve` mode (`local/fluid.rs`, one WAV per request in `app_cache/local-stt`), Intel Macs on the CPU with transcribe.cpp (`local/cpu.rs`, the only `[target.'cfg(…)']` section `check:macos-only` allows). On Apple silicon the helper boosts the custom words with FluidAudio's CTC keyword spotting (`local/boost.rs`; the CTC model is an optional `local_models` source, fetched after the others are ready); Intel and Codex take no vocabulary. Debug builds fail the local run with `MURMUR_FORCE_LOCAL_FAILURE`. In `serve` the read loop runs at QoS user-interactive and each request in a task of priority `userInitiated`, or `utility` when the request says `"background": true` (dictation never does). Each load sends a 1 s silent prewarm request right after the helper reports ready, discards its text, and a failed prewarm fails the load. Every load logs `Local model ready in {ms} ms` (spawn to ready) and `Local model prewarm {ms} ms`, numbers only, to decide the 10-minute idle release from real loads. `.murmur-os-build`, next to `.murmur-ready`, holds the macOS build (`kern.osversion`, read with `sysctlbyname`) Core ML last compiled the models for, written after every successful `prepare`. At launch, ready models whose recorded build differs or is missing get one background `prepare`, state `optimizing` ("Optimizing the speech model for this macOS version…" in Transcription settings). Meanwhile a dictation goes to Codex when the user is signed in (History reason `local_preparing`), else waits for the compile. Intel Macs record no build and never optimize
- `local_models/` - The on-device speech models shared by dictation and meetings: a manifest pinned to Hugging Face commits with every file's SHA-256 (`pinned.rs`, written by `scripts/generate-model-manifest.ts`), a resumable downloader, and the `Missing → Downloading → Preparing → Ready` state behind the `*_local_model*` commands. Apple silicon keeps them in `app_data/meetings/models` as FluidAudio expects and loads them once with the helper's `prepare`; folders an earlier version downloaded are checked file by file
- `meetings/` - Meeting recording, on by default (`meetings_enabled`, switch in Advanced): microphone and system audio (Core Audio process tap in `swift/meeting_audio.swift`) streamed to two WAVs (synced to disk every 10 s; audio a full writer queue drops is counted in `dropped_ms`), the 3-hour cap (`cap.rs`, warned 5 minutes before; the `previous_id` and `part` columns of an unreleased build stay unused), a low-disk warning at start and a clean stop when a write fails (`coordinator.rs`), a power assertion that keeps an idle Mac awake while recording (`power.rs`; a lid closed still sleeps it, which is noticed on waking, told in a notification and counted in `slept_ms`), a notification once the writers dropped more than 2 s, and the `meetings.db` library. What a reset or an orphan sweep set aside next to it (`meetings.orphaned-*`, `meetings.corrupt-*`, `meetings.db.corrupt-*`) is deleted after 30 days, read from the name, and `meeting_audio_retention` (off by default) deletes the audio of processed meetings after 7, 30 or 90 days, at launch and once a day (`retention.rs`). A system track of pure digital zeros while the mic has speech and another app played sound (polled from Core Audio's per-process output state, `system_played`) flags the meeting (`system_audio_silent`), since that is what a denied System Audio Recording permission produces. A denied microphone is read from macOS at start (`microphone_denied`, alert `MicrophoneDenied`), never guessed from zeros, which a muted input gives too; each processing run logs its stage timings (`Meeting N timings:`). After a meeting stops, `processor.rs` splits each track at pauses (`chunking.rs`), sends the pieces to Codex (`transcribe.rs`, at most 3 at a time, resumable; after about 5 minutes without a connection the pieces left go to the local model when it is ready, else the queue waits and checks the connection, `offline.rs`; a meeting a crash cut off is processed at the next launch), and interleaves both tracks into the transcript (`spread.rs`, `merge.rs`). Once the transcript is complete, Codex writes the title and summary (`summary.rs`, people as `[[key]]` markers so a rename needs no rewrite), and the tracks are compressed into one `audio.m4a` (`archive.rs`, `swift/meeting_archive.swift`, on Intel too; after 3 failures the WAVs stay and are no longer requeued). Meanwhile `murmur-meetings-helper` (`swift/MurmurMeetingsHelper`, FluidAudio, a separate process; `helper.rs`, `protocol.rs`, the shared `local_models`, fetched with a meeting unless declined, `models.rs`) times the words heard on device and tells speakers apart, on system audio when the remote side was heard speaking there, else on the microphone (`analysis.rs`); `align/` places the Codex words at those times, `speakers.rs` gives each word its speaker and `bleed.rs` drops remote voices the microphone echoed. Without the models, or on Intel, each track keeps one speaker. Without the models `bleed.rs` still runs, on the sentences as placed by the levels, with a stricter match. Where the meeting played sound is read from the default output each second (`capture/route.rs`, `output_route`): a meeting on headphones throughout keeps every microphone word. When a call app (Zoom, Teams, Slack, FaceTime, Webex, Discord, or a browser on a meeting page, read through Accessibility) holds the microphone for 3 s, a panel of its own beside the overlay asks whether to record it, until it is answered, a meeting starts, or the call has left the microphone for 15 s (`calls.rs`, pure; `call_watch.rs` polls Core Audio's per-process input each second, macOS 14 on Apple silicon; setting `meeting_call_prompt`). While a meeting records, the same panel shows "{{app}} left the call. Murmur is still recording." once every call app that held the microphone 3 s during the meeting has been off it for 3 s (`ENDED_PROMPT_AFTER`), with **Stop and transcribe** (the normal stop) and **Keep recording** (asks again only after a call is back on the microphone for 3 s and leaves again). It hides when a call comes back or the meeting stops, and otherwise stays: unanswered, the meeting goes on to the 3-hour cap. A meeting started by hand with no call on the microphone gets no such prompt. An answer acts only while the prompt it answers shows (`CallAnswer`, `PromptKind`). It never starts or stops a meeting by itself, and a dictation never hides it. While a meeting records, `live/` makes a live transcript (setting `meeting_live_transcript`, on by default): each track writer hands the audio it writes to a `LiveTap`, which cuts it at the first pause after 6 s, or at 12 s with the next piece starting 2 s earlier (`live/chunker.rs`, the pauses of `audio_toolkit/pauses.rs`; pieces under -60 dBFS are not sent). `live/worker.rs` sends the pieces one at a time to the dictation's own `LocalEngine` (the same helper and model copy, never a second one, never Codex) as `background` requests asking for word times (`words` in `protocol.rs`), so the helper runs them at utility priority; `Session::transcribe_background` answers `Busy` without queuing while a dictation records, transcribes or holds the model, and the worker tries again 250 ms later, so a dictation waits at most for the one piece running. `live/seam.rs` joins pieces on a word both heard within 300 ms nearest the overlap's middle (else cuts at the middle and drops a repeated 2-12 word phrase) and publishes a sentence only once no later piece can change it; `live/echo.rs` holds microphone sentences until system audio is heard 1.5 s past them and hides those sharing 60 % of their words with system sentences around them (not on headphones). Over 30 s of speech waiting shows "Catching up"; over 120 s skips the oldest pieces with a gap entry; three failed pieces in a row stop the live text for the meeting. Sentences are appended to `live.ndjson` in the meeting's folder and sent as `MeetingLiveUpdated`; `get_live_transcript` returns them, or after a crash the file's, as `stopped`. The live text is a draft, never saved as the meeting's transcript: the processor deletes `live.ndjson` once it saves final turns. Without the local model (or on Intel) the live view says why and the transcript is made after Stop as before. The "Paste new meeting text" shortcut (`meeting_paste_new`, shipped unbound; a binding whose default is empty can be cleared again; like the meeting toggle it holds its keys only while meetings are on and yields them to an action that uses them, both listed in `shortcut::MEETING_BINDINGS`) pastes the sentences published since its last use, or of the last minute, as `You:`/`Others:` lines. `scripts/echo-probe` measures macOS voice processing on speakers (it plays sound; not shipped)
- `window_names.rs`, `accessibility.rs`, `field_excerpt.rs` - Name hints and the text around the cursor, read from the frontmost window for cleanup
- `updates/` - Automatic update checks while `update_checks_enabled` is on: at launch, every 4 hours and after the Mac wakes (`schedule.rs`, a wake is wall-clock time `Instant` did not see), one check at a time (`checks.rs`). The result is the one update status the tray item and the settings footer show (`update-status` event). Nothing downloads until the user clicks the footer's "Update available"
- `utils.rs` - Tray, window, and application helpers
- `search.rs` - Search across meetings and dictations (`search_meetings`, `search_history`, 50 results). `fts_query` turns the user's words into an FTS5 query: each word a prefix, quoted phrases kept, every piece quoted so no input is FTS5 syntax, nothing under 2 characters. A meeting matches when every word is somewhere in it (title, summary or any turn, `fts_terms` checked one at a time), a dictation when its text holds every word. The FTS5 tables (`meetings_fts` on title and summary and `meeting_turns_fts` in `meetings.db`, `history_fts` on `transcription_text` in `history.db`, accents folded, prefix indexes for 2 and 3 letters) are external-content tables kept current by triggers only, never by call sites, so cascades, reprocessing and retention deletes update them; their migrations stay append-only like the others. `meetings::store::search` and `HistoryManager::search_with_conn` take any `&Connection`, so a read-only one works. Meetings rank by where they matched (title, summary, transcript), then relevance, then date; a query matching more than 2,000 turns skips the per-turn ranking and reads the newest turns (`search_bench`, ignored, measures it). Hits come back as plain text with highlight ranges in UTF-16 units, never HTML; summary snippets show speaker names for `[[key]]` markers. Speaker names and `raw_text` are not searched. `meeting_turns_fts` points at the implicit rowid of `meeting_turns`: a `VACUUM` of `meetings.db` must be followed by a `'rebuild'` of it
- `settings_window.rs` - When the settings window is destroyed, the section it reopens on, and the actions it takes when it opens (see Window Lifecycle)

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

**Text passes:** deterministic fixes on every dictation, English and French only. They run only when the language is known: the dictation setting when it is `en*` or `fr*`, in `auto` a stopword vote on the text (at least 3 hits and twice the other language), otherwise not at all, since fillers are real words elsewhere ("um" in Portuguese, "er" in German). `before_cleanup` (fillers, spoken line breaks) runs before cleanup; `raw_only` (spoken punctuation) runs only on text cleanup did not edit (off, skipped, failed), because cleanup reads those words in context ("trois virgule cinq"). A command never follows a determiner ("a new line of products"). Meetings do not use them. There is no switch; a new language comes in its own pull request with its own must-not-change corpus.

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

Settings live in `settings_store.json` in the app data folder, as `{"settings": …}` pretty-printed, the shape the store plugin of earlier versions wrote, so an older Murmur still reads the file (`settings/file.rs`). Every change is written by a background thread through `durable::write`, so a burst (a slider drag) costs the caller no disk sync and lands as its latest value; `update_settings_checked` waits for its write to report a failure, and quitting waits for a pending one. `durable::write` takes a temporary file in the same folder, synced, renamed over the file, then the folder synced; a symlink stays a link): a crash or power cut leaves the old file or the new one. Write any file that must survive a crash through `durable::write`, never `fs::write`. A field this version cannot read falls back to its default; the file as it was is kept first as `settings_store.json.unreadable-<12 hex of its SHA-256>` (mode 0600, once per content).

Panics on any thread go to `murmur.log` as one `error` record (`panic_log.rs`): thread, `file:line:col`, the executable's load address and a backtrace, never the panic message, which can quote dictated text. Release builds are stripped; resolve the addresses with the build's `murmur.dSYM` artifact: `atos -o murmur.dSYM -l <load address> <address>…`.

The settings cover:

- Keyboard shortcuts (configurable; the recording shortcut is hold-to-talk or double-tap hands-free, see `dictation_gesture.rs`)
- Audio devices (microphone/output selection)
- Codex / ChatGPT session for cloud transcription
- Audio feedback and overlay options

### Single Instance Architecture

The app enforces single instance behavior — launching when already running brings the settings window to front rather than creating a new process. Remote control flags (`--toggle-transcription`, etc.) work by launching a second instance that sends args to the running instance via `tauri_plugin_single_instance`, then exits.

### Window Lifecycle

While Murmur sits in the menu bar, the recording pill is the only live webview.

- Closing the settings window destroys it, web process included, when the tray icon is visible, `initialize_shortcuts` has succeeded, and the page reported that it left onboarding (`report_settings_view`). Otherwise closing hides it: with `--no-tray` or the tray icon off, before the shortcuts run, on any onboarding step, and while the page holds it (`holdSettingsWindow` in `src/lib/`) for work whose later steps run in the page, such as an update install and its relaunch; the release destroys a window closed meanwhile. A hidden start builds the window so its page can run the onboarding checks and start the shortcuts. Rust destroys it once both are done and it is still hidden.
- Every open goes through `show_main_window`, which builds the window again if it is gone. The new page opens on the section the user left (`get_last_settings_section`, kept in memory, not across launches).
- What happens while the window is closed is kept as a pending action in Rust (`MainAction`). The event still goes out. The page takes the action when it hears the event, and once after it listens on mount (`take_main_action`), so it runs exactly once. The tray's "Check for updates" and the switch to the local engine work this way. Dictation errors are not kept: the pill and the notifications show them.
- The call prompt panel is created when a prompt shows and destroyed when it hides, together with its entry in the nspanel store. Its page asks for the prompt (`get_call_prompt`) once it listens.
- Locales load lazily: a window loads `en` and the active language before its first render (`i18nReady` in `src/i18n`). History, Meetings, Insights and the What's New modal load with their first open.

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

CI runs the Rust tests with `cargo nextest run --locked --profile ci` (one process per test; `.config/nextest.toml` fails a test still running after 2 minutes), then the doctests with `cargo test --locked --doc`. `cargo test --locked` still works locally. A test that needs the state of another test is a bug: fix the coupling, or put the tests in a nextest `test-group` with `max-threads = 1`.

For a change to how meetings record or process, also run the 3-hour soak on a Mac with the models downloaded: from `src-tauri`, `cargo test --locked meetings_soak -- --ignored --nocapture` (about half an hour, headless). It rewrites `src/meetings/soak-report.md`.

`bun install` sets `core.hooksPath` to `.githooks`. The pre-commit hook runs the fast subset. Let it run. If you skip it, say why in the pull request.

## Release smoke test

`build.yml`, which the release, main-build, build-test and PR test-build workflows share, launches the release bundle it built, right after the build step (tauri-action has already put the assets on the draft release) and before the build artifacts are uploaded, so a failed launch stops `publish-release` and a release never goes out with an app that dies at launch. Debug builds are skipped: they write `../src/bindings.ts` when they start and only run from `src-tauri`. `scripts/smoke-macos-app.sh <Murmur.app>` starts the executable with `--start-hidden` in a throwaway `HOME`: the app starts as on a first launch, onboarding not done, so the microphone and shortcuts stay off and nothing asks for a permission. It waits up to 15 s (`SMOKE_TIMEOUT`; 60 s for the x86_64 bundle, which Rosetta translates at its first launch on the arm64 runner) for the `Setup complete (version …)` line that `lib.rs` logs as the last step of setup, keeps watching for 3 s (`SMOKE_SETTLE`), then stops the app with SIGTERM. The job fails when the line does not come, when the app exits, when `murmur.log` holds a panic record (`panicked at`, from `panic_log.rs`) or when SIGTERM does not stop the app within 10 s. The log tail and the app's own output (a dyld error lands there, not in the log) go to the job log. Keep the `Setup complete` line, and keep it last in setup: the script reads it.

The script runs the executable directly, so it checks that the signed bundle launches, not Gatekeeper's check of a downloaded app. Run it on a CI runner. The single-instance lock lives in `/tmp` and the login item belongs to the bundle identifier, so on a Mac with Murmur installed it refuses to start while Murmur runs and it switches Open at Login off; test a copy of the bundle with another `CFBundleIdentifier` there. If the first runs on a cold runner are too slow, raise `SMOKE_TIMEOUT` in the step; do not retry. A crash later than the 3 s of watching passes the step.

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
