use super::*;
use std::fs;

#[cfg(target_os = "windows")]
#[test]
fn windows_records_with_ctrl_and_win_held_by_default() {
    let settings = get_default_settings();
    assert_eq!(
        settings.bindings["transcribe"].current_binding,
        "ctrl+super"
    );
    assert!(crate::shortcut::tauri_impl::validate_binding("ctrl+super").is_ok());
    // Modifiers alone are a Windows chord; a single one is not.
    assert!(crate::shortcut::tauri_impl::validate_binding("ctrl").is_err());
}

#[cfg(target_os = "windows")]
#[test]
fn windows_defaults_have_what_the_windows_build_offers() {
    let settings = get_default_settings();
    assert!(settings.meetings_enabled);
    assert!(settings.meetings_turned_on_once);
    assert!(settings.window_names_enabled);
    assert!(settings.update_checks_enabled);
    assert!(settings.update_checks_turned_on_once);
}

#[cfg(target_os = "windows")]
#[test]
fn a_store_with_macos_only_features_is_restricted_on_windows() {
    let mut stored = default_settings_json();
    stored["meetings_enabled"] = serde_json::json!(true);
    stored["clamshell_microphone"] = serde_json::json!("Built-in Microphone");
    stored["selected_language"] = serde_json::json!("fr");

    let (settings, updated) = parse_stored_settings(&stored);

    assert!(updated);
    assert!(settings.meetings_enabled);
    assert!(settings.update_checks_enabled);
    assert!(settings.clamshell_microphone.is_none());
    assert_eq!(settings.selected_language, "fr");
    let (_, rewritten) = parse_stored_settings(&serde_json::to_value(settings).unwrap());
    assert!(!rewritten);
}

#[cfg(target_os = "windows")]
#[test]
fn window_context_is_turned_on_once_for_a_beta_store() {
    // A store this version cannot read whole is salvaged field by field
    // over the defaults, which already carry the marker.
    for paste_method in [None, Some("shift_insert")] {
        let mut stored = default_settings_json();
        stored["window_names_enabled"] = serde_json::json!(false);
        if let Some(paste_method) = paste_method {
            stored["paste_method"] = serde_json::json!(paste_method);
        }
        stored
            .as_object_mut()
            .unwrap()
            .remove("window_names_turned_on_once");

        let (mut settings, updated) = parse_stored_settings(&stored);
        assert!(updated, "{paste_method:?}");
        assert!(settings.window_names_enabled, "{paste_method:?}");

        // Turned off afterwards, it stays off.
        settings.window_names_enabled = false;
        let (settings, updated) = parse_stored_settings(&serde_json::to_value(settings).unwrap());
        assert!(!updated, "{paste_method:?}");
        assert!(!settings.window_names_enabled, "{paste_method:?}");
    }
}

#[cfg(target_os = "windows")]
#[test]
fn meetings_are_turned_on_once_for_a_beta_store() {
    // A store this version cannot read whole is salvaged field by field
    // over the defaults, which already carry the marker.
    for paste_method in [None, Some("shift_insert")] {
        let mut stored = default_settings_json();
        stored["meetings_enabled"] = serde_json::json!(false);
        if let Some(paste_method) = paste_method {
            stored["paste_method"] = serde_json::json!(paste_method);
        }
        stored
            .as_object_mut()
            .unwrap()
            .remove("meetings_turned_on_once");
        stored["bindings"]["meeting_toggle"]["current_binding"] =
            serde_json::json!(crate::meetings::PREVIOUS_DEFAULT_BINDING);
        stored["bindings"]["meeting_toggle"]["default_binding"] =
            serde_json::json!(crate::meetings::PREVIOUS_DEFAULT_BINDING);

        let (mut settings, updated) = parse_stored_settings(&stored);
        assert!(updated, "{paste_method:?}");
        assert!(settings.meetings_enabled, "{paste_method:?}");
        assert!(settings.meetings_turned_on_once, "{paste_method:?}");
        let binding = &settings.bindings[crate::meetings::BINDING_ID];
        assert_eq!(binding.current_binding, crate::meetings::DEFAULT_BINDING);
        assert_eq!(binding.default_binding, crate::meetings::DEFAULT_BINDING);

        // The user's own "off" is kept afterwards.
        settings.meetings_enabled = false;
        let (kept, rewritten) = parse_stored_settings(&serde_json::to_value(settings).unwrap());
        assert!(!kept.meetings_enabled, "{paste_method:?}");
        assert!(!rewritten, "{paste_method:?}");
    }
}

