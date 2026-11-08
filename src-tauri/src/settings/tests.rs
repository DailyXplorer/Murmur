use super::*;
use chrono::{DateTime, Utc};
use std::path::Path;

#[cfg(target_os = "windows")]
mod windows;

fn default_settings_json() -> serde_json::Value {
    serde_json::to_value(get_default_settings()).unwrap()
}

fn launched_at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-19T09:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn store_bytes(settings: &AppSettings) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "settings": settings })).unwrap()
}

fn recovered(path: &Path) -> Option<SettingsRecovery> {
    check_store_file(path, launched_at()).map(|(recovery, _)| recovery)
}

fn settings_in(path: &Path) -> AppSettings {
    let store: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    parse_stored_settings(&store["settings"]).0
}

/// A settings write cut short (a full disk, a power cut) used to read as
/// a first launch: every setting back to its default, and the default
/// history limit of 5 deleting the rest of the history.
#[test]
fn a_damaged_settings_file_is_set_aside_and_the_last_good_copy_restored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SETTINGS_STORE_PATH);
    let mut chosen = get_default_settings();
    chosen.onboarding_completed = true;
    chosen.history_limit = MAX_HISTORY_LIMIT;
    std::fs::write(&path, store_bytes(&chosen)).unwrap();
    assert_eq!(recovered(&path), None);

    std::fs::write(&path, b"").unwrap();
    assert_eq!(recovered(&path), Some(SettingsRecovery::Restored));
    let restored = settings_in(&path);
    assert!(restored.onboarding_completed);
    assert_eq!(restored.history_limit, MAX_HISTORY_LIMIT);
    let set_aside = dir
        .path()
        .join("settings_store.json.corrupt-20261019-090000");
    assert_eq!(std::fs::read(set_aside).unwrap(), b"");
}

#[test]
fn a_damaged_settings_file_without_a_good_copy_keeps_the_whole_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SETTINGS_STORE_PATH);
    let cut_short = store_bytes(&get_default_settings())[..40].to_vec();
    std::fs::write(&path, &cut_short).unwrap();

    let (recovery, seeded) = check_store_file(&path, launched_at()).unwrap();
    assert_eq!(recovery, SettingsRecovery::Reset);
    let reset = settings_in(&path);
    // This launch reads the same settings, whatever reached the disk.
    assert_eq!(
        serde_json::to_value(parse_stored_settings(&seeded).0).unwrap(),
        serde_json::to_value(&reset).unwrap()
    );
    assert_eq!(
        reset.recording_retention_period,
        RecordingRetentionPeriod::Never
    );
    assert!(!reset.onboarding_completed);
    let set_aside = dir
        .path()
        .join("settings_store.json.corrupt-20261019-090000");
    assert_eq!(std::fs::read(set_aside).unwrap(), cut_short);
}

#[test]
fn a_first_launch_has_no_settings_file_to_repair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SETTINGS_STORE_PATH);
    assert_eq!(recovered(&path), None);
    assert!(!path.exists());
}

/// Users who finished onboarding before the local engine existed keep
/// the cloud engine, written down so a later default cannot move them.
#[test]
fn an_existing_user_without_an_engine_keeps_the_cloud() {
    let mut stored = default_settings_json();
    stored
        .as_object_mut()
        .unwrap()
        .remove("transcription_engine");
    stored["onboarding_completed"] = serde_json::json!(true);

    let (settings, updated) = parse_stored_settings(&stored);

    assert_eq!(settings.transcription_engine, Engine::Cloud);
    assert!(updated, "the engine must be persisted");
}

#[test]
fn a_stored_engine_choice_is_kept() {
    let mut stored = default_settings_json();
    stored["onboarding_completed"] = serde_json::json!(true);
    stored["transcription_engine"] = serde_json::json!("local");

    let (settings, updated) = parse_stored_settings(&stored);

    assert_eq!(settings.transcription_engine, Engine::Local);
    assert!(!updated);
}

