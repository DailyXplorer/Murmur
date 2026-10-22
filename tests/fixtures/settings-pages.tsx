import React from "react";
import ReactDOM from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  AccentColor,
  AnalysisSupport,
  AppSettings,
  AudioDevice,
  DictationStyle,
  DownloadFailure,
  Engine,
  HistoryEntry,
  InsightsSummary,
  MeetingDetail,
  LocalModelStatus,
  MeetingListItem,
  MeetingProgress,
  MeetingRuntime,
  MeetingStage,
  SpeakerAnalysis,
} from "../../src/bindings";
import { events } from "../../src/bindings";

declare global {
  interface Window {
    settingsPages: {
      unhandledCommands: string[];
      calls: { command: string; args: unknown }[];
      emitProgress: (progress: MeetingProgress) => Promise<void>;
      emitMeetingsUpdated: (id: number) => Promise<void>;
      emitMeetingState: (runtime: MeetingRuntime) => Promise<void>;
      /** Keeps list loads waiting, each with the list as it was asked for. */
      holdMeetingLoads: () => void;
      releaseMeetingLoads: () => void;
      /** The local model finished downloading, as the backend reports it. */
      finishModelDownload: () => void;
      /** The browser sign-in saved a session. */
      resolveSignIn: () => void;
      /** What the backend's last update check found. */
      emitUpdateStatus: (available: string | null) => Promise<void>;
    };
  }
}

const params = new URLSearchParams(window.location.search);
const language = params.get("lang") ?? "en";
const showOnboarding = params.get("onboarding") === "1";
/** On by default, as in the app; `meetings=0` turns them off. */
const meetingsEnabled = params.get("meetings") !== "0";
const systemAudioSilent = params.get("systemAudioSilent") === "1";
const systemAudioPermission = params.get("systemAudioPermission") ?? "granted";
const audioBytes = Number(params.get("audioBytes") ?? 34_600_000);
const meetingRecording = params.get("meeting") === "recording";
const systemAudio = params.get("systemAudio") ?? "available";
const meetingFailed = params.get("meetingFailed") === "1";
const analysis = params.get("analysis") ?? "available";
/** `failed` or `pending`; ready by default. */
const meetingSummary = params.get("summary");
/** The stage saved for the processing meeting; `transcribing` by default. */
const processingStage = (params.get("stage") ?? "transcribing") as MeetingStage;
/**
 * `held=1` keeps the queued meetings behind a Codex rate-limit hold: the
 * processing one, and a ready one whose summary is pending.
 */
const heldUntilMs =
  params.get("held") === "1" ? Date.now() + 2 * 60 * 60 * 1000 : null;
/** Enough older meetings to fill more than one page of the list. */
const manyMeetings = params.get("manyMeetings") === "1";
const speakerAnalysis = (params.get("speakers") ??
  "separated") as SpeakerAnalysis;
/** `local` or `cloud`; cloud by default, as for every existing user. */
const engine = (params.get("engine") ?? "cloud") as Engine;
/** `engineRefused=1`: the backend refuses every engine change. */
const engineRefused = params.get("engineRefused") === "1";
/** `account=none`: no ChatGPT session on this Mac; `account=expired`: one
 * that can no longer be refreshed. */
