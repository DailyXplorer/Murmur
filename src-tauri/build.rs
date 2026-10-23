const APP_COMMANDS: &[&str] = &[
    "change_binding",
    "reset_binding",
    "get_shortcut_registration_errors",
    "get_key_labels",
    "change_audio_feedback_setting",
    "change_audio_feedback_volume_setting",
    "change_sound_theme_setting",
    "change_theme_setting",
    "change_accent_color_setting",
    "change_start_hidden_setting",
    "change_autostart_setting",
    "change_selected_language_setting",
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
    "change_ai_cleanup_setting",
    "change_dictation_style_setting",
    "change_window_names_setting",
    "change_whisper_mode_setting",
    "change_app_language_setting",
    "change_update_checks_setting",
    "change_show_whats_new_on_update_setting",
    "change_whats_new_last_seen_version_setting",
    "change_show_tray_icon_setting",
    "show_main_window_command",
    "relaunch_after_update",
    "get_update_status",
    "check_for_updates",
    "cancel_operation",
    "copy_notice_again",
    "fit_copy_notice",
    "get_app_dir_path",
    "get_app_settings",
    "get_default_settings",
    "get_log_dir_path",
    "set_log_level",
    "open_recordings_folder",
    "open_log_dir",
    "open_app_data_dir",
    "repair_accessibility_permission",
    "open_microphone_settings",
    "get_microphone_authorization_status",
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
    "sign_in_with_chatgpt",
    "cancel_chatgpt_sign_in",
    "sign_out_of_chatgpt",
    "complete_onboarding",
    "change_transcription_engine_setting",
    "switch_to_local_when_ready",
    "change_local_stt_card_dismissed_setting",
    "get_history_entries",
    "get_insights",
    "toggle_history_entry_saved",
    "get_audio_file_path",
    "delete_history_entry",
    "retry_history_entry_transcription",
    "update_history_limit",
    "update_recording_retention_period",
    "is_laptop",
    "get_meeting_runtime",
    "start_meeting",
    "stop_meeting",
    "discard_meeting",
    "get_meetings",
    "delete_meeting",
    "get_meeting",
    "rename_meeting",
    "rename_speaker",
    "retry_meeting",
    "reanalyze_meeting",
    "retry_meeting_summary",
    "delete_meeting_audio",
    "retranscribe_meeting",
    "get_local_model_status",
    "download_local_model",
    "cancel_local_model_download",
    "delete_local_model",
    "copy_meeting_markdown",
    "export_meeting_markdown",
    "get_meeting_capabilities",
    "get_system_audio_permission",
    "open_system_audio_privacy_settings",
    "change_meetings_enabled_setting",
    "change_meeting_capture_mode_setting",
    "change_meeting_audio_retention_setting",
    "get_meetings_storage",
    "change_meeting_call_prompt_setting",
    "answer_call_prompt",
    "fit_call_prompt",
    "get_call_prompt",
];

fn main() {
    generate_tray_translations();
    build_swift_bridges();
    build_meetings_helper();
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS));
    let attributes = embed_windows_manifest(attributes);
    tauri_build::try_build(attributes).expect("failed to build Tauri application ACL");
}

/// Tauri puts its Windows manifest (Common Controls 6, which WebView2 and
/// tauri need) in a resource linked into the app binary only, so the test
/// binaries fail to start with STATUS_ENTRYPOINT_NOT_FOUND. The linker
/// embeds the same manifest into every binary instead.
fn embed_windows_manifest(attributes: tauri_build::Attributes) -> tauri_build::Attributes {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return attributes;
    }
    let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    attributes.windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
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