#[test]
fn a_new_install_gets_the_default_engine() {
    let (settings, _) = parse_stored_settings(&serde_json::json!({}));
    assert_eq!(
        settings.transcription_engine,
        get_default_settings().transcription_engine
    );
    assert!(!settings.local_stt_card_dismissed);
    assert!(!settings.local_model_download_wanted);
}

#[test]
fn a_fresh_store_dictates_on_this_mac() {
    let (settings, _) = parse_stored_settings(&serde_json::json!({}));
    assert_eq!(settings.transcription_engine, Engine::Local);
    assert_eq!(get_default_settings().transcription_engine, Engine::Local);
}

#[test]
fn a_requested_switch_to_local_waits_for_the_model_and_happens_once() {
    let mut settings = get_default_settings();
    settings.transcription_engine = Engine::Cloud;
    settings.switch_to_local_when_ready = true;

    assert!(!settings.switch_to_local_if_asked(false));
    assert_eq!(settings.transcription_engine, Engine::Cloud);
    assert!(settings.switch_to_local_when_ready, "still waiting");

    assert!(settings.switch_to_local_if_asked(true));
    assert_eq!(settings.transcription_engine, Engine::Local);
    assert!(!settings.switch_to_local_when_ready);

    // A later choice of the cloud is not undone by the next ready model.
    settings.transcription_engine = Engine::Cloud;
    assert!(!settings.switch_to_local_if_asked(true));
    assert_eq!(settings.transcription_engine, Engine::Cloud);
}

/// Every field must survive a partial store: a missing key must never fail
/// the whole-settings parse. `json!({})` is the extreme case.
#[test]
fn empty_store_parses_with_defaults() {
    let settings: AppSettings = serde_json::from_value(serde_json::json!({}))
        .expect("all AppSettings fields need serde defaults");
    assert!(!settings.audio_feedback);
    assert!(settings.ai_cleanup_enabled);
    assert!(settings.window_names_enabled);
    assert!(!settings.whisper_mode);
    assert_eq!(settings.accent_color, AccentColor::Pink);
    // Bindings default to empty; the load path merges the real defaults in.
    assert!(settings.bindings.is_empty());
}

#[test]
fn leading_space_is_on_for_new_and_existing_users_unless_turned_off() {
    assert!(get_default_settings().leading_space);
    let mut stored = default_settings_json();
    stored.as_object_mut().unwrap().remove("leading_space");
    assert!(parse_stored_settings(&stored).0.leading_space);

    stored["leading_space"] = serde_json::json!(false);
    let settings = parse_stored_settings(&stored).0;
    assert!(!settings.leading_space);
    assert_eq!(
        serde_json::to_value(settings).unwrap()["leading_space"],
        false
    );
}

#[test]
fn log_level_accepts_legacy_numbers_and_serializes_as_names() {
    let expected = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
    ];
    for (legacy, level) in (1_u64..=5).zip(expected) {
        assert_eq!(
            serde_json::from_value::<LogLevel>(serde_json::json!(legacy)).unwrap(),
            level
        );
    }
    assert_eq!(serde_json::to_value(LogLevel::Trace).unwrap(), "trace");
    assert_eq!(serde_json::to_value(LogLevel::Error).unwrap(), "error");
}

#[test]
fn salvage_preserves_legacy_numeric_log_level() {
    let mut stored = default_settings_json();
    stored["log_level"] = serde_json::json!(5);
    stored["sound_theme"] = serde_json::json!("theremin");

    let salvaged = salvage_settings(&stored);
    assert_eq!(salvaged.log_level, LogLevel::Error);
    assert_eq!(salvaged.sound_theme, default_sound_theme());
}