let account = params.get("account") ?? "signedIn";
/** `signOut=fails`: the backend cannot remove the session. */
const signOutFails = params.get("signOut") === "fails";
/** The browser sign-in under way, as the backend runs it. */
let signIn: {
  resolve: (value: null) => void;
  reject: (error: string) => void;
} | null = null;
/** `cleanup=off` turns AI cleanup off; `style` is the stored dictation style. */
const cleanupEnabled = params.get("cleanup") !== "off";
const dictationStyle = (params.get("style") ?? "normal") as DictationStyle;
/** `styleRefused=1`: the backend refuses every style change. */
const styleRefused = params.get("styleRefused") === "1";
/** `card=1` shows the one-time local transcription card. */
const cardShown = params.get("card") === "1";
/** `history=fallback`: entries transcribed by the engine not chosen. */
const historyFallback = params.get("history") === "fallback";
/** `localFallback=1`: part of a meeting was transcribed on this Mac. */
const localFallbackMs = params.get("localFallback") === "1" ? 12 * 60_000 : 0;
/** The updater finds a new version, which the footer offers. */
const updateAvailable = params.get("update") === "1";
/** The download as the manifest gives it, on Intel (`unsupported`) or Apple silicon. */
const MODEL_BYTES = analysis === "unsupported" ? 739_508_576 : 505_044_208;
const failed = (reason: DownloadFailure, kept_bytes = 0): LocalModelStatus => ({
  kept_bytes,
  size_bytes: MODEL_BYTES,
  state: { reason, state: "failed" },
});
const MODELS: Record<string, LocalModelStatus> = {
  downloading: {
    kept_bytes: 0,
    size_bytes: MODEL_BYTES,
    state: {
      done_bytes: 212_000_000,
      state: "downloading",
      total_bytes: MODEL_BYTES,
    },
  },
  missing: {
    kept_bytes: 0,
    size_bytes: MODEL_BYTES,
    state: { state: "missing" },
  },
  /** A cancelled download whose partial files stay to be resumed. */
  cancelled: {
    kept_bytes: 212_000_000,
    size_bytes: MODEL_BYTES,
    state: { state: "missing" },
  },
  preparing: {
    kept_bytes: 0,
    size_bytes: MODEL_BYTES,
    state: { state: "preparing" },
  },
  ready: { kept_bytes: 0, size_bytes: MODEL_BYTES, state: { state: "ready" } },
  failed: failed({ kind: "no_internet" }),
  serverDown: failed({ kind: "server_unavailable" }),
  damaged: failed({ kind: "damaged" }),
  disk: failed({ kind: "disk" }),
  noSpace: failed({
    free_bytes: 120_000_000,
    kind: "not_enough_space",
    needed_bytes: 555_548_628,
  }),
  /** Downloaded and checked, then would not load: every file stays. */
  prepareFailed: failed({ kind: "prepare_failed" }, MODEL_BYTES),
};
const MODEL_COMMANDS: Record<string, LocalModelStatus> = {
  download_local_model: MODELS.downloading,
  cancel_local_model_download: MODELS.missing,
  delete_local_model: MODELS.missing,
};
// Intel Macs (`unsupported`) download the model for dictation, without
// speaker detection; macOS 14 has neither.
const modelsStatus: LocalModelStatus =
  analysis !== "os_too_old"
    ? MODELS[params.get("models") ?? "ready"]
    : {
        kept_bytes: 0,
        size_bytes: MODEL_BYTES,
        state: {
          state: "unsupported",
          support: analysis as AnalysisSupport,
        },
      };
const theme = (params.get("theme") ?? "light") as AppSettings["theme"];

const APP_DATA_DIR =
  "/Users/user/Library/Application Support/com.dailyxplorer.murmur";
const LOG_DIR = "/Users/user/Library/Logs/com.dailyxplorer.murmur";

const MICROPHONES: AudioDevice[] = [
  { index: "default", is_default: true, name: "Default" },
  { index: "1", is_default: false, name: "Built-in Microphone" },
];
const OUTPUTS: AudioDevice[] = [
  { index: "default", is_default: true, name: "Default" },
  { index: "1", is_default: false, name: "Built-in Speakers" },
];

const SETTINGS: AppSettings = {
  accent_color: (params.get("accent") ?? "pink") as AccentColor,
  ai_cleanup_enabled: cleanupEnabled,
  always_on_microphone: false,
  app_language: language,
  append_trailing_space: false,
  audio_feedback: true,
  audio_feedback_volume: 0.6,
  auto_submit: false,
  auto_submit_key: "enter",
  autostart_enabled: true,
  bindings: {
    cancel: {
      current_binding: "escape",
      default_binding: "escape",
      description: "Cancel the current recording",
      id: "cancel",
      name: "Cancel",
    },
    meeting_toggle: {
      current_binding: "ctrl+option+command+m",
      default_binding: "ctrl+option+command+m",
      description: "Starts recording a meeting, or stops the one in progress.",
      id: "meeting_toggle",
      name: "Start or stop a meeting",
    },
    paste_last: {
      current_binding: "option+shift+v",
      default_binding: "option+shift+v",
      description: "Paste the last transcription",
      id: "paste_last",
      name: "Paste last transcript",
    },
    transcribe: {
      current_binding: "option+space",
      default_binding: "option+space",
      description: "Record and transcribe",
      id: "transcribe",
      name: "Transcribe",
    },
  },
  clamshell_microphone: null,
  clipboard_handling: "dont_modify",
  custom_words: ["Murmur", "Tauri", "Playwright"],
  debug_mode: params.get("debug") !== "0",
  dictation_style: dictationStyle,
  experimental_enabled: true,
  extra_recording_buffer_ms: 0,
  history_limit: 50,
  learned_words: [{ heard_as: "play right", word: "Playwright" }],
  local_model_download_wanted: false,
  local_stt_card_dismissed: !cardShown,
  log_level: "info",
  meeting_capture_mode: "both",
  meetings_enabled: meetingsEnabled,
  mute_while_recording: false,
  onboarding_completed: !showOnboarding,
  overlay_position: "bottom",
  overlay_style: "minimal",
  paste_delay_after_ms: 0,
  paste_delay_ms: 60,
  paste_method: "ctrl_v",
  recording_retention_period: "preserve_limit",
  reliable_paste: true,
  selected_channel: null,
  selected_language: params.get("dictation") ?? "auto",
  selected_microphone: "Default",
  selected_output_device: "Default",
  show_tray_icon: true,
  show_whats_new_on_update: false,
  sound_theme: "marimba",
  start_hidden: false,
  theme,
  switch_to_local_when_ready: false,
  transcription_engine: engine,
  update_checks_enabled: true,
  whats_new_last_seen_version: "1.3.7",
  whisper_mode: false,
  window_names_enabled: true,
  word_correction_threshold: 0.18,
};

