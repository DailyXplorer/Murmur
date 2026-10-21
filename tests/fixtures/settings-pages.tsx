import React from "react";
import ReactDOM from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  AnalysisSupport,
  AppSettings,
  AudioDevice,
  HistoryEntry,
  InsightsSummary,
  MeetingDetail,
  MeetingListItem,
  MeetingModelsStatus,
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
const MODELS: Record<string, MeetingModelsStatus> = {
  downloading: {
    bytes_on_disk: 210_000_000,
    state: { fraction: 0.42, state: "downloading" },
  },
  missing: { bytes_on_disk: 0, state: { state: "missing" } },
  ready: { bytes_on_disk: 482_000_000, state: { state: "ready" } },
};
const MODEL_COMMANDS: Record<string, MeetingModelsStatus> = {
  download_meeting_models: MODELS.downloading,
  cancel_meeting_models_download: MODELS.missing,
  delete_meeting_models: MODELS.missing,
};
const modelsStatus: MeetingModelsStatus =
  analysis === "available"
    ? MODELS[params.get("models") ?? "ready"]
    : {
        bytes_on_disk: 0,
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
  ai_cleanup_enabled: true,
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
  experimental_enabled: true,
  extra_recording_buffer_ms: 0,
  history_limit: 50,
  learned_words: [{ heard_as: "play right", word: "Playwright" }],
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
    file_name: "murmur-3.wav",
    id: 1,
    raw_text: "remember to water the plants",
    saved: false,
    timestamp: NOW - 26 * HOUR,
    title: "Reminder",
    transcription_text: "remember to water the plants",
  },
];

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

const RESPONSES: Record<string, unknown> = {
  change_meeting_capture_mode_setting: null,
  change_meetings_enabled_setting: null,
  copy_meeting_markdown: null,
  discard_meeting: null,
  export_meeting_markdown: "/Users/user/Documents/2025-10-08 Point hebdo.md",
  retry_meeting: null,
  retranscribe_meeting: null,
  reanalyze_meeting: null,
  get_meeting_models_status: modelsStatus,
  download_meeting_models: null,
  cancel_meeting_models_download: null,
  delete_meeting_models: null,
  get_meeting_capabilities: { analysis, system_audio: systemAudio },
  get_meeting_runtime: MEETING_RUNTIME,
  get_system_audio_permission: systemAudioPermission,
  open_system_audio_privacy_settings: null,
  start_meeting: null,
  stop_meeting: null,
  check_custom_sounds: { start: false, stop: false },
  get_app_dir_path: APP_DATA_DIR,
  get_app_settings: SETTINGS,
  get_audio_file_path: `${APP_DATA_DIR}/recordings/murmur-1.wav`,
  get_available_microphones: MICROPHONES,
  get_available_output_devices: OUTPUTS,
  get_codex_auth_status: { expired: false, signed_in: true },
  get_default_settings: SETTINGS,
  get_history_entries: { entries: HISTORY, has_more: false },
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
  "plugin:updater|check": null,
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
};

mockIPC(
  (command, args) => {
    window.settingsPages.calls.push({ command, args });
    const meeting = handleMeetingCommand(
      command,
      (args ?? {}) as Record<string, unknown>,
    );
    if (meeting.handled) return meeting.value;
    // The backend emits the models' new state before the command returns.
    const models = MODEL_COMMANDS[command];
    if (models) void events.meetingModelsStatus.emit(models);
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
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <App />,
  );
};

void renderFixture();