#[test]
fn salvage_preserves_valid_fields_when_one_value_is_invalid() {
    let mut stored = default_settings_json();
    let map = stored.as_object_mut().unwrap();
    map.insert("selected_language".into(), serde_json::json!("fr"));
    map.insert("onboarding_completed".into(), serde_json::json!(true));
    // An enum variant this build doesn't know, e.g. written by a newer
    // version before a downgrade.
    map.insert("sound_theme".into(), serde_json::json!("theremin"));
    stored["bindings"]["transcribe"]["current_binding"] = serde_json::json!("f13");

    // Precondition: the whole-store parse fails, so salvage is what runs.
    assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

    let salvaged = salvage_settings(&stored);
    assert_eq!(salvaged.selected_language, "fr");
    assert!(salvaged.onboarding_completed);
    assert_eq!(salvaged.bindings["transcribe"].current_binding, "f13");
    assert_eq!(salvaged.sound_theme, default_sound_theme());
}

/// The default keeps only the latest 5 dictations, so an unknown period
/// must not fall back to it and delete the rest of the history.
#[test]
fn salvage_of_an_unknown_retention_period_deletes_no_history() {
    let mut stored = default_settings_json();
    stored["recording_retention_period"] = serde_json::json!("years1");
    stored["history_limit"] = serde_json::json!(5);
    assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

    let salvaged = salvage_settings(&stored);
    assert_eq!(
        salvaged.recording_retention_period,
        RecordingRetentionPeriod::Never
    );
}

/// Gemini and Apple Intelligence were removed, so Codex is the only
/// service. A store that chose either, or a value no build knows, keeps
/// every other setting and is rewritten without the old choice.
#[test]
fn a_removed_provider_or_cleanup_engine_lands_on_codex() {
    for (key, value) in [
        ("transcription_provider", "gemini"),
        ("transcription_provider", "codex"),
        ("transcription_provider", "whisper"),
        ("cleanup_engine", "apple"),
        ("cleanup_engine", "codex"),
        ("cleanup_engine", "cloud"),
    ] {
        let mut stored = default_settings_json();
        stored[key] = serde_json::json!(value);
        stored["selected_language"] = serde_json::json!("fr");
        stored["onboarding_completed"] = serde_json::json!(true);
        stored["ai_cleanup_enabled"] = serde_json::json!(false);
        stored["custom_words"] = serde_json::json!(["Murmur"]);

        let (settings, updated) = parse_stored_settings(&stored);

        assert!(updated, "{key}={value} must be rewritten");
        assert_eq!(settings.selected_language, "fr", "{key}={value}");
        assert!(settings.onboarding_completed, "{key}={value}");
        assert!(!settings.ai_cleanup_enabled, "{key}={value}");
        assert_eq!(settings.custom_words, ["Murmur"], "{key}={value}");
        let rewritten = serde_json::to_value(&settings).unwrap();
        assert!(rewritten.get(key).is_none(), "{key}={value}");
    }
}

#[test]
fn salvage_defaults_only_an_invalid_accent_color() {
    let mut stored = default_settings_json();
    stored["accent_color"] = serde_json::json!("ultraviolet");
    stored["selected_language"] = serde_json::json!("fr");

    let salvaged = salvage_settings(&stored);
    assert_eq!(salvaged.accent_color, AccentColor::Pink);
    assert_eq!(salvaged.selected_language, "fr");
}

#[test]
fn a_store_from_before_dictation_styles_gets_normal() {
    let mut stored = default_settings_json();
    stored.as_object_mut().unwrap().remove("dictation_style");
    stored["onboarding_completed"] = serde_json::json!(true);
    assert_eq!(
        parse_stored_settings(&stored).0.dictation_style,
        DictationStyle::Normal
    );
}

#[test]
fn dictation_style_round_trips_and_an_unknown_one_falls_back_to_normal() {
    for (style, name) in [
        (DictationStyle::Formal, "formal"),
        (DictationStyle::Normal, "normal"),
        (DictationStyle::Casual, "casual"),
    ] {
        assert_eq!(serde_json::to_value(style).unwrap(), name);
        let mut stored = default_settings_json();
        stored["dictation_style"] = serde_json::json!(name);
        assert_eq!(parse_stored_settings(&stored).0.dictation_style, style);
    }
    let mut stored = default_settings_json();
    stored["dictation_style"] = serde_json::json!("shouty");
    stored["selected_language"] = serde_json::json!("fr");
    let salvaged = salvage_settings(&stored);
    assert_eq!(salvaged.dictation_style, DictationStyle::Normal);
    assert_eq!(salvaged.selected_language, "fr");
}

