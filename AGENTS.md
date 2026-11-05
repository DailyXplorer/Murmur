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
- `updates/` - Automatic update checks while `update_checks_enabled` is on: at launch, every 4 hours and after the Mac wakes (`schedule.rs`, a wake is wall-clock time `Instant` did not see), one check at a time (`checks.rs`). The result is the one update status the tray item and the settings footer show (`update-status` event). Nothing downloads until the user clicks the footer's "Update available". On Windows the checks are on by default as well (`update_checks_turned_on_once` turns them on once for a store the earlier betas wrote with them off), the footer pill, the Advanced "Updates" switch and the tray item exist, and the feed is the Windows-only one described under "Windows beta updates". Before the installer starts, the page calls `prepare_update_install`: on Windows the updater plugin ends the process right after launching the installer, which skips the exit sequence that saves settings, so the command leaves the update-relaunch marker and flushes the pending settings write
- `utils.rs` - Tray, window, and application helpers
- `search.rs` - Search across meetings and dictations (`search_meetings`, `search_history`, 50 results). `fts_query` turns the user's words into an FTS5 query: each word a prefix, quoted phrases kept, every piece quoted so no input is FTS5 syntax, nothing under 2 characters. A meeting matches when every word is somewhere in it (title, summary or any turn, `fts_terms` checked one at a time), a dictation when its text holds every word. The FTS5 tables (`meetings_fts` on title and summary and `meeting_turns_fts` in `meetings.db`, `history_fts` on `transcription_text` in `history.db`, accents folded, prefix indexes for 2 and 3 letters) are external-content tables kept current by triggers only, never by call sites, so cascades, reprocessing and retention deletes update them; their migrations stay append-only like the others. `meetings::store::search` and `HistoryManager::search_with_conn` take any `&Connection`, so a read-only one works. Meetings rank by where they matched (title, summary, transcript), then relevance, then date; a query matching more than 2,000 turns skips the per-turn ranking and reads the newest turns (`search_bench`, ignored, measures it). Hits come back as plain text with highlight ranges in UTF-16 units, never HTML; summary snippets show speaker names for `[[key]]` markers. Speaker names and `raw_text` are not searched. `meeting_turns_fts` points at the implicit rowid of `meeting_turns`: a `VACUUM` of `meetings.db` must be followed by a `'rebuild'` of it. On Windows only History search is reachable: `history_fts` ships with `rusqlite`'s bundled SQLite (FTS5 included), its migration builds the index from existing rows when `history.db` opens, and the meetings half stays compiled but unused because Meetings are off there (no `meetings.db` exists, so none is created or migrated)
- `mcp/` - `murmur mcp`, the read-only Model Context Protocol server AI assistants launch (hand-written JSON-RPC 2.0 over stdio, one message per line, on `serde_json`; see "Read-only MCP server" below). `main.rs` runs `mcp::run()` and exits before `murmur::run`, so the process starts no Tauri, window, tray, single-instance plugin, microphone or network. `access.rs` reads the two switches, `rpc.rs` frames and dispatches, `tools.rs` holds the tools (five on macOS, two on Windows: `MEETINGS_EXIST` in `mod.rs` is false there), `setup.rs` writes the command path the settings page shows, `sample.rs` seeds an invented data directory for the tests (`tests/mcp_stdio.rs` runs the built binary against it)
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

### Window Lifecycle

While Murmur sits in the menu bar (the notification area on Windows), the recording pill is the only live webview.

