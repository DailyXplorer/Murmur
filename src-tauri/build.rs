const APP_COMMANDS: &[&str] = &[
    "change_binding",
    "reset_binding",
    "change_ptt_setting",
    "change_audio_feedback_setting",
    "change_audio_feedback_volume_setting",
    "change_sound_theme_setting",
    "change_theme_setting",
    "change_accent_color_setting",
    "change_start_hidden_setting",
    "change_autostart_setting",
    "change_selected_language_setting",
    "change_transcription_provider_setting",
    "change_overlay_position_setting",
    "change_overlay_style_setting",
    "change_debug_mode_setting",
    "change_word_correction_threshold_setting",
    "change_extra_recording_buffer_setting",
    "change_paste_delay_ms_setting",
    "change_paste_delay_after_ms_setting",
    "change_reliable_paste_setting",
    "change_paste_method_setting",
    "change_clipboard_handling_setting",
    "change_auto_submit_setting",
    "change_auto_submit_key_setting",
    "change_experimental_enabled_setting",
    "update_custom_words",
    "suspend_all_bindings",
    "resume_all_bindings",
    "change_mute_while_recording_setting",
    "change_append_trailing_space_setting",
    "change_lazy_stream_close_setting",
    "change_filler_word_removal_enabled_setting",
    "change_app_language_setting",
    "change_update_checks_setting",
    "change_show_whats_new_on_update_setting",
    "change_whats_new_last_seen_version_setting",
    "change_show_tray_icon_setting",
    "show_main_window_command",
    "cancel_operation",
    "get_app_dir_path",
    "get_app_settings",
    "get_default_settings",
    "get_log_dir_path",
    "set_log_level",
    "open_recordings_folder",
    "open_log_dir",
    "open_app_data_dir",
    "repair_accessibility_permission",
    "initialize_enigo",
    "initialize_shortcuts",
    "update_microphone_mode",
    "get_available_microphones",
    "set_selected_microphone",
    "get_available_output_devices",
    "set_selected_output_device",
    "play_test_sound",
    "check_custom_sounds",
    "set_clamshell_microphone",
    "get_microphone_channels",
    "set_selected_channel",
    "get_codex_auth_status",
    "get_gemini_status",
    "open_antigravity",
    "complete_onboarding",
    "get_history_entries",
    "toggle_history_entry_saved",
    "get_audio_file_path",
    "delete_history_entry",
    "retry_history_entry_transcription",
    "update_history_limit",
    "update_recording_retention_period",
    "is_laptop",
];

fn main() {
    generate_tray_translations();
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS));
    tauri_build::try_build(attributes).expect("failed to build Tauri application ACL");
}

fn generate_tray_translations() {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by Cargo");
    let locales_dir = Path::new("../src/i18n/locales");
    println!("cargo:rerun-if-changed=../src/i18n/locales");

    let mut translations: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for entry_result in fs::read_dir(locales_dir).expect("locale directory is readable") {
        let entry = entry_result.expect("locale directory entry is readable");
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(language) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let json_path = path.join("translation.json");
        println!("cargo:rerun-if-changed={}", json_path.display());
        let content = fs::read_to_string(&json_path).expect("locale file is readable");
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("locale file contains valid JSON");
        if let Some(tray) = parsed.get("tray").cloned() {
            translations.insert(language.to_string(), tray);
        }
    }

    let english = translations
        .get("en")
        .and_then(serde_json::Value::as_object)
        .expect("English tray translations define the schema");
    let fields = english
        .keys()
        .map(|key| (camel_to_snake(key), key.clone()))
        .collect::<Vec<_>>();

    let mut output = String::from("// Auto-generated from locale files. Do not edit.\n\n");
    output.push_str("#[derive(Debug, Clone)]\npub struct TrayStrings {\n");
    for (field, _) in &fields {
        output.push_str(&format!("    pub {field}: String,\n"));
    }
    output.push_str("}\n\n");
    output.push_str(
        "pub static TRANSLATIONS: Lazy<HashMap<&'static str, TrayStrings>> = Lazy::new(|| {\n    let mut map = HashMap::new();\n",
    );
    for (language, tray) in &translations {
        output.push_str(&format!("    map.insert(\"{language}\", TrayStrings {{\n"));
        for (field, json_key) in &fields {
            let value = tray
                .get(json_key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_else(|| panic!("{language} tray.{json_key} must be a string"));
            output.push_str(&format!(
                "        {field}: \"{}\".to_string(),\n",
                escape_string(value)
            ));
        }
        output.push_str("    });\n");
    }
    output.push_str("    map\n});\n");

    fs::write(Path::new(&out_dir).join("tray_translations.rs"), output)
        .expect("generated tray translations are writable");
}

fn camel_to_snake(value: &str) -> String {
    value
        .chars()
        .enumerate()
        .fold(String::new(), |mut output, (index, character)| {
            if character.is_uppercase() && index > 0 {
                output.push('_');
            }
            output.extend(character.to_lowercase());
            output
        })
}

fn escape_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