#[test]
fn salvage_drops_only_wrong_typed_fields() {
    let mut stored = default_settings_json();
    let map = stored.as_object_mut().unwrap();
    map.insert("paste_delay_ms".into(), serde_json::json!("sixty"));
    map.insert("sound_theme".into(), serde_json::json!(42));
    map.insert("custom_words".into(), serde_json::json!(["Murmur"]));

    assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

    let salvaged = salvage_settings(&stored);
    assert_eq!(salvaged.paste_delay_ms, default_paste_delay_ms());
    assert_eq!(salvaged.sound_theme, default_sound_theme());
    assert_eq!(salvaged.custom_words, vec!["Murmur".to_string()]);
}

#[test]
fn salvage_of_poisoned_bindings_keeps_other_fields() {
    let mut stored = default_settings_json();
    let map = stored.as_object_mut().unwrap();
    // One malformed entry poisons the whole bindings map, but must not
    // take the rest of the settings down with it.
    map.insert(
        "bindings".into(),
        serde_json::json!({ "transcribe": { "id": 42 } }),
    );
    map.insert("selected_language".into(), serde_json::json!("es"));

    assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

    let salvaged = salvage_settings(&stored);
    assert_eq!(salvaged.selected_language, "es");
    let defaults = get_default_settings();
    assert_eq!(
        salvaged.bindings["transcribe"].current_binding,
        defaults.bindings["transcribe"].current_binding
    );
}

#[test]
fn salvage_tolerates_unknown_keys() {
    let mut stored = default_settings_json();
    let map = stored.as_object_mut().unwrap();
    map.insert(
        "field_from_the_future".into(),
        serde_json::json!({ "nested": true }),
    );
    map.insert("selected_language".into(), serde_json::json!("de"));
    map.insert("sound_theme".into(), serde_json::json!("theremin"));

    let salvaged = salvage_settings(&stored);
    assert_eq!(salvaged.selected_language, "de");
    assert_eq!(salvaged.sound_theme, default_sound_theme());
}

#[test]
fn salvage_of_non_object_store_falls_back_to_defaults() {
    for stored in [
        serde_json::json!("corrupt"),
        serde_json::json!(null),
        serde_json::json!([1, 2, 3]),
    ] {
        let salvaged = salvage_settings(&stored);
        assert_eq!(
            serde_json::to_value(&salvaged).unwrap(),
            default_settings_json()
        );
    }
}

#[test]
fn default_settings_disable_auto_submit() {
    let settings = get_default_settings();
    assert!(!settings.auto_submit);
    assert_eq!(settings.auto_submit_key, AutoSubmitKey::Enter);
    assert!(settings.whats_new_last_seen_version.is_empty());
    assert!(settings.reliable_paste);
}

#[test]
fn default_overlay_style_is_minimal_when_overlay_defaults_on() {
    let settings = get_default_settings();
    assert_eq!(settings.overlay_style, OverlayStyle::Minimal);
    assert_eq!(settings.paste_method, PasteMethod::CtrlV);
    assert_eq!(
        settings.bindings["transcribe"].default_binding,
        DEFAULT_TRANSCRIBE_BINDING
    );
    assert_eq!(
        settings.bindings["transcribe"].current_binding,
        DEFAULT_TRANSCRIBE_BINDING
    );
}

#[test]
fn update_checks_are_on_by_default() {
    assert!(get_default_settings().update_checks_enabled);
}