const HOUR = 3600;
const NOW = 1_760_000_000;
const HISTORY: HistoryEntry[] = [
  {
    cleanup: "cleaned",
    engine: "cloud",
    fallback_reason: null,
    file_name: "murmur-1.wav",
    id: 3,
    raw_text: "um so the quarterly report is ready for review",
    saved: true,
    timestamp: NOW,
    title: "Quarterly report",
    transcription_text: "The quarterly report is ready for review.",
  },
  {
    cleanup: null,
    engine: "cloud",
    fallback_reason: null,
    file_name: "murmur-2.wav",
    id: 2,
    raw_text: null,
    saved: false,
    timestamp: NOW - HOUR,
    title: "Meeting notes",
    transcription_text: "Move the design review to Thursday afternoon.",
  },
  {
    cleanup: "failed",
    engine: "cloud",
    fallback_reason: null,
    file_name: "murmur-3.wav",
    id: 1,
    raw_text: "remember to water the plants",
    saved: false,
    timestamp: NOW - 26 * HOUR,
    title: "Reminder",
    transcription_text: "remember to water the plants",
  },
];

/** One entry per fallback reason, then one transcribed by the chosen engine. */
const FALLBACK_HISTORY: HistoryEntry[] = (
  [
    ["cloud", "model_not_ready"],
    ["cloud", "local_failed"],
    ["cloud", "language_not_supported"],
    ["local", "cloud_limit_reached"],
    ["local", "cloud_failed"],
    ["local", null],
  ] as const
).map(([used, fallback_reason], index) => ({
  ...HISTORY[1],
  engine: used,
  fallback_reason,
  file_name: `murmur-fallback-${index}.wav`,
  id: 10 + index,
  timestamp: NOW - index * HOUR,
}));

const INSIGHTS: InsightsSummary = {
  apps_used: 9,
  categories: [
    { category: "ai_prompts", count: 64 },
    { category: "work_messages", count: 31 },
    { category: "emails", count: 12 },
  ],
  categorized_dictations: 107,
  heatmap: Array.from({ length: 182 }, (_, index) => {
    const date = new Date(Date.UTC(2025, 3, 1) + index * 86_400_000);
    const count = index % 6 === 0 ? 0 : (index * 5) % 9;
    return {
      count,
      date: date.toISOString().slice(0, 10),
      level: count === 0 ? 0 : Math.min(4, Math.ceil(count / 2)),
    };
  }),
  minutes_saved: 273,
  streaks: { current: 6, longest: 21 },
  total_dictations: 412,
  total_words: 15_380,
  words_per_minute: 138,
};

