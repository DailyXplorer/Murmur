import type {
  AccentColor,
  AppSettings,
  AudioDevice,
  RecordedKeys,
} from "../../../src/bindings";
import {
  cardShown,
  cleanupEnabled,
  dictationStyle,
  engine,
  language,
  meetingsEnabled,
  params,
  showOnboarding,
  type Keyboard,
} from "./params";

export const theme = (params.get("theme") ?? "light") as AppSettings["theme"];
/** `platform=windows` runs the Windows build, with its Ctrl+Win default. */
export const windows = params.get("platform") === "windows";
export const transcribeBinding = windows ? "ctrl+super" : "option+space";
/**
 * `keyboard=azerty` reads a French AZERTY layout as Windows reports it, and
 * saves recorded shortcuts; `keyboard=qwerty` a US one. A test can switch
 * the layout afterwards.
 */
export const savesBindings = params.has("keyboard");
export const KEYBOARDS: Record<
  Keyboard,
  { labels: Record<string, string>; recorded: RecordedKeys }
> = {
  azerty: {
    // What the layout types on a few virtual keys, by the name they
    // register as.
    labels: { "1": "&", a: "a", q: "q", m: "m", ",": ",", "/": ":" },
    // The virtual key under a few physical keys, by the physical key's US
    // name, and the "!" and "<" keys, whose virtual keys no shortcut
    // registers.
    recorded: {
      names: { q: "a", a: "q", ";": "m", m: ",", ".": "/" },
      unusable: ["/", "intlbackslash"],
    },
  },
  qwerty: { labels: {}, recorded: { names: {}, unusable: [] } },
};

export const APP_DATA_DIR =
  "/Users/user/Library/Application Support/com.dailyxplorer.murmur";
export const LOG_DIR = "/Users/user/Library/Logs/com.dailyxplorer.murmur";

export const MICROPHONES: AudioDevice[] = [
  { index: "default", is_default: true, name: "Default" },
  { index: "1", is_default: false, name: "Built-in Microphone" },
];
export const OUTPUTS: AudioDevice[] = [
  { index: "default", is_default: true, name: "Default" },
  { index: "1", is_default: false, name: "Built-in Speakers" },
];

export const SETTINGS: AppSettings = {
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
    meeting_paste_new: {
      current_binding: "",
      default_binding: "",
      description: "Pastes what was said in the meeting since the last paste.",
      id: "meeting_paste_new",
      name: "Paste new meeting text",
    },
    paste_last: {
      current_binding: "option+shift+v",
      default_binding: "option+shift+v",
      description: "Paste the last transcription",
      id: "paste_last",
      name: "Paste last transcript",
    },
    transcribe: {
      current_binding: transcribeBinding,
      default_binding: transcribeBinding,
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
  leading_space: params.get("leadingSpace") !== "0",
  local_model_download_wanted: false,
  local_stt_card_dismissed: !cardShown,
  log_level: "info",
  meeting_audio_retention: "keep",
  meeting_call_prompt: true,
  meeting_capture_mode: "both",
  mcp_enabled: params.get("mcp") === "1",
  mcp_include_dictations: params.get("mcpDictations") === "1",
  meeting_live_transcript: true,
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
  update_checks_turned_on_once: windows,
  whats_new_last_seen_version: "1.3.7",
  whisper_mode: false,
  window_names_enabled: true,
  word_correction_threshold: 0.18,
};