#[test]
fn obsolete_settings_fall_back_without_resetting_other_fields() {
    for legacy_paste_method in ["shift_insert", "ctrl_shift_v", "external_script"] {
        let mut stored = default_settings_json();
        let stored = stored.as_object_mut().unwrap();
        stored.insert("selected_language".into(), serde_json::json!("de"));
        stored.insert(
            "paste_method".into(),
            serde_json::json!(legacy_paste_method),
        );
        stored.insert("typing_tool".into(), serde_json::json!("wtype"));
        stored.insert(
            "external_script_path".into(),
            serde_json::json!("/tmp/paste"),
        );

        let settings = salvage_settings(&serde_json::Value::Object(stored.clone()));
        assert_eq!(settings.selected_language, "de");
        assert_eq!(settings.paste_method, PasteMethod::CtrlV);

        let repaired = serde_json::to_value(settings).unwrap();
        assert!(repaired.get("typing_tool").is_none());
        assert!(repaired.get("external_script_path").is_none());
    }
}

#[test]
fn either_old_recording_mode_migrates_silently_to_the_one_shortcut() {
    for push_to_talk in [true, false] {
        let mut stored = default_settings_json();
        let stored = stored.as_object_mut().unwrap();
        stored.insert("push_to_talk".into(), serde_json::json!(push_to_talk));
        stored.insert("selected_language".into(), serde_json::json!("fr"));
        let mut transcribe = stored["bindings"]["transcribe"].clone();
        transcribe["current_binding"] = serde_json::json!("ctrl+shift+d");
        stored["bindings"]["transcribe"] = transcribe;

        let (settings, updated) = parse_stored_settings(&serde_json::Value::Object(stored.clone()));
        assert!(updated, "push_to_talk={push_to_talk} must be rewritten");
        assert_eq!(settings.selected_language, "fr");
        assert_eq!(
            settings.bindings["transcribe"].current_binding,
            "ctrl+shift+d"
        );

        let repaired = serde_json::to_value(settings).unwrap();
        assert!(repaired.get("push_to_talk").is_none());
    }
}

#[test]
fn a_binding_removed_by_a_release_is_dropped_and_missing_ones_are_added() {
    let mut stored = default_settings_json();
    let bindings = stored["bindings"].as_object_mut().unwrap();
    bindings.remove("paste_last");
    bindings.insert(
        "transcribe_with_post_process".into(),
        serde_json::json!({
            "id": "transcribe_with_post_process",
            "name": "Transcribe with Post-Processing",
            "description": "Converts your speech into text and applies AI post-processing.",
            "default_binding": "option+shift+space",
            "current_binding": "option+shift+space"
        }),
    );

    let (settings, updated) = parse_stored_settings(&stored);
    assert!(updated);
    let mut ids: Vec<&str> = settings.bindings.keys().map(String::as_str).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        [
            "cancel",
            "meeting_paste_new",
            "meeting_toggle",
            "paste_last",
            "transcribe"
        ]
    );
}

#[test]
fn meetings_are_on_by_default_and_record_both_sources() {
    let mut stored = default_settings_json();
    let settings_object = stored.as_object_mut().unwrap();
    settings_object.remove("meetings_enabled");
    settings_object.remove("meeting_capture_mode");
    stored["bindings"]
        .as_object_mut()
        .unwrap()
        .remove("meeting_toggle");

    let (settings, _) = parse_stored_settings(&stored);

    assert!(settings.meetings_enabled);
    assert!(get_default_settings().meetings_enabled);
    assert_eq!(settings.meeting_capture_mode, MeetingCaptureMode::Both);
    assert_eq!(
        settings.bindings["meeting_toggle"].current_binding,
        crate::meetings::DEFAULT_BINDING
    );
    assert_eq!(
        serde_json::to_value(MeetingCaptureMode::SystemOnly).unwrap(),
        "system_only"
    );
}

#[test]
fn settings_saved_before_call_prompts_offer_them() {
    let mut stored = default_settings_json();
    stored
        .as_object_mut()
        .unwrap()
        .remove("meeting_call_prompt");

    assert!(parse_stored_settings(&stored).0.meeting_call_prompt);
    assert!(get_default_settings().meeting_call_prompt);
    stored["meeting_call_prompt"] = serde_json::json!(false);
    assert!(!parse_stored_settings(&stored).0.meeting_call_prompt);
}

