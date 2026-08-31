use crate::input::{self, EnigoState};
use crate::settings::{get_settings, AutoSubmitKey, ClipboardHandling, PasteMethod};
use enigo::{Direction, Enigo, Key, Keyboard};
use log::info;
use objc2_app_kit::NSPasteboard;
use objc2_foundation::NSInteger;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

fn with_enigo<T>(
    app_handle: &AppHandle,
    f: impl FnOnce(&mut Enigo) -> Result<T, String>,
) -> Result<T, String> {
    let enigo_state = app_handle
        .try_state::<EnigoState>()
        .ok_or("Enigo state not initialized")?;
    let mut enigo = enigo_state
        .0
        .lock()
        .map_err(|e| format!("Failed to lock Enigo: {}", e))?;
    f(&mut enigo)
}

fn write_text_to_clipboard(app_handle: &AppHandle, text: &str) -> Result<(), String> {
    app_handle
        .clipboard()
        .write_text(text)
        .map_err(|e| format!("Failed to write to clipboard: {}", e))
}

#[derive(Debug, PartialEq, Eq)]
enum LegacyClipboardAction {
    RestoreSnapshot,
    KeepTranscript,
    LeaveUntouched,
}

fn decide_legacy_clipboard_action(
    published_change_count: NSInteger,
    current_change_count: NSInteger,
    clipboard_handling: ClipboardHandling,
) -> LegacyClipboardAction {
    if current_change_count != published_change_count {
        return LegacyClipboardAction::LeaveUntouched;
    }

    match clipboard_handling {
        ClipboardHandling::DontModify => LegacyClipboardAction::RestoreSnapshot,
        ClipboardHandling::CopyToClipboard => LegacyClipboardAction::KeepTranscript,
    }
}

fn finish_clipboard_paste(
    paste_result: Result<(), String>,
    paste_delay_after_ms: u64,
    restore_clipboard: impl FnOnce(),
) -> Result<(), String> {
    std::thread::sleep(Duration::from_millis(paste_delay_after_ms));
    restore_clipboard();
    paste_result
}

fn paste_via_clipboard(
    text: &str,
    app_handle: &AppHandle,
    paste_delay_ms: u64,
    paste_delay_after_ms: u64,
    clipboard_handling: ClipboardHandling,
) -> Result<(), String> {
    let clipboard = app_handle.clipboard();
    let saved_text = clipboard.read_text().ok().filter(|t| !t.is_empty());
    let saved_image = if saved_text.is_none() {
        clipboard.read_image().ok().map(|image| image.to_owned())
    } else {
        None
    };

    write_text_to_clipboard(app_handle, text)?;
    let published_change_count = NSPasteboard::generalPasteboard().changeCount();

    std::thread::sleep(Duration::from_millis(paste_delay_ms));

    let paste_result = with_enigo(app_handle, |enigo| input::send_paste_ctrl_v(enigo, 100));

    finish_clipboard_paste(paste_result, paste_delay_after_ms, || {
        let current_change_count = NSPasteboard::generalPasteboard().changeCount();

        if decide_legacy_clipboard_action(
            published_change_count,
            current_change_count,
            clipboard_handling,
        ) == LegacyClipboardAction::RestoreSnapshot
        {
            if let Some(clipboard_content) = saved_text {
                let _ = write_text_to_clipboard(app_handle, &clipboard_content);
            } else if let Some(image) = saved_image {
                info!("Restoring image to clipboard");
                let _ = clipboard.write_image(&image);
            } else {
                let _ = clipboard.clear();
            }
        }
    })
}

fn paste_direct(text: &str, app_handle: &AppHandle) -> Result<(), String> {
    with_enigo(app_handle, |enigo| input::paste_text_direct(enigo, text))
}