const MINUTE_MS = 60_000;
const MEETINGS: MeetingListItem[] = [
  {
    duration_ms: meetingRecording ? 0 : 47 * MINUTE_MS + 12_000,
    id: 4,
    started_at_ms: NOW * 1000 - 50 * MINUTE_MS,
    status: meetingRecording ? "recording" : "processing",
    title: null,
    tracks: "both",
  },
  {
    duration_ms: 72 * MINUTE_MS + 5_000,
    id: 3,
    started_at_ms: NOW * 1000 - 26 * HOUR * 1000,
    status: meetingFailed ? "failed" : "ready",
    title: "Point hebdo produit",
    tracks: "both",
  },
  {
    duration_ms: 9 * MINUTE_MS + 41_000,
    id: 2,
    started_at_ms: NOW * 1000 - 74 * HOUR * 1000,
    status: "interrupted",
    title: null,
    tracks: "both",
  },
  ...(manyMeetings
    ? Array.from({ length: 60 }, (_, index) => ({
        duration_ms: 30 * MINUTE_MS,
        id: -index - 1,
        started_at_ms: NOW * 1000 - (100 + index) * HOUR * 1000,
        status: "ready" as const,
        title: `Older meeting ${index + 1}`,
        tracks: "both" as const,
      }))
    : []),
];
const MEETING_RUNTIME: MeetingRuntime = meetingRecording
  ? {
      id: 4,
      started_at_ms: Date.now() - 12 * MINUTE_MS - 34_000,
      state: "recording",
      stops_at_ms: Date.now() + 3 * HOUR * 1000,
      tracks: "both",
    }
  : { state: "idle" };

const SECOND_MS = 1000;
const turn = (speaker_key: string, start_s: number, text: string) => ({
  end_ms: (start_s + 4) * SECOND_MS,
  low_confidence: true,
  speaker_key,
  start_ms: start_s * SECOND_MS,
  text,
});
/** A fictional summary, as Codex writes it: people as markers. */
const SUMMARY = `Weekly product check-in between [[me]] and [[s1]].

**Home page**
- The new home page is ready for Thursday's review
  - Two screens left, nothing blocking

**Payment integration**
- Already in testing; production planned for early November

**Action items**
- [[s1]] — tell the support team
- [[s1]] — send the invite for the sales demo — next week`;

const summaryOf = (item: MeetingListItem) => {
  if (
    item.id !== 3 ||
    item.status !== "ready" ||
    meetingSummary === "pending"
  ) {
    return {
      summary: null,
      summary_error: null,
      summary_status: "pending" as const,
    };
  }
  return meetingSummary === "failed"
    ? {
        summary: null,
        summary_error: "rate_limited" as const,
        summary_status: "failed" as const,
      }
    : {
        summary: SUMMARY,
        summary_error: null,
        summary_status: "ready" as const,
      };
};

const queuedForSummary = (item: MeetingListItem) =>
  item.status === "ready" && summaryOf(item).summary_status === "pending";

const detailOf = (item: MeetingListItem): MeetingDetail => ({
  ...item,
  ...summaryOf(item),
  audio: item.id === 3 ? "compressed" : "tracks",
  audio_bytes: item.id === 3 ? audioBytes : 0,
  system_audio_silent: item.id === 3 && systemAudioSilent,
  error: item.status === "failed" ? "parts_failed" : null,
  held_until_ms:
    item.status === "processing" || queuedForSummary(item) ? heldUntilMs : null,
  local_fallback_ms: item.id === 3 ? localFallbackMs : 0,
  speakers:
    item.id === 3
      ? [
          { color_index: 0, default_name: "Me", key: "me", name: "Me" },
          {
            color_index: 1,
            default_name: "Speaker 2",
            key: "s1",
            name: "Speaker 2",
          },
        ]
      : [],
  speaker_analysis: item.id === 3 ? speakerAnalysis : null,
  stage: item.status === "processing" ? processingStage : null,
  turns:
    item.id === 3
      ? [
          turn("s1", 4, "Bonjour à tous, on commence par le point produit ?"),
          turn(
            "me",
            9,
            "Oui. La nouvelle page d'accueil est prête pour la revue de jeudi.",
          ),
          turn("me", 15, "Il reste deux écrans à finaliser, rien de bloquant."),
          turn(
            "s1",
            31,
            "Parfait. Et l'intégration avec la plateforme de paiement ?",
          ),
          turn(
            "me",
            38,
            "Elle est déjà en test. On vise une mise en production début novembre.",
          ),
          ...(meetingFailed
            ? [{ ...turn("_system", 600, ""), end_ms: 1_200_000 }]
            : []),
          turn("s1", 1_262, "Super, je préviens l'équipe support."),
          turn(
            "me",
            1_270,
            "Let's also schedule a quick demo for the sales team next week.",
          ),
          turn("s1", 1_278, "Sounds good, I'll send the invite."),
        ]
      : [],
});
const DETAILS = new Map(MEETINGS.map((item) => [item.id, detailOf(item)]));