#[test]
fn call_rules_saved_by_1_10_are_removed_from_the_file_at_launch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SETTINGS_STORE_PATH);
    let mut stored = default_settings_json();
    stored["history_limit"] = serde_json::json!(MAX_HISTORY_LIMIT);
    stored["meeting_call_policies"] = serde_json::json!({
        "zoom": "always",
        "face_time": "never",
    });
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({ "settings": stored })).unwrap(),
    )
    .unwrap();

    let file = file::SettingsFile::open(path.clone());
    let settings = read_from(&file);
    file.flush();

    assert_eq!(settings.history_limit, MAX_HISTORY_LIMIT);
    let on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(on_disk["settings"].get("meeting_call_policies").is_none());
    assert_eq!(on_disk["settings"]["history_limit"], MAX_HISTORY_LIMIT);
}

#[test]
fn settings_saved_before_live_transcripts_show_them_and_leave_the_paste_unbound() {
    let mut stored = default_settings_json();
    stored
        .as_object_mut()
        .unwrap()
        .remove("meeting_live_transcript");
    stored["bindings"]
        .as_object_mut()
        .unwrap()
        .remove(crate::meetings::live::PASTE_BINDING_ID);

    let (settings, updated) = parse_stored_settings(&stored);
    assert!(updated);
    assert!(settings.meeting_live_transcript);
    assert!(get_default_settings().meeting_live_transcript);
    let paste = &settings.bindings[crate::meetings::live::PASTE_BINDING_ID];
    assert_eq!(paste.current_binding, "");
    assert!(!crate::shortcut::binding_registers(
        &settings,
        crate::meetings::live::PASTE_BINDING_ID
    ));
    stored["meeting_live_transcript"] = serde_json::json!(false);
    assert!(!parse_stored_settings(&stored).0.meeting_live_transcript);
}

#[test]
fn settings_saved_before_audio_retention_keep_meeting_audio() {
    let mut stored = default_settings_json();
    stored
        .as_object_mut()
        .unwrap()
        .remove("meeting_audio_retention");

    let (settings, _) = parse_stored_settings(&stored);

    assert_eq!(
        settings.meeting_audio_retention,
        MeetingAudioRetention::Keep
    );
    assert_eq!(
        get_default_settings().meeting_audio_retention,
        MeetingAudioRetention::Keep
    );
    // The values the settings page sends, as `src/bindings.ts` types them.
    for (sent, retention) in [
        ("keep", MeetingAudioRetention::Keep),
        ("days7", MeetingAudioRetention::Days7),
        ("days30", MeetingAudioRetention::Days30),
        ("days90", MeetingAudioRetention::Days90),
    ] {
        stored["meeting_audio_retention"] = serde_json::json!(sent);
        assert_eq!(
            parse_stored_settings(&stored).0.meeting_audio_retention,
            retention
        );
        assert_eq!(serde_json::to_value(retention).unwrap(), sent);
    }
}

#[test]
fn a_stored_shortcut_the_hotkey_parser_rejects_is_reset_to_its_default() {
    let mut stored = default_settings_json();
    stored["bindings"]["cancel"]["current_binding"] = serde_json::json!("page up");
    stored["bindings"]["transcribe"]["current_binding"] = serde_json::json!("f5+f6");

    let (settings, updated) = parse_stored_settings(&stored);

    assert!(updated);
    assert_eq!(settings.bindings["cancel"].current_binding, "escape");
    assert_eq!(
        settings.bindings["transcribe"].current_binding,
        DEFAULT_TRANSCRIBE_BINDING
    );
}

#[test]
fn a_stored_caps_lock_shortcut_is_reset_to_its_default() {
    let mut stored = default_settings_json();
    stored["bindings"]["transcribe"]["current_binding"] = serde_json::json!("capslock");

    let (settings, updated) = parse_stored_settings(&stored);

    assert!(updated);
    assert_eq!(
        settings.bindings["transcribe"].current_binding,
        DEFAULT_TRANSCRIBE_BINDING
    );
}

