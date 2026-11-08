import React from "react";
import ReactDOM from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  HistoryHit,
  MeetingImport,
  MeetingLiveUpdated,
  MeetingProgress,
  MeetingRuntime,
  OpenTarget,
} from "../../src/bindings";
import { events } from "../../src/bindings";
import { findMatches } from "../../src/lib/meetings/search";
import { resolveSignIn, handleAccountCommand } from "./settings-pages/account";
import {
  handleEngineCommand,
  setModel,
  switchIfAsked,
  dropSwitch,
} from "./settings-pages/engine";
import { HISTORY, HOUR, VERSIONS } from "./settings-pages/history";
import {
  DETAILS,
  emitLive,
  handleMeetingCommand,
  holdMeetingDetail,
  holdMeetingLoads,
  releaseMeetingDetail,
  releaseMeetingLoads,
} from "./settings-pages/meetings";
import { MODEL_COMMANDS, MODELS } from "./settings-pages/models";
import {
  IMPORT_RUNNING,
  importMode,
  language,
  meetingRecording,
  openMeeting,
  params,
  pendingMainActions,
  type Keyboard,
} from "./settings-pages/params";
import { RESPONSES } from "./settings-pages/responses";
import {
  KEYBOARDS,
  SETTINGS,
  savesBindings,
  windows,
} from "./settings-pages/settings";

declare global {
  interface Window {
    settingsPages: {
      unhandledCommands: string[];
      calls: { command: string; args: unknown }[];
      emitProgress: (progress: MeetingProgress) => Promise<void>;
      emitMeetingsUpdated: (id: number) => Promise<void>;
      emitMeetingState: (runtime: MeetingRuntime) => Promise<void>;
      emitLive: (update: MeetingLiveUpdated) => Promise<void>;
      openMeetingNotice: (target: OpenTarget) => Promise<void>;
      /** Keeps list loads waiting, each with the list as it was asked for. */
      holdMeetingLoads: () => void;
      releaseMeetingLoads: () => void;
      holdMeetingDetail: (id: number) => void;
      releaseMeetingDetail: (id: number) => void;
      /** The local model finished downloading, as the backend reports it. */
      finishModelDownload: () => void;
      /** The browser sign-in saved a session. */
      resolveSignIn: () => void;
      /** What the backend's last update check found. */
      emitUpdateStatus: (available: string | null) => Promise<void>;
      /** The update found is no longer offered by the next check. */
      withdrawUpdate: () => void;
      /** The user switched to another keyboard layout. */
      switchKeyboard: (keyboard: Keyboard) => void;
      /** Keeps the keyboard layout reads asked from now on unanswered. */
      holdKeyboard: (held: boolean) => void;
      /** Answers the held reads with the layout as it was when asked. */
      releaseKeyboard: () => void;
    };
  }
}

let importResolve: ((id: number | null) => void) | null = null;
/** `busy`: another window's import is running, so the call is refused. */
let importBusyRefused = false;
/** `history=fails`: the first load fails, a retry reads the entries. */
let historyFails = params.get("history") === "fails";
let updateWithdrawn = false;
let pendingMeetingTarget: OpenTarget | null =
  openMeeting === null
    ? null
    : openMeeting === "all"
      ? "meetings"
      : { meeting: Number(openMeeting) };
let keyboard: Keyboard =
  params.get("keyboard") === "azerty" ? "azerty" : "qwerty";
let keyboardHeld = false;
let heldKeyboardReads: (() => void)[] = [];

window.settingsPages = {
  unhandledCommands: [],
  calls: [],
  emitProgress: (progress) => events.meetingProgress.emit(progress),
  emitMeetingsUpdated: (id) => events.meetingsUpdated.emit({ id }),
  emitMeetingState: (runtime) => events.meetingStateChanged.emit(runtime),
  emitLive,
  openMeetingNotice: (target) => {
    pendingMeetingTarget = target;
    return emit("open-meetings");
  },
  holdMeetingLoads,
  releaseMeetingLoads,
  holdMeetingDetail,
  releaseMeetingDetail,
  finishModelDownload: () => {
    setModel(MODELS.ready);
    switchIfAsked();
  },
  resolveSignIn,
  emitUpdateStatus: (available) => emit("update-status", { available }),
  withdrawUpdate: () => {
    updateWithdrawn = true;
  },
  switchKeyboard: (layout) => {
    keyboard = layout;
  },
  holdKeyboard: (held) => {
    keyboardHeld = held;
  },
  releaseKeyboard: () => {
    const reads = heldKeyboardReads;
    heldKeyboardReads = [];
    reads.forEach((answer) => answer());
  },
};