#[cfg(target_os = "windows")]
#[test]
fn a_shortcut_the_user_chose_for_meetings_survives_the_one_time_switch_on() {
    let mut stored = default_settings_json();
    stored["meetings_enabled"] = serde_json::json!(false);
    stored
        .as_object_mut()
        .unwrap()
        .remove("meetings_turned_on_once");
    stored["bindings"]["meeting_toggle"]["current_binding"] = serde_json::json!("ctrl+shift+f9");

    let (settings, _) = parse_stored_settings(&stored);

    assert_eq!(
        settings.bindings[crate::meetings::BINDING_ID].current_binding,
        "ctrl+shift+f9"
    );
}

#[cfg(target_os = "windows")]
#[test]
fn update_checks_are_turned_on_once_for_a_beta_store() {
    // A salvaged store takes the marker from the defaults, already set.
    for paste_method in [None, Some("shift_insert")] {
        let mut stored = default_settings_json();
        stored["update_checks_enabled"] = serde_json::json!(false);
        if let Some(paste_method) = paste_method {
            stored["paste_method"] = serde_json::json!(paste_method);
        }
        stored
            .as_object_mut()
            .unwrap()
            .remove("update_checks_turned_on_once");

        let (mut settings, updated) = parse_stored_settings(&stored);
        assert!(updated, "{paste_method:?}");
        assert!(settings.update_checks_enabled, "{paste_method:?}");
        assert!(settings.update_checks_turned_on_once, "{paste_method:?}");

        // Turned off afterwards, it stays off.
        settings.update_checks_enabled = false;
        let (settings, updated) = parse_stored_settings(&serde_json::to_value(settings).unwrap());
        assert!(!updated, "{paste_method:?}");
        assert!(!settings.update_checks_enabled, "{paste_method:?}");
    }
}

#[cfg(target_os = "windows")]
#[test]
fn a_store_with_updates_marked_done_keeps_the_users_off() {
    let mut stored = default_settings_json();
    stored["update_checks_enabled"] = serde_json::json!(false);
    stored["update_checks_turned_on_once"] = serde_json::json!(true);

    let (settings, updated) = parse_stored_settings(&stored);

    assert!(!updated);
    assert!(!settings.update_checks_enabled);
}

#[cfg(target_os = "windows")]
#[test]
fn persisted_windows_settings_keep_restrictions_and_migrate_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SETTINGS_STORE_PATH);
    let mut stored = default_settings_json();
    stored["meetings_enabled"] = serde_json::json!(false);
    stored["update_checks_enabled"] = serde_json::json!(false);
    stored["clamshell_microphone"] = serde_json::json!("Built-in Microphone");
    stored["window_names_enabled"] = serde_json::json!(false);
    stored["overlay_position"] = serde_json::json!("from-a-newer-build");
    let fields = stored.as_object_mut().unwrap();
    fields.remove("window_names_turned_on_once");
    fields.remove("update_checks_turned_on_once");
    fields.remove("meetings_turned_on_once");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({ "settings": stored })).unwrap(),
    )
    .unwrap();

    let file = file::SettingsFile::open(path.clone());
    let mut settings = read_from(&file);
    assert!(settings.meetings_enabled);
    assert!(settings.meetings_turned_on_once);
    assert!(settings.update_checks_enabled);
    assert!(settings.update_checks_turned_on_once);
    assert!(settings.clamshell_microphone.is_none());
    assert!(settings.window_names_enabled);
    assert!(settings.window_names_turned_on_once);
    file.flush();
    let persisted: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let persisted: AppSettings = serde_json::from_value(persisted["settings"].clone()).unwrap();
    assert!(persisted.meetings_enabled);
    assert!(persisted.meetings_turned_on_once);
    assert!(persisted.update_checks_enabled);
    assert!(persisted.update_checks_turned_on_once);
    assert!(persisted.clamshell_microphone.is_none());
    assert!(persisted.window_names_turned_on_once);
    assert!(persisted.window_names_enabled);

    settings.window_names_enabled = false;
    settings.update_checks_enabled = false;
    settings.meetings_enabled = false;
    file.store_checked(serde_json::to_value(settings).unwrap())
        .unwrap();
    drop(file);

    let reloaded = read_from(&file::SettingsFile::open(path));
    assert!(!reloaded.window_names_enabled);
    assert!(reloaded.window_names_turned_on_once);
    assert!(!reloaded.meetings_enabled);
    assert!(reloaded.meetings_turned_on_once);
    assert!(!reloaded.update_checks_enabled);
    assert!(reloaded.update_checks_turned_on_once);
    assert!(reloaded.clamshell_microphone.is_none());
}