/** Commands whose answer depends on their arguments or changes the data. */
let meetingLoadsHeld: Promise<void> | null = null;
let releaseMeetingLoads = () => {};

const handleMeetingCommand = (
  command: string,
  args: Record<string, unknown>,
): { handled: boolean; value?: unknown } => {
  const meeting = DETAILS.get(args.id as number);
  switch (command) {
    case "get_meetings": {
      // Pages of at most 50, newest first, after the `cursor` id.
      const limit = Math.min((args.limit as number | null) ?? 50, 50);
      const after = MEETINGS.slice(
        MEETINGS.findIndex(({ id }) => id === args.cursor) + 1,
      );
      const page = {
        has_more: after.length > limit,
        meetings: after.slice(0, limit),
      };
      return {
        handled: true,
        value: meetingLoadsHeld ? meetingLoadsHeld.then(() => page) : page,
      };
    }
    case "delete_meeting":
      MEETINGS.splice(
        MEETINGS.findIndex(({ id }) => id === args.id),
        1,
      );
      return { handled: true, value: null };
    case "get_meeting":
      // A copy, as the real IPC returns: React sees each load as new data.
      return { handled: true, value: structuredClone(meeting) };
    case "rename_meeting":
      if (meeting) {
        const title = String(args.title).trim();
        meeting.title = title === "" ? null : title;
      }
      return { handled: true, value: null };
    case "rename_speaker":
      meeting?.speakers.forEach((speaker) => {
        if (speaker.key === args.key) speaker.name = String(args.name);
      });
      return { handled: true, value: null };
    case "retry_meeting_summary":
      // Codex answers at once.
      if (meeting) {
        Object.assign(meeting, {
          summary: SUMMARY,
          summary_error: null,
          summary_status: "ready",
        });
      }
      return { handled: true, value: null };
    case "delete_meeting_audio":
      if (meeting) Object.assign(meeting, { audio: "deleted", audio_bytes: 0 });
      return { handled: true, value: null };
    default:
      return { handled: false };
  }
};

let modelNow = modelsStatus;
const setModel = (status: LocalModelStatus) => {
  modelNow = status;
  void events.localModelStatus.emit(status);
};

/** The backend's switch to this Mac once the model is ready. */
const switchIfAsked = () => {
  if (!SETTINGS.switch_to_local_when_ready || modelNow.state.state !== "ready")
    return;
  SETTINGS.switch_to_local_when_ready = false;
  SETTINGS.transcription_engine = "local";
  void emit("settings-changed", { setting: "transcription_engine" });
  void events.switchedToLocal.emit({});
};

const dropSwitch = () => {
  if (!SETTINGS.switch_to_local_when_ready) return;
  SETTINGS.switch_to_local_when_ready = false;
  void emit("settings-changed", { setting: "switch_to_local_when_ready" });
};

/** Engine and card commands, which change the stored settings. */
const handleEngineCommand = (
  command: string,
  args: Record<string, unknown>,
): { handled: boolean; value?: unknown } => {
  switch (command) {
    case "get_app_settings":
      return { handled: true, value: structuredClone(SETTINGS) };
    case "get_local_model_status":
      return { handled: true, value: modelNow };
    case "change_transcription_engine_setting":
      // The backend refuses an engine this Mac cannot run.
      if (engineRefused) throw "This Mac cannot run the local model";
      SETTINGS.transcription_engine = args.engine as Engine;
      dropSwitch();
      if (args.engine === "local" && modelNow.state.state !== "ready") {
        setModel(MODELS.downloading);
      }
      return { handled: true, value: null };
    case "switch_to_local_when_ready":
      SETTINGS.switch_to_local_when_ready = true;
      if (modelNow.state.state === "ready") switchIfAsked();
      else setModel(MODELS.downloading);
      return { handled: true, value: null };
    case "change_dictation_style_setting":
      if (styleRefused) throw "The settings store is read-only";
      SETTINGS.dictation_style = args.style as DictationStyle;
      return { handled: true, value: null };
    case "change_local_stt_card_dismissed_setting":
      SETTINGS.local_stt_card_dismissed = args.dismissed as boolean;
      return { handled: true, value: null };
    default:
      return { handled: false };
  }
};