- Closing the settings window destroys it, web process included, when the tray icon is visible, `initialize_shortcuts` has succeeded, and the page reported that it left onboarding (`report_settings_view`). Otherwise closing hides it: with `--no-tray` or the tray icon off, before the shortcuts run, on any onboarding step, and while the page holds it (`holdSettingsWindow` in `src/lib/`) for work whose later steps run in the page, such as an update install and its relaunch; the release destroys a window closed meanwhile. A hidden start builds the window so its page can run the onboarding checks and start the shortcuts. Rust destroys it once both are done and it is still hidden. With no window left, `RunEvent::ExitRequested` without an exit code is prevented (`lib.rs`), so the tray keeps the process alive; only the tray's Quit, which carries a code, exits.
- Every open goes through `show_main_window`, which builds the window again if it is gone (on Windows from a thread of its own, as WebView2 deadlocks a window built inside the tray menu or single-instance handler). The new page opens on the section the user left (`get_last_settings_section`, kept in memory, not across launches), as long as that section is still enabled (Meetings never is on Windows). On Windows `show_main_window` places the new window like any other show (`window_placement.rs`: the monitor under the cursor, fitted to its work area), it opens at the size the destroyed one had (`remember_size`, in memory, not when it was maximized, full screen or minimized), and it takes the accent icon again, which the taskbar reads from the window.
- A destroyed page cannot undo what it did in Rust. Hotkeys it suspended while a field captured keys are put back when the window is destroyed (`resume_bindings_if_suspended`, off the main thread).
- What happens while the window is closed is kept as a pending action in Rust (`MainAction`). The event still goes out. The page takes the action when it hears the event, and once after it listens on mount (`take_main_action`), so it runs exactly once. The tray's "Check for updates" and the switch to the local engine work this way. Dictation errors are not kept: the pill and the notifications show them.
- The call prompt panel (macOS only: Windows shows no call prompt) is created when a prompt shows and destroyed when it hides, together with its entry in the nspanel store. Its page asks for the prompt (`get_call_prompt`) once it listens.
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

## UI rules

**Settings rows show a title and a control, nothing else.** Any explanation goes in the row's info bubble, the (i) tooltip of `SettingContainer` (its `description`, `RadioOption`'s `description`, `DescriptionTooltip` elsewhere). `SettingContainer` has no inline mode and no section heading takes a subtitle: no helper text under or beside a title anywhere in Settings. A dynamic note (a reason a row is disabled, a size) is merged into the bubble's text, not rendered below.

The only text that may stay inline is a live status that is not an explanation, or an error the user must act on: the ChatGPT row's state ("Signed in", "Session expired, sign in again"), a failed model download's reason, a shortcut registration error, a download's progress bar. A state that sits under a row title goes in `SettingContainer`'s `detail`, in a paragraph marked `data-live-status` (the ChatGPT row); `tests/settings-no-inline-help.spec.ts` fails on any other paragraph under a row title. The download failure and the shortcut error are error paragraphs of their own, outside the title block. Dialogs, the onboarding, history entries and the announcement card (`LocalSttCard`) are not settings rows.

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
| `mcp` (subcommand)       | Read-only MCP server over stdio for AI assistants          |

**Key design decisions:**

- CLI flags are runtime-only overrides — they do NOT modify persisted settings
- Remote control flags work via `tauri_plugin_single_instance`: second instance sends args, then exits
- `send_transcription_input()` in `signal_handle.rs` is shared between signal handlers and CLI

## Read-only MCP server

`murmur mcp` lets an AI assistant (Claude Code, Codex, Claude Desktop) list, search and read meetings. The assistant launches it as a child process; it is the same binary as the app. Settings → Advanced → "AI assistants" holds the switch and a copyable setup for each assistant, built from `std::env::current_exe()` (`get_mcp_setup`, `src/lib/mcp.ts`).

**Security model. Keep every point when changing it:**