pub(crate) fn send_return_key(enigo: &mut Enigo, key_type: AutoSubmitKey) -> Result<(), String> {
    match key_type {
        AutoSubmitKey::Enter => {
            enigo
                .key(Key::Return, Direction::Press)
                .map_err(|e| format!("Failed to press Return key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Release)
                .map_err(|e| format!("Failed to release Return key: {}", e))?;
        }
        AutoSubmitKey::CtrlEnter => {
            enigo
                .key(Key::Control, Direction::Press)
                .map_err(|e| format!("Failed to press Control key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Press)
                .map_err(|e| format!("Failed to press Return key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Release)
                .map_err(|e| format!("Failed to release Return key: {}", e))?;
            enigo
                .key(Key::Control, Direction::Release)
                .map_err(|e| format!("Failed to release Control key: {}", e))?;
        }
        AutoSubmitKey::CmdEnter => {
            enigo
                .key(Key::Meta, Direction::Press)
                .map_err(|e| format!("Failed to press Meta/Cmd key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Press)
                .map_err(|e| format!("Failed to press Return key: {}", e))?;
            enigo
                .key(Key::Return, Direction::Release)
                .map_err(|e| format!("Failed to release Return key: {}", e))?;
            enigo
                .key(Key::Meta, Direction::Release)
                .map_err(|e| format!("Failed to release Meta/Cmd key: {}", e))?;
        }
    }

    Ok(())
}

fn should_send_auto_submit(auto_submit: bool, paste_method: PasteMethod) -> bool {
    auto_submit && paste_method != PasteMethod::None
}

pub fn paste(text: String, app_handle: AppHandle) -> Result<(), String> {
    let settings = get_settings(&app_handle);
    let paste_method = settings.paste_method;
    let paste_delay_ms = settings.paste_delay_ms;
    let paste_delay_after_ms = settings.paste_delay_after_ms;

    let text = if settings.append_trailing_space {
        format!("{} ", text)
    } else {
        text
    };

    info!(
        "Using paste method: {:?}, delay before: {}ms, delay after: {}ms",
        paste_method, paste_delay_ms, paste_delay_after_ms
    );

    match paste_method {
        PasteMethod::None => {
            info!("PasteMethod::None selected - skipping paste action");
        }
        PasteMethod::Direct => {
            paste_direct(&text, &app_handle)?;
        }
        PasteMethod::CtrlV => {
            if settings.reliable_paste {
                let reliable_result = with_enigo(&app_handle, |enigo| {
                    crate::paste_tx::try_reliable_paste(
                        &text,
                        &app_handle,
                        enigo,
                        settings.auto_submit,
                        settings.auto_submit_key,
                        settings.clipboard_handling,
                    )
                });
                match reliable_result {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        log::warn!("Reliable paste unavailable ({e}); falling back to legacy paste")
                    }
                }
            }
            paste_via_clipboard(
                &text,
                &app_handle,
                paste_delay_ms,
                paste_delay_after_ms,
                settings.clipboard_handling,
            )?
        }
    }

    if should_send_auto_submit(settings.auto_submit, paste_method) {
        std::thread::sleep(Duration::from_millis(50));
        if let Err(error) = with_enigo(&app_handle, |enigo| {
            send_return_key(enigo, settings.auto_submit_key)
        }) {
            log::warn!("Paste succeeded, but auto-submit failed: {error}");
        }
    }

    if settings.clipboard_handling == ClipboardHandling::CopyToClipboard
        && paste_method != PasteMethod::CtrlV
    {
        write_text_to_clipboard(&app_handle, &text)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn auto_submit_requires_setting_enabled() {
        assert!(!should_send_auto_submit(false, PasteMethod::CtrlV));
        assert!(!should_send_auto_submit(false, PasteMethod::Direct));
    }

    #[test]
    fn auto_submit_skips_none_paste_method() {
        assert!(!should_send_auto_submit(true, PasteMethod::None));
    }

    #[test]
    fn auto_submit_runs_for_active_paste_methods() {
        assert!(should_send_auto_submit(true, PasteMethod::CtrlV));
        assert!(should_send_auto_submit(true, PasteMethod::Direct));
    }

    #[test]
    fn clipboard_is_restored_before_key_injection_error_is_returned() {
        let restored = Cell::new(false);
        let result = finish_clipboard_paste(Err("input failed".into()), 0, || {
            restored.set(true);
        });

        assert_eq!(result.unwrap_err(), "input failed");
        assert!(restored.get());
    }

    #[test]
    fn restores_snapshot_when_murmur_still_owns_the_clipboard() {
        assert_eq!(
            decide_legacy_clipboard_action(42, 42, ClipboardHandling::DontModify,),
            LegacyClipboardAction::RestoreSnapshot
        );
    }

    #[test]
    fn doesnt_restore_snapshot_after_external_copy_of_identical_text() {
        assert_eq!(
            decide_legacy_clipboard_action(42, 43, ClipboardHandling::DontModify,),
            LegacyClipboardAction::LeaveUntouched
        );
    }

    #[test]
    fn doesnt_restore_snapshot_after_external_non_text_or_unreadable_change() {
        assert_eq!(
            decide_legacy_clipboard_action(42, 43, ClipboardHandling::DontModify,),
            LegacyClipboardAction::LeaveUntouched
        );
    }

    #[test]
    fn keeps_transcript_when_murmur_still_owns_the_clipboard() {
        assert_eq!(
            decide_legacy_clipboard_action(42, 42, ClipboardHandling::CopyToClipboard,),
            LegacyClipboardAction::KeepTranscript
        );
    }

    #[test]
    fn doesnt_preserve_transcript_after_external_copy_of_identical_text() {
        assert_eq!(
            decide_legacy_clipboard_action(42, 43, ClipboardHandling::CopyToClipboard,),
            LegacyClipboardAction::LeaveUntouched
        );
    }

    #[test]
    fn doesnt_preserve_transcript_after_external_non_text_or_unreadable_change() {
        assert_eq!(
            decide_legacy_clipboard_action(42, 43, ClipboardHandling::CopyToClipboard,),
            LegacyClipboardAction::LeaveUntouched
        );
    }
}