mockIPC(
  (command, args) => {
    window.settingsPages.calls.push({ command, args });
    const meeting = handleMeetingCommand(
      command,
      (args ?? {}) as Record<string, unknown>,
    );
    if (meeting.handled) return meeting.value;
    const engineCommand = handleEngineCommand(
      command,
      (args ?? {}) as Record<string, unknown>,
    );
    if (engineCommand.handled) return engineCommand.value;
    const accountCommand = handleAccountCommand(command);
    if (accountCommand.handled) return accountCommand.value;
    if (command === "get_key_labels" || command === "get_recorded_key_names") {
      // Read when asked, answered when released.
      const layout = KEYBOARDS[keyboard];
      const value =
        command === "get_key_labels" ? layout.labels : layout.recorded;
      if (!keyboardHeld) return value;
      return new Promise((resolve) => {
        heldKeyboardReads.push(() => resolve(value));
      });
    }
    if (command === "change_binding" && savesBindings) {
      const { id, binding } = args as { id: string; binding: string };
      const bindings = SETTINGS.bindings ?? {};
      return {
        success: true,
        binding: { ...bindings[id], current_binding: binding },
        error: null,
      };
    }
    // The backend emits the models' new state before the command returns.
    const models = MODEL_COMMANDS[command];
    if (models) setModel(models);
    // Stopping or deleting the model drops a pending switch to it.
    if (command !== "download_local_model" && models) dropSwitch();
    // So does the meeting's.
    if (command === "start_meeting") {
      void events.meetingStateChanged.emit({
        id: 5,
        started_at_ms: Date.now(),
        state: "recording",
        stops_at_ms: Date.now() + 3 * HOUR * 1000,
        tracks: "both",
      });
    }
    if (command === "stop_meeting") {
      const stopped = DETAILS.get(4);
      if (meetingRecording && stopped) {
        Object.assign(stopped, { stage: "transcribing", status: "processing" });
        void emitLive({ appended: [], id: 4, state: { state: "stopped" } });
      }
      void events.meetingStateChanged.emit({ state: "idle" });
    }
    // The update's download stops at 42% so its progress stays on screen.
    if (command === "plugin:updater|download_and_install") {
      const { onEvent } = args as {
        onEvent: { onmessage: (event: unknown) => void };
      };
      onEvent.onmessage({ data: { contentLength: 100 }, event: "Started" });
      onEvent.onmessage({ data: { chunkLength: 42 }, event: "Progress" });
      return new Promise(() => {});
    }
    if (command === "get_meeting_import") {
      return importMode === "running" ||
        (importMode === "busy" && importBusyRefused)
        ? IMPORT_RUNNING
        : null;
    }
    if (command === "import_meeting_audio") {
      if (importMode === "busy") {
        importBusyRefused = true;
        return Promise.reject("busy");
      }
      if (importMode?.startsWith("fails=")) {
        return Promise.reject(importMode.slice("fails=".length));
      }
      const shown: MeetingImport = { ...IMPORT_RUNNING, fraction: 0.42 };
      void events.meetingImportChanged.emit(shown);
      return new Promise((resolve) => {
        importResolve = resolve;
      });
    }
    if (command === "cancel_meeting_import") {
      void events.meetingImportChanged.emit(null);
      importResolve?.(null);
      importResolve = null;
      return null;
    }
    if (command === "plugin:updater|check" && updateWithdrawn) return null;
    if (command === "check_for_updates" && updateWithdrawn) {
      return { available: null };
    }
    if (command === "take_main_action") {
      return pendingMainActions.delete((args as { action: string }).action);
    }
    if (command === "take_meetings_action") {
      const target = pendingMeetingTarget;
      pendingMeetingTarget = null;
      return target;
    }
    if (command === "get_history_entries" && historyFails) {
      historyFails = false;
      return Promise.reject("database disk image is malformed");
    }
    if (command === "retry_history_entry_transcription") {
      const { id } = args as { id: number };
      return HISTORY.find((entry) => entry.id === id);
    }
    if (command === "restore_history_version") {
      const { id, versionId } = args as { id: number; versionId: number };
      const version = VERSIONS.find((candidate) => candidate.id === versionId);
      const entry = HISTORY.find((candidate) => candidate.id === id);
      if (!entry || !version) return null;
      // A restore swaps the two texts, as the backend does.
      [entry.transcription_text, version.transcription_text] = [
        version.transcription_text,
        entry.transcription_text,
      ];
      return { ...entry };
    }
    if (command === "search_history") {
      const query = String((args as { query: string }).query);
      return HISTORY.flatMap((entry): HistoryHit[] => {
        const ranges = findMatches(entry.transcription_text, query);
        return ranges.length > 0 ? [{ entry, ranges }] : [];
      });
    }
    if (command in RESPONSES) return RESPONSES[command];
    if (!window.settingsPages.unhandledCommands.includes(command)) {
      window.settingsPages.unhandledCommands.push(command);
    }
    return null;
  },
  { shouldMockEvents: true },
);

Object.assign(window, {
  __TAURI_OS_PLUGIN_INTERNALS__: windows
    ? {
        arch: "x86_64",
        eol: "\r\n",
        exe_extension: "exe",
        family: "windows",
        os_type: "windows",
        platform: "windows",
        version: "10.0.26100",
      }
    : {
        arch: "aarch64",
        eol: "\n",
        exe_extension: "",
        family: "unix",
        os_type: "macos",
        platform: "macos",
        version: "15.0.0",
      },
});

const renderFixture = async () => {
  const [{ default: App }, { default: i18n }] = await Promise.all([
    import("../../src/App"),
    import("../../src/i18n"),
  ]);
  await i18n.changeLanguage(language);
  document.documentElement.dataset.accentColor = SETTINGS.accent_color;
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    params.get("strict") === "1" ? (
      <React.StrictMode>
        <App />
      </React.StrictMode>
    ) : (
      <App />
    ),
  );
};

void renderFixture();
