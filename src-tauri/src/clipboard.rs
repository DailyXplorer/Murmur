use crate::input::{self, TargetedModifier};
use crate::settings::{get_settings, AutoSubmitKey, ClipboardHandling, PasteMethod};
use log::info;
use objc2_app_kit::NSWorkspace;
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

// Kept at parity with the legacy path: some systems drop Cmd+V when the
// modifier is released too quickly.
const PASTE_CHORD_HOLD_MS: u64 = 100;

fn write_text_to_clipboard(app_handle: &AppHandle, text: &str) -> Result<(), String> {
    app_handle
        .clipboard()
        .write_text(text)
        .map_err(|e| format!("Failed to write to clipboard: {}", e))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrontmostTarget {
    pid: i32,
    bundle_identifier: String,
    launch_time_bits: u64,
}

pub(crate) fn frontmost_target() -> Option<FrontmostTarget> {
    let application = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = application.processIdentifier();
    if pid <= 0 || u32::try_from(pid).ok() == Some(std::process::id()) {
        return None;
    }
    let bundle_identifier = application.bundleIdentifier()?.to_string();
    if bundle_identifier.is_empty() {
        return None;
    }
    let launch_time_bits = application
        .launchDate()?
        .timeIntervalSinceReferenceDate()
        .to_bits();
    Some(FrontmostTarget {
        pid,
        bundle_identifier,
        launch_time_bits,
    })
}

fn same_frontmost_target(
    captured: Option<FrontmostTarget>,
    current: Option<FrontmostTarget>,
) -> bool {
    matches!((captured, current), (Some(a), Some(b)) if a == b)
}

pub(crate) fn send_return_if_target_unchanged(
    key_type: AutoSubmitKey,
    captured: Option<FrontmostTarget>,
) -> Result<bool, String> {
    let Some(target) = captured else {
        info!("Skipping auto-submit because there is no stable target");
        return Ok(false);
    };
    if !same_frontmost_target(Some(target.clone()), frontmost_target()) {
        info!("Skipping auto-submit because the frontmost application changed");
        return Ok(false);
    }
    let modifier = match key_type {
        AutoSubmitKey::Enter => None,
        AutoSubmitKey::CtrlEnter => Some(TargetedModifier::Control),
        AutoSubmitKey::CmdEnter => Some(TargetedModifier::Command),
    };
    input::send_return_to_pid(target.pid, modifier)?;
    Ok(true)
}

pub(crate) fn send_paste_if_target_unchanged(
    captured: Option<FrontmostTarget>,
) -> Result<bool, String> {
    let Some(target) = captured else {
        info!("Skipping paste because there is no stable target");
        return Ok(false);
    };
    if !same_frontmost_target(Some(target.clone()), frontmost_target()) {
        info!("Skipping paste because the frontmost application changed");
        return Ok(false);
    }
    input::send_paste_to_pid(target.pid, PASTE_CHORD_HOLD_MS)?;
    Ok(true)
}

/// Conservative fallback for a platform pasteboard transaction that could not
/// start. It never snapshots or restores previous clipboard data: the
/// transcript remains available, so a delayed Cmd+V cannot read an old secret.
fn paste_via_clipboard_without_restore(
    text: &str,
    app_handle: &AppHandle,
    paste_delay_ms: u64,
    paste_delay_after_ms: u64,
) -> Result<(), String> {
    let target = frontmost_target().ok_or("No stable frontmost paste target")?;
    write_text_to_clipboard(app_handle, text)?;
    std::thread::sleep(Duration::from_millis(paste_delay_ms));
    let paste_result = send_paste_if_target_unchanged(Some(target)).map(|_| ());
    std::thread::sleep(Duration::from_millis(paste_delay_after_ms));
    paste_result
}

fn paste_direct_if_target_unchanged(
    text: &str,
    captured: Option<FrontmostTarget>,
) -> Result<bool, String> {
    let Some(target) = captured else {
        info!("Skipping direct paste because there is no stable target");
        return Ok(false);
    };
    if !same_frontmost_target(Some(target.clone()), frontmost_target()) {
        info!("Skipping direct paste because the frontmost application changed");
        return Ok(false);
    }
    input::send_text_to_pid(target.pid, text)?;
    Ok(true)
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
            let target = frontmost_target();
            let pasted = paste_direct_if_target_unchanged(&text, target.clone())?;
            if pasted && settings.auto_submit {
                std::thread::sleep(Duration::from_millis(50));
                if let Err(error) =
                    send_return_if_target_unchanged(settings.auto_submit_key, target).map(|_| ())
                {
                    log::warn!("Paste succeeded, but auto-submit failed: {error}");
                }
            }
        }
        PasteMethod::CtrlV => {
            let reliable_result = crate::paste_tx::try_reliable_paste(
                &text,
                &app_handle,
                settings.auto_submit,
                settings.auto_submit_key,
                settings.clipboard_handling,
            );
            match reliable_result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    log::warn!(
                        "Guarded paste unavailable ({error}); leaving the transcript on the clipboard"
                    );
                }
            }
            paste_via_clipboard_without_restore(
                &text,
                &app_handle,
                paste_delay_ms,
                paste_delay_after_ms,
            )?;
            return Ok(());
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

    #[test]
    fn frontmost_target_comparison_fails_closed() {
        let first = Some(FrontmostTarget {
            pid: 42,
            bundle_identifier: "com.example.first".to_string(),
            launch_time_bits: 1,
        });
        assert!(same_frontmost_target(first.clone(), first.clone()));
        assert!(!same_frontmost_target(
            first.clone(),
            Some(FrontmostTarget {
                pid: 43,
                bundle_identifier: "com.example.first".to_string(),
                launch_time_bits: 1,
            })
        ));
        assert!(!same_frontmost_target(
            first.clone(),
            Some(FrontmostTarget {
                pid: 42,
                bundle_identifier: "com.example.second".to_string(),
                launch_time_bits: 2,
            })
        ));
        assert!(!same_frontmost_target(first.clone(), None));
        assert!(!same_frontmost_target(None, first));
        assert!(!same_frontmost_target(None, None));
    }
}