- **Opt-in.** `mcp_enabled` is off by default. `recent_dictations` needs its own second switch, `mcp_include_dictations`, also off, because dictations are more personal than meetings; it needs `mcp_enabled` too. The server reads `settings_store.json` itself on every request (`access.rs`) and treats anything but a JSON `true` as off, so a switch turned off applies to a running server. Off, `tools/list` is empty and every `tools/call` is a tool error saying where to turn it on. Dictations off, `recent_dictations` is absent from `tools/list` and refused, and `search` with `scope: "all"` looks at meetings only.
- **Read-only.** Both databases are opened with `SQLITE_OPEN_READ_ONLY` and `PRAGMA query_only = ON`, `busy_timeout` 2 s (`tools::open_read_only`). The server never migrates: a database with pending migrations (the app was updated and not opened) gets "Open Murmur once to finish updating". No tool writes, and no tool takes a path: the data directory is `~/Library/Application Support/com.dailyxplorer.murmur` (the identifier is pinned to `tauri.conf.json` by a test), or `MURMUR_MCP_DATA_DIR` set by whoever launches the process, which is how the tests and measurements point it at invented data.
- **stdio only.** No TCP or Unix socket listener, no HTTP, no network code. Nothing but the launching process can talk to it. Stdout carries protocol messages only; logs never hold transcripts.
- **What each tool returns** (JSON in one `text` content item): `list_meetings` (id, title, start time, duration, status; paged by `before_id`), `search` (meetings: id, title, start time, plain-text snippets, `first_match_seq`; dictations: id, time, text; the same SQL as the app's search through `store::search` and `HistoryManager::search_with_conn`), `get_transcript` (turns with speaker display names, times, text, paged by `from_seq`, at most 400), `get_summary` (title and Markdown with `[[key]]` replaced by the names), `recent_dictations` (id, time, delivered text). Speaker names are the current ones, resolved at read time. **Never returned:** audio, audio paths or file names, raw (pre-cleanup) text, window names, field excerpts, settings, account data.
- **Threat model.** Any process running as the user can already read these files, so this grants no new OS access. The real risk is the assistant sending what it reads to its own cloud model. The opt-in, the second switch and the wording of the setting are the controls; do not weaken them, and do not add a tool that writes, executes, or reads a path.
- **Protocol.** The handshake-based revisions `2025-11-25`, `2025-06-18`, `2025-03-26` and `2024-11-05`; a client asking for another one is answered with the newest. A tool that cannot answer returns a result with `isError: true`; only an unknown tool is a JSON-RPC error. The stateless revision (`server/discover`, per-request `_meta`) is not implemented.

**On Windows**, which has no Meetings (`MEETINGS_EXIST` false), only dictations exist, so `tools/list` holds `search` (scope `dictations` only) and `recent_dictations`, and `list_meetings`, `get_transcript` and `get_summary` are "Unknown tool". Both switches keep their rules: the first alone lists nothing, and dictations stay behind their own switch. The data directory is `%APPDATA%\com.dailyxplorer.murmur` (Roaming, what Tauri's `app_data_dir` gives; `data_root` in `mod.rs` reads `APPDATA`). The settings page shows a Windows command with double quotes and the real `murmur.exe` path without a `\\?\` prefix (`setup.rs`). The release exe has the GUI subsystem, so the assistant's `CreateProcess` with redirected pipes hands it stdin and stdout and no console window flashes; with no pipes (run by hand in a terminal) Rust's std treats the invalid handles as an empty stdin and discards writes, so it prints nothing and exits. `main.rs` reaches `mcp::run()` before any Tauri code, so a running app's single-instance plugin is never involved. `tests/mcp_stdio.rs` cannot prove this (a test build has a console), so `.github/scripts/windows-smoke.ps1` runs the installed release `murmur.exe mcp` with redirected pipes beside the running app. Do not call `AllocConsole`: it would flash a window.

Adding a tool: describe it in `tools::list` with `readOnlyHint`, read through `open_database` only, add it to the "never returned" review above, and add its round trip to `tests/mcp_stdio.rs`. `cargo test --release --test mcp_stdio mcp_timing -- --ignored --nocapture` prints the startup and search timings.

## Debug Mode

Access debug features with `Cmd+Shift+D`.

## Platform Notes

- Accessibility permission is required for keyboard shortcuts and text insertion
- Windows work stays on `beta`. Run `bun run check:platforms`. Keep native APIs behind the existing macOS and Windows cfg boundaries. The Windows beta keeps meetings disabled. Its updater is on, with a feed of its own.

### Windows beta updates

- **Feed.** `https://github.com/DailyXplorer/Murmur/releases/download/windows-beta/latest.json`, an asset of one rolling pre-release tagged `windows-beta`, set only in `src-tauri/tauri.windows.conf.json` (`plugins.updater.endpoints`, with `windows.installMode` `passive`: a progress bar, no administrator prompt, per user). The base `tauri.conf.json` (the macOS endpoint, `releases/latest/download/latest.json`, and the `pubkey`) is not touched, so a Mac never reads the Windows feed and a Windows install never reads the macOS one. The feed holds `windows-*` keys only, and `.github/scripts/publish-updater-manifest.mjs` keeps refusing any other key than `darwin-` on the macOS release. `bun run check:platforms` asserts all of it.
- **Installers.** Each beta is a pre-release `v1.9.0-beta.N` (`Murmur_1.9.0-beta.N_x64-setup.exe` and its `.sig`), never the latest release. `latest.json` points at it. The rolling release is titled "update feed" and holds nothing to download.
- **Version scheme.** The four version files carry `1.9.0-beta.N` at every beta (release-prep PR). The signature records the build's version and the updater compares semver, where `1.9.0-beta.7` is below `1.9.0`, so the announced version must equal the built one. `check:versions` takes the release notes of the base version (`1.9.0.md`) for a pre-release.
- **Signing.** The macOS updater key pair: the `pubkey` is the shared one, and the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` reach only the build step of `beta-release.yml`. `createUpdaterArtifacts` stays `false` in the Windows overlay, so ordinary CI builds sign nothing; the release workflow turns it on with `--config src-tauri/updater-artifacts.conf.json`. There is no Authenticode certificate: SmartScreen still asks on a first install by hand.
- **Publishing.** Pushing the annotated tag `v<version>-beta.N` runs `beta-release.yml` from the tagged commit (a `workflow_dispatch` could never run: the file is not on `main`). Guards: the pusher is the repository owner, the tag equals the `tauri.conf.json` version, the commit is on `beta`, the release does not exist. Jobs: `build` (only job with the signing key, no write token), `manifest` (notes from the tag message, `latest.json`, offline signature check against the `pubkey`), `publish` (the versioned pre-release first, then `latest.json` uploaded to `windows-beta`), `verify` (the public feed, the installer, its signature and checksum, the macOS feed still darwin-only, the latest release still not a beta). A pull request that touches the workflow or the feed tooling runs `build` and `manifest` as a dry run and publishes nothing. Tooling and tests: `.github/scripts/windows-updater-manifest.mjs`, `minisign.mjs`, `bun run test:windows-updater`.
- **First update.** Beta 6 and older have no updater: the first auto-updating build is installed by hand once.

## Troubleshooting

See the [Troubleshooting](BUILD.md#troubleshooting) section in BUILD.md.

## No-go

- Do not edit `src/bindings.ts` by hand. It is generated.
- Do not remove the `compile_error!` at the top of `src-tauri/src/lib.rs`. `main` is macOS-only. Windows work stays on `beta`.
- Do not add a Windows target, a `target_os` cfg, or a Windows CI job on `main`.
- Do not create GitHub labels.
- Do not silence a check with `#[allow]`, `eslint-disable`, a skipped test, or `--no-verify` unless the pull request explains why.
- Do not ask CodeRabbit to push commits. Autofix, fix-ci, and the "Prompt for AI Agents" block are disabled.
- A version bump updates `package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, and `src/content/release-notes/<version>.md` together. Do not update one of them alone. On `beta` a Windows beta is `1.9.0-beta.N` in the four version files and uses the notes of `1.9.0`.
- Do not reformat or refactor files the change does not need.

## Checks before pushing

Run `bun run check` before pushing. It is the CI `code quality` job. On macOS it also runs Clippy.

For a UI change, also run `bun run test:playwright`. For Rust behavior, from `src-tauri` run `cargo test --locked`.

CI runs the Rust tests with `cargo nextest run --locked --profile ci`, one process per test. `.config/nextest.toml` terminates a test after two 60-second slow periods. Both macOS and Windows then run doctests with `cargo test --locked --doc`. Windows CPU-model tests also use nextest. `.github/scripts/check-nextest-timeout.ps1` proves that the real CI profile terminates a hung test and reports its name. `cargo test --locked` still works locally. Tests must not depend on another test's global state. Tests of the panic hook and logger run in a private child process. Every test process starts UI Automation cold, and processes run side by side, so a test that reads a window through it opens the window with `uia::test_window`, which waits until UI Automation lists the window's controls, and passes the read a deadline it never reaches unless the budget is what the test checks; a budget is tested with a stepped clock or a deadline already past, never with `sleep` and an elapsed-time bound.

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
