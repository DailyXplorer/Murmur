import React from "react";
import ReactDOM from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  AppSettings,
  AudioDevice,
  HistoryEntry,
  InsightsSummary,
  MeetingListItem,
  MeetingRuntime,
} from "../../src/bindings";

declare global {
  interface Window {
    settingsPages: {
      unhandledCommands: string[];
      calls: { command: string; args: unknown }[];
    };
  }
}

const params = new URLSearchParams(window.location.search);
const language = params.get("lang") ?? "en";
const showOnboarding = params.get("onboarding") === "1";
const geminiSignedOut = params.get("gemini") === "notSignedIn";
const appleIntelligence = params.get("apple") ?? "available";
const meetingsEnabled = params.get("meetings") === "1";
const meetingRecording = params.get("meeting") === "recording";
const systemAudio = params.get("systemAudio") ?? "available";

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
  cleanup_engine: "apple",
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
      current_binding: "option+command+m",
      default_binding: "option+command+m",
      description: "Starts recording a meeting, or stops the one in progress.",
      id: "meeting_toggle",
      name: "Start or stop a meeting",
    },
    paste_last: {
      current_binding: "option+shift+v",
      default_binding: "option+shift+v",
      description: "Paste the last transcription",
      id: "paste_last",
      name: "Paste Last Transcript",
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
  debug_mode: true,
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
  theme: "light",
  transcription_provider: "codex",
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
  dictionary_fixes: 18,
  heatmap: Array.from({ length: 182 }, (_, index) => {
    const date = new Date(Date.UTC(2025, 3, 1) + index * 86_400_000);
    const count = index % 6 === 0 ? 0 : (index * 5) % 9;
    return {
      count,
      date: date.toISOString().slice(0, 10),
      level: count === 0 ? 0 : Math.min(4, Math.ceil(count / 2)),
    };
  }),
  streaks: { current: 6, longest: 21 },
  total_dictations: 412,
  total_words: 15_380,
  words_corrected: 7,
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
    status: "processing",
    title: null,
    tracks: "mic",
  },
  {
    duration_ms: 9 * MINUTE_MS + 41_000,
    id: 2,
    started_at_ms: NOW * 1000 - 74 * HOUR * 1000,
    status: "interrupted",
    title: null,
    tracks: "both",
  },
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

const RESPONSES: Record<string, unknown> = {
  change_meeting_capture_mode_setting: null,
  delete_meeting: null,
  discard_meeting: null,
  get_meeting_capabilities: { system_audio: systemAudio },
  get_meeting_runtime: MEETING_RUNTIME,
  get_meetings: { has_more: false, meetings: MEETINGS },
  open_system_audio_privacy_settings: null,
  start_meeting: null,
  stop_meeting: null,
  change_cleanup_engine_setting: null,
  check_custom_sounds: { start: false, stop: false },
  get_app_dir_path: APP_DATA_DIR,
  get_apple_intelligence_status: appleIntelligence,
  get_app_settings: SETTINGS,
  get_audio_file_path: `${APP_DATA_DIR}/recordings/murmur-1.wav`,
  get_available_microphones: MICROPHONES,
  get_available_output_devices: OUTPUTS,
  get_codex_auth_status: { expired: false, signed_in: true },
  get_default_settings: SETTINGS,
  get_gemini_status: { installed: geminiSignedOut, signed_in: false },
  get_history_entries: { entries: HISTORY, has_more: false },
  get_insights: INSIGHTS,
  get_log_dir_path: LOG_DIR,
  get_microphone_authorization_status: 3,
  get_microphone_channels: 1,
  get_shortcut_registration_errors: {},
  initialize_enigo: null,
  initialize_shortcuts: null,
  is_laptop: true,
  "plugin:app|version": "1.3.7",
  "plugin:macos-permissions|check_accessibility_permission": !showOnboarding,
  "plugin:macos-permissions|check_microphone_permission": !showOnboarding,
  "plugin:os|locale": language,
  "plugin:updater|check": null,
};

window.settingsPages = { unhandledCommands: [], calls: [] };

mockIPC(
  (command, args) => {
    window.settingsPages.calls.push({ command, args });
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