/** The ChatGPT session commands, which change the account. */
const handleAccountCommand = (
  command: string,
): { handled: boolean; value?: unknown } => {
  switch (command) {
    case "get_codex_auth_status":
      return {
        handled: true,
        value: {
          expired: account === "expired",
          signed_in: account === "signedIn",
        },
      };
    case "sign_in_with_chatgpt":
      signIn?.reject("cancelled");
      return {
        handled: true,
        value: new Promise((resolve, reject) => {
          signIn = { resolve, reject };
        }),
      };
    case "cancel_chatgpt_sign_in":
      signIn?.reject("cancelled");
      signIn = null;
      return { handled: true, value: null };
    case "sign_out_of_chatgpt":
      if (signOutFails) {
        return {
          handled: true,
          value: Promise.reject("Couldn't remove auth.json"),
        };
      }
      account = "none";
      return { handled: true, value: null };
    default:
      return { handled: false };
  }
};

const RESPONSES: Record<string, unknown> = {
  change_meeting_capture_mode_setting: null,
  change_meetings_enabled_setting: null,
  copy_meeting_markdown: null,
  discard_meeting: null,
  export_meeting_markdown: "/Users/user/Documents/2025-10-08 Point hebdo.md",
  retry_meeting: null,
  retranscribe_meeting: null,
  reanalyze_meeting: null,
  download_local_model: null,
  cancel_local_model_download: null,
  delete_local_model: null,
  get_meeting_capabilities: { analysis, system_audio: systemAudio },
  get_meeting_runtime: MEETING_RUNTIME,
  get_system_audio_permission: systemAudioPermission,
  open_system_audio_privacy_settings: null,
  start_meeting: null,
  stop_meeting: null,
  check_custom_sounds: { start: false, stop: false },
  check_for_updates: { available: updateAvailable ? "1.9.0" : null },
  get_update_status: { available: updateAvailable ? "1.9.0" : null },
  get_app_dir_path: APP_DATA_DIR,
  get_audio_file_path: `${APP_DATA_DIR}/recordings/murmur-1.wav`,
  get_available_microphones: MICROPHONES,
  get_available_output_devices: OUTPUTS,
  get_default_settings: SETTINGS,
  get_history_entries: {
    entries: historyFallback ? FALLBACK_HISTORY : HISTORY,
    has_more: false,
  },
  get_insights:
    params.get("insights") === "huge"
      ? {
          ...INSIGHTS,
          minutes_saved: 999 * 60 + 59,
          total_words: 9_999_999,
          words_per_minute: 999,
        }
      : INSIGHTS,
  get_log_dir_path: LOG_DIR,
  get_microphone_authorization_status: 3,
  get_microphone_channels: 1,
  get_shortcut_registration_errors: {},
  get_key_labels: {},
  initialize_enigo: null,
  initialize_shortcuts: null,
  is_laptop: true,
  "plugin:app|version": "1.3.7",
  "plugin:macos-permissions|check_accessibility_permission": !showOnboarding,
  "plugin:macos-permissions|check_microphone_permission": !showOnboarding,
  "plugin:os|locale": language,
  "plugin:updater|check": updateAvailable
    ? {
        body: "",
        currentVersion: "1.3.7",
        date: null,
        rawJson: {},
        rid: 1,
        version: "1.9.0",
      }
    : null,
};

window.settingsPages = {
  unhandledCommands: [],
  calls: [],
  emitProgress: (progress) => events.meetingProgress.emit(progress),
  emitMeetingsUpdated: (id) => events.meetingsUpdated.emit({ id }),
  emitMeetingState: (runtime) => events.meetingStateChanged.emit(runtime),
  holdMeetingLoads: () => {
    meetingLoadsHeld = new Promise((resolve) => {
      releaseMeetingLoads = () => {
        meetingLoadsHeld = null;
        resolve();
      };
    });
  },
  releaseMeetingLoads: () => releaseMeetingLoads(),
  finishModelDownload: () => {
    setModel(MODELS.ready);
    switchIfAsked();
  },
  resolveSignIn: () => {
    account = "signedIn";
    signIn?.resolve(null);
    signIn = null;
  },
  emitUpdateStatus: (available) => emit("update-status", { available }),
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
    if (command in RESPONSES) return RESPONSES[command];
    if (!window.settingsPages.unhandledCommands.includes(command)) {
      window.settingsPages.unhandledCommands.push(command);
    }
    return null;
  },
  { shouldMockEvents: true },
);

Object.assign(window, {
  __TAURI_OS_PLUGIN_INTERNALS__: {
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
    <App />,
  );
};

void renderFixture();