/// Compiles each Swift bridge in `swift/` into a static library: the meeting
/// system-audio tap and the meeting audio compression. The tap needs Apple
/// silicon, so Intel builds get Rust's stubs for it; compression only needs
/// AudioToolbox, which Intel Macs have too.
fn build_swift_bridges() {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    // The tap comes first: Intel builds skip it.
    const BRIDGES: [(&str, &str, &str); 2] = [
        (
            "swift/meeting_audio.swift",
            "MurmurMeetingAudio",
            "murmur_meeting_audio",
        ),
        (
            "swift/meeting_archive.swift",
            "MurmurMeetingArchive",
            "murmur_meeting_archive",
        ),
    ];
    for (source, _, _) in BRIDGES {
        println!("cargo:rerun-if-changed={source}");
    }
    println!("cargo:rerun-if-env-changed=SDKROOT");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let (bridges, swift_target) = match (target_os.as_str(), target_arch.as_str()) {
        ("macos", "aarch64") => (&BRIDGES[..], "arm64-apple-macosx11.0"),
        ("macos", "x86_64") => (&BRIDGES[1..], "x86_64-apple-macosx11.0"),
        _ => return,
    };

    let xcrun = |args: &[&str]| -> String {
        let output = Command::new("xcrun")
            .args(args)
            .output()
            .expect("xcrun runs");
        assert!(output.status.success(), "xcrun {args:?} failed");
        String::from_utf8(output.stdout)
            .expect("xcrun prints UTF-8")
            .trim()
            .to_string()
    };
    let sdk = std::env::var("SDKROOT")
        .ok()
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| xcrun(&["--sdk", "macosx", "--show-sdk-path"]));
    let swiftc = xcrun(&["--sdk", "macosx", "--find", "swiftc"]);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by Cargo"));

    for &(source, module, library) in bridges {
        let object = out_dir.join(format!("{library}.o"));
        let status = Command::new(&swiftc)
            .args(["-parse-as-library", "-swift-version", "5", "-O", "-c"])
            .args(["-target", swift_target, "-sdk", &sdk])
            // The bridges use no Swift concurrency, but Foundation's import
            // still asks to link it. Below macOS 12 that is
            // `@rpath/libswift_Concurrency.dylib`, which the bundle does not
            // ship.
            .args([
                "-Xfrontend",
                "-disable-autolink-library",
                "-Xfrontend",
                "swift_Concurrency",
                "-Xfrontend",
                "-disable-autolinking-runtime-compatibility-concurrency",
            ])
            .arg("-module-name")
            .arg(module)
            .arg(source)
            .arg("-o")
            .arg(&object)
            .status()
            .expect("swiftc runs");
        assert!(status.success(), "swiftc failed to compile {source}");
        let status = Command::new("libtool")
            .args(["-static", "-o"])
            .arg(out_dir.join(format!("lib{library}.a")))
            .arg(&object)
            .status()
            .expect("libtool runs");
        assert!(
            status.success(),
            "libtool failed to archive the Swift bridge {source}"
        );
        println!("cargo:rustc-link-lib=static={library}");
    }

    let toolchain_swift = Path::new(&swiftc)
        .parent()
        .and_then(Path::parent)
        .expect("swiftc sits in <toolchain>/usr/bin")
        .join("lib/swift/macosx");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!(
        "cargo:rustc-link-search=native={}",
        toolchain_swift.display()
    );
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=CoreAudio");
    println!("cargo:rustc-link-lib=framework=AudioToolbox");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}

/// Builds `murmur-meetings-helper`, the on-device meeting analysis (FluidAudio,
/// macOS 14, Apple silicon), where Tauri bundles it from: `binaries/` with the
/// target triple. Intel builds get a stub that reports the analysis as
/// unsupported, so meetings fall back to one speaker per track.
fn build_meetings_helper() {
    use std::path::Path;
    use std::process::Command;

    const PACKAGE: &str = "swift/MurmurMeetingsHelper";
    for path in ["Package.swift", "Package.resolved", "Sources"] {
        println!("cargo:rerun-if-changed={PACKAGE}/{path}");
    }
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        return;
    }
    let target = std::env::var("TARGET").expect("TARGET is set by Cargo");
    std::fs::create_dir_all("binaries").expect("binaries/ is writable");
    let destination = Path::new("binaries").join(format!("murmur-meetings-helper-{target}"));
    println!("cargo:rerun-if-changed={}", destination.display());

    if target == "aarch64-apple-darwin" {
        let swift_build = |extra: &[&str]| {
            Command::new("swift")
                .args([
                    "build",
                    "-c",
                    "release",
                    "--arch",
                    "arm64",
                    "--package-path",
                ])
                .arg(PACKAGE)
                .args(extra)
                .output()
                .expect("swift runs (Xcode 26 or later, with Swift 6.2, is needed for the meetings helper)")
        };
        let built = swift_build(&[]);
        assert!(
            built.status.success(),
            "swift build failed for {PACKAGE}:\n{}{}",
            String::from_utf8_lossy(&built.stdout),
            String::from_utf8_lossy(&built.stderr)
        );
        // Where the products go depends on the toolchain.
        let bin = swift_build(&["--show-bin-path"]);
        assert!(bin.status.success(), "swift cannot tell its build folder");
        let bin = String::from_utf8(bin.stdout).expect("swift prints UTF-8");
        std::fs::copy(
            Path::new(bin.trim()).join("murmur-meetings-helper"),
            &destination,
        )
        .expect("the built helper can be copied");
        return;
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by Cargo");
    let source = Path::new(&out_dir).join("meetings_helper_stub.c");
    std::fs::write(
        &source,
        "#include <stdio.h>\nint main(void) {\n  puts(\"{\\\"type\\\":\\\"error\\\",\\\"stage\\\":\\\"analyze\\\",\\\"code\\\":\\\"unsupported\\\",\\\"message\\\":\\\"needs Apple silicon\\\"}\");\n  puts(\"{\\\"type\\\":\\\"done\\\"}\");\n  return 0;\n}\n",
    )
    .expect("the helper stub source is writable");
    let arch = target.split('-').next().unwrap_or("x86_64");
    let status = Command::new("cc")
        .args(["-arch", arch, "-mmacosx-version-min=10.15", "-O2", "-o"])
        .arg(&destination)
        .arg(&source)
        .status()
        .expect("cc runs");
    assert!(
        status.success(),
        "cc failed to build the meetings helper stub"
    );
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