#[test]
fn valid_store_without_obsolete_fields_does_not_require_rewrite() {
    let (_, updated) = parse_stored_settings(&default_settings_json());
    assert!(!updated);
}

#[test]
fn valid_store_with_obsolete_fields_is_marked_for_rewrite() {
    let mut stored = default_settings_json();
    let stored = stored.as_object_mut().unwrap();
    stored.insert("selected_language".into(), serde_json::json!("de"));
    stored.insert("paste_method".into(), serde_json::json!("direct"));
    stored.insert("typing_tool".into(), serde_json::json!("wtype"));
    stored.insert(
        "external_script_path".into(),
        serde_json::json!("/tmp/paste"),
    );

    let (settings, updated) = parse_stored_settings(&serde_json::Value::Object(stored.clone()));
    assert!(updated);
    assert_eq!(settings.selected_language, "de");
    assert_eq!(settings.paste_method, PasteMethod::Direct);

    let repaired = serde_json::to_value(settings).unwrap();
    assert!(repaired.get("typing_tool").is_none());
    assert!(repaired.get("external_script_path").is_none());
}

#[test]
fn stored_unbounded_values_are_repaired() {
    let mut stored = default_settings_json();
    stored["paste_delay_ms"] = serde_json::json!(u64::MAX);
    stored["paste_delay_after_ms"] = serde_json::json!(MAX_PASTE_DELAY_MS + 1);
    stored["extra_recording_buffer_ms"] = serde_json::json!(MAX_EXTRA_RECORDING_BUFFER_MS + 1);
    stored["history_limit"] = serde_json::json!(5000);
    stored["custom_words"] = serde_json::json!([
        "  Murmur   App  ",
        "Murmur App",
        "valid <markup>",
        "x".repeat(MAX_CUSTOM_WORD_CHARS + 1)
    ]);

    let (settings, updated) = parse_stored_settings(&stored);
    assert!(updated);
    assert_eq!(settings.paste_delay_ms, default_paste_delay_ms());
    assert_eq!(
        settings.paste_delay_after_ms,
        default_paste_delay_after_ms()
    );
    assert_eq!(settings.extra_recording_buffer_ms, 0);
    // A limit saved before it was bounded keeps as many dictations as allowed.
    assert_eq!(settings.history_limit, MAX_HISTORY_LIMIT);
    assert_eq!(
        settings.custom_words,
        vec!["Murmur App".to_string(), "valid <markup>".to_string()]
    );
}

#[test]
fn filler_removal_choice_carries_over_to_ai_cleanup() {
    let mut stored = default_settings_json();
    let map = stored.as_object_mut().unwrap();
    map.remove("ai_cleanup_enabled");
    map.insert(
        "filler_word_removal_enabled".into(),
        serde_json::json!(false),
    );
    map.insert("custom_filler_words".into(), serde_json::json!(["um"]));

    let (settings, updated) = parse_stored_settings(&stored);
    assert!(updated);
    assert!(!settings.ai_cleanup_enabled);
    let rewritten = serde_json::to_value(&settings).unwrap();
    assert!(rewritten.get("filler_word_removal_enabled").is_none());
    assert!(rewritten.get("custom_filler_words").is_none());

    let (settings, _) = parse_stored_settings(&rewritten);
    assert!(!settings.ai_cleanup_enabled);

    let mut fresh = default_settings_json();
    fresh.as_object_mut().unwrap().remove("ai_cleanup_enabled");
    assert!(parse_stored_settings(&fresh).0.ai_cleanup_enabled);
}

