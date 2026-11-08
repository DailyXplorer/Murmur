use crate::engine::Engine;
use std::collections::HashMap;

use super::*;

pub fn get_default_settings() -> AppSettings {
    let default_shortcut = DEFAULT_TRANSCRIBE_BINDING;

    let mut bindings = HashMap::new();
    bindings.insert(
        "transcribe".to_string(),
        ShortcutBinding {
            id: "transcribe".to_string(),
            name: "Transcribe".to_string(),
            description: "Converts your speech into text.".to_string(),
            default_binding: default_shortcut.to_string(),
            current_binding: default_shortcut.to_string(),
        },
    );
    bindings.insert(
        "cancel".to_string(),
        ShortcutBinding {
            id: "cancel".to_string(),
            name: "Cancel".to_string(),
            description: "Cancels the current recording.".to_string(),
            default_binding: "escape".to_string(),
            current_binding: "escape".to_string(),
        },
    );
    bindings.insert(
        crate::paste_last::BINDING_ID.to_string(),
        ShortcutBinding {
            id: crate::paste_last::BINDING_ID.to_string(),
            name: "Paste last transcription".to_string(),
            description: "Pastes the latest transcription again.".to_string(),
            default_binding: crate::paste_last::DEFAULT_BINDING.to_string(),
            current_binding: crate::paste_last::DEFAULT_BINDING.to_string(),
        },
    );
    bindings.insert(
        crate::meetings::BINDING_ID.to_string(),
        ShortcutBinding {
            id: crate::meetings::BINDING_ID.to_string(),
            name: "Start or stop a meeting".to_string(),
            description: "Starts recording a meeting, or stops the one in progress.".to_string(),
            default_binding: crate::meetings::DEFAULT_BINDING.to_string(),
            current_binding: crate::meetings::DEFAULT_BINDING.to_string(),
        },
    );
    bindings.insert(
        crate::meetings::live::PASTE_BINDING_ID.to_string(),
        ShortcutBinding {
            id: crate::meetings::live::PASTE_BINDING_ID.to_string(),
            name: "Paste new meeting text".to_string(),
            description: "Pastes what was said in the meeting since the last paste.".to_string(),
            default_binding: String::new(),
            current_binding: String::new(),
        },
    );

    AppSettings {
        bindings,
        audio_feedback: false,
        audio_feedback_volume: default_audio_feedback_volume(),
        sound_theme: default_sound_theme(),
        start_hidden: default_start_hidden(),
        autostart_enabled: default_autostart_enabled(),
        update_checks_enabled: default_update_checks_enabled(),
        update_checks_turned_on_once: cfg!(target_os = "windows"),
        show_whats_new_on_update: default_show_whats_new_on_update(),
        whats_new_last_seen_version: default_whats_new_last_seen_version(),
        onboarding_completed: false,
        always_on_microphone: false,
        selected_microphone: None,
        selected_channel: None,
        clamshell_microphone: None,
        selected_output_device: None,
        selected_language: "auto".to_string(),
        overlay_position: default_overlay_position(),
        debug_mode: false,
        log_level: default_log_level(),
        custom_words: Vec::new(),
        word_correction_threshold: default_word_correction_threshold(),
        history_limit: default_history_limit(),
        recording_retention_period: default_recording_retention_period(),
        paste_method: PasteMethod::default(),
        clipboard_handling: ClipboardHandling::default(),
        auto_submit: default_auto_submit(),
        auto_submit_key: AutoSubmitKey::default(),
        mute_while_recording: false,
        leading_space: default_leading_space(),
        append_trailing_space: false,
        app_language: default_app_language(),
        theme: default_theme(),
        accent_color: AccentColor::default(),
        experimental_enabled: false,
        lazy_stream_close: false,
        show_tray_icon: default_show_tray_icon(),
        paste_delay_ms: default_paste_delay_ms(),
        paste_delay_after_ms: default_paste_delay_after_ms(),
        reliable_paste: default_reliable_paste(),
        ai_cleanup_enabled: default_ai_cleanup_enabled(),
        dictation_style: DictationStyle::default(),
        window_names_enabled: default_window_names_enabled(),
        window_names_turned_on_once: cfg!(target_os = "windows"),
        whisper_mode: false,
        extra_recording_buffer_ms: 0,
        overlay_style: default_overlay_style(),
        meetings_enabled: default_meetings_enabled(),
        meetings_turned_on_once: cfg!(target_os = "windows"),
        meeting_capture_mode: MeetingCaptureMode::default(),
        meeting_audio_retention: MeetingAudioRetention::default(),
        meeting_call_prompt: default_meeting_call_prompt(),
        meeting_live_transcript: default_meeting_live_transcript(),
        transcription_engine: Engine::default(),
        local_stt_card_dismissed: false,
        local_model_download_wanted: false,
        switch_to_local_when_ready: false,
        mcp_enabled: false,
        mcp_include_dictations: false,
        cleanup_short_dictations: false,
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        get_default_settings()
    }
}
