import { UPDATE } from "./models";
import { MEETING_RUNTIME } from "./meetings";
import { HISTORY, INSIGHTS, VERSIONS, FALLBACK_HISTORY } from "./history";
import {
  analysis,
  callDetection,
  engine,
  historyFallback,
  language,
  lastSection,
  meetingsStorageBytes,
  missedUpdate,
  params,
  setAsideBytes,
  showOnboarding,
  systemAudio,
  systemAudioPermission,
  updateAvailable,
} from "./params";
import {
  APP_DATA_DIR,
  LOG_DIR,
  MICROPHONES,
  OUTPUTS,
  SETTINGS,
  windows,
} from "./settings";

export const RESPONSES: Record<string, unknown> = {
  change_mcp_enabled_setting: null,
  change_mcp_include_dictations_setting: null,
  get_mcp_setup: {
    command: windows
      ? "C:\\Users\\Some One\\AppData\\Local\\Murmur\\murmur.exe"
      : "/Applications/Murmur.app/Contents/MacOS/murmur",
  },
  change_meeting_audio_retention_setting: null,
  change_meeting_call_prompt_setting: null,
  change_meeting_capture_mode_setting: null,
  get_meetings_storage: {
    audio_bytes: meetingsStorageBytes,
    set_aside_bytes: setAsideBytes,
  },
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
  clear_history: null,
  get_meeting_capabilities: {
    analysis,
    system_audio: systemAudio,
    call_detection: callDetection,
  },
  get_meeting_runtime: MEETING_RUNTIME,
  get_system_audio_permission: systemAudioPermission,
  open_system_audio_privacy_settings: null,
  open_microphone_settings: null,
  start_meeting: null,
  stop_meeting: null,
  check_custom_sounds: { start: false, stop: false },
  check_for_updates: {
    available: updateAvailable || missedUpdate ? "1.9.0" : null,
  },
  get_update_status: { available: updateAvailable ? "1.9.0" : null },
  prepare_update_install: null,
  get_app_dir_path: APP_DATA_DIR,
  get_audio_file_path: `${APP_DATA_DIR}/recordings/murmur-1.wav`,
  get_available_microphones: MICROPHONES,
  get_available_output_devices: OUTPUTS,
  get_default_settings: SETTINGS,
  get_retranscribe_languages:
    engine === "local"
      ? ["en", "fr", "de", "es", "it", "pt", "nl", "pl", "sv"]
      : null,
  get_history_versions: VERSIONS,
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
  initialize_enigo: null,
  initialize_shortcuts: null,
  report_settings_view: null,
  report_opened_meeting: null,
  get_last_settings_section: lastSection,
  hold_settings_window: null,
  release_settings_window: null,
  is_laptop: true,
  "plugin:app|version": "1.3.7",
  "plugin:macos-permissions|check_accessibility_permission": !showOnboarding,
  "plugin:macos-permissions|check_microphone_permission": !showOnboarding,
  "plugin:os|locale": language,
  "plugin:updater|check": updateAvailable ? UPDATE : null,
};