/// Correction learning is gone: the words it added leave the dictionary
/// at the next launch, once, and the words the user typed in stay.
#[test]
fn learned_words_leave_the_dictionary_once() {
    let mut stored = default_settings_json();
    stored["custom_words"] = serde_json::json!(["Murmur", "Tauri", "Kubernetes"]);
    stored["learned_words"] =
        serde_json::json!([{ "word": "Kubernetes", "heard_as": "cooper netties" }]);

    let (settings, updated) = parse_stored_settings(&stored);
    assert!(updated, "the cleaned dictionary must be persisted");
    assert_eq!(settings.custom_words, ["Murmur", "Tauri"]);

    let rewritten = serde_json::to_value(&settings).unwrap();
    assert!(rewritten.get("learned_words").is_none());
    let (again, updated) = parse_stored_settings(&rewritten);
    assert!(!updated, "a second launch has nothing left to clean");
    assert_eq!(again.custom_words, ["Murmur", "Tauri"]);
}

#[test]
fn learned_words_leave_a_salvaged_store_too() {
    let mut stored = default_settings_json();
    stored["sound_theme"] = serde_json::json!("theremin");
    stored["selected_language"] = serde_json::json!("fr");
    stored["custom_words"] = serde_json::json!(["Murmur", "Kubernetes"]);
    stored["learned_words"] =
        serde_json::json!([{ "word": "Kubernetes", "heard_as": "cooper netties" }]);
    // Precondition: the whole-store parse fails, so salvage is what runs.
    assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

    let (settings, updated) = parse_stored_settings(&stored);
    assert!(updated);
    assert_eq!(settings.selected_language, "fr");
    assert_eq!(settings.custom_words, ["Murmur"]);
}

#[test]
fn a_store_without_learned_words_keeps_its_custom_words() {
    let mut stored = default_settings_json();
    stored["custom_words"] = serde_json::json!(["Murmur", "Tauri", "Kubernetes"]);

    let (settings, updated) = parse_stored_settings(&stored);
    assert!(!updated);
    assert_eq!(settings.custom_words, ["Murmur", "Tauri", "Kubernetes"]);
}

#[test]
fn custom_word_validation_rejects_unbounded_or_ambiguous_input() {
    let too_many = vec!["word".to_string(); MAX_CUSTOM_WORDS + 1];
    assert!(validate_custom_words(&too_many).is_err());
    assert!(validate_custom_words(&["x".repeat(MAX_CUSTOM_WORD_CHARS + 1)]).is_err());
    assert!(validate_custom_words(&[" two  spaces ".to_string()]).is_err());
    assert!(validate_custom_words(&["same".to_string(), "same".to_string()]).is_err());
    assert!(validate_custom_words(&["valid word".to_string()]).is_ok());
}

/// A field this version cannot read (say, written by a newer build) is
/// dropped, but the file as it was stays next to it, once per content.
#[test]
fn salvage_preserves_original_bytes_once() {
    use sha2::{Digest, Sha256};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SETTINGS_STORE_PATH);
    let mut stored = default_settings_json();
    stored["history_limit"] = serde_json::json!(MAX_HISTORY_LIMIT);
    stored["overlay_position"] = serde_json::json!("from-a-newer-build");
    let original = serde_json::to_vec_pretty(&serde_json::json!({ "settings": stored })).unwrap();
    let digest = Sha256::digest(&original);
    let prefix: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
    let copy = dir
        .path()
        .join(format!("settings_store.json.unreadable-{prefix}"));
    let copies = || {
        std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|entry| {
                let name = entry.as_ref().unwrap().file_name();
                name.to_string_lossy().contains(".unreadable-")
            })
            .count()
    };

    for _launch in 0..2 {
        std::fs::write(&path, &original).unwrap();
        let settings = read_from(&file::SettingsFile::open(path.clone()));
        assert_eq!(settings.history_limit, MAX_HISTORY_LIMIT);
        assert_eq!(settings.overlay_position, default_overlay_position());
        assert_eq!(std::fs::read(&copy).unwrap(), original);
        assert_eq!(copies(), 1);
    }
    #[cfg(unix)]
    {
        let mode = std::fs::metadata(&copy).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    assert!(settings_in(&path).overlay_position == default_overlay_position());

    // The repaired file reads cleanly: nothing more to keep.
    read_from(&file::SettingsFile::open(path.clone()));
    assert_eq!(copies(), 1);
}
