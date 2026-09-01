use crate::input::{self, TargetedModifier};
use crate::paste_tx::SETTLEMENT_TIMEOUT;
use crate::settings::{get_settings, AutoSubmitKey, ClipboardHandling, PasteMethod};
use log::info;
use objc2_app_kit::{NSPasteboard, NSWorkspace};
use objc2_foundation::{NSInteger, NSString};
use std::thread;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

// Kept at parity with the legacy path: some systems drop Cmd+V when the
// modifier is released too quickly.
const PASTE_CHORD_HOLD_MS: u64 = 100;
const TEXT_PASTEBOARD_TYPE: &str = "public.utf8-plain-text";

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

#[derive(Debug, PartialEq, Eq)]
enum FallbackClipboardAction {
    PasteSkipped,
    ClearAfterSafetyWindow,
    KeepTranscript,
}

fn fallback_clipboard_action(
    paste_injected: bool,
    clipboard_handling: ClipboardHandling,
) -> FallbackClipboardAction {
    if !paste_injected {
        FallbackClipboardAction::PasteSkipped
    } else if clipboard_handling == ClipboardHandling::DontModify {
        FallbackClipboardAction::ClearAfterSafetyWindow
    } else {
        FallbackClipboardAction::KeepTranscript
    }
}

fn same_change_count(published: NSInteger, current: NSInteger) -> bool {
    published == current
}

/// Publishes plain text without snapshotting or restoring earlier clipboard
/// data. A concurrent clipboard owner wins if it takes ownership between the
/// clear and the write.
fn publish_fallback_transcript(text: &str) -> Result<NSInteger, String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let change_count = pasteboard.clearContents();
    let payload = NSString::from_str(text);
    let text_type = NSString::from_str(TEXT_PASTEBOARD_TYPE);
    if !pasteboard.setString_forType(&payload, &text_type) {
        return Err("Clipboard changed while publishing the transcription".to_string());
    }
    Ok(change_count)
}

/// Erases Murmur's fallback text without taking ownership away from a newer
/// clipboard writer. `setString:forType:` fails if ownership changed after the
/// initial publication, including in the interval after the change-count test.
fn erase_fallback_transcript_if_owned(published_change_count: NSInteger) -> bool {
    let pasteboard = NSPasteboard::generalPasteboard();
    if !same_change_count(published_change_count, pasteboard.changeCount()) {
        return false;
    }
    let empty = NSString::from_str("");
    let text_type = NSString::from_str(TEXT_PASTEBOARD_TYPE);
    pasteboard.setString_forType(&empty, &text_type)
}

fn spawn_fallback_follow_up(
    app_handle: AppHandle,
    target: FrontmostTarget,
    published_at: Instant,
    published_change_count: NSInteger,
    auto_submit_key: Option<AutoSubmitKey>,
    clear_after_paste: bool,
) {
    thread::spawn(move || {
        if let Some(auto_submit_key) = auto_submit_key {
            thread::sleep(Duration::from_millis(50));
            if let Err(error) = app_handle.run_on_main_thread(move || {
                let current_change_count = NSPasteboard::generalPasteboard().changeCount();
                if !same_change_count(published_change_count, current_change_count) {
                    info!("Skipping fallback auto-submit because the paste was superseded");
                    return;
                }
                if let Err(error) =
                    send_return_if_target_unchanged(auto_submit_key, Some(target)).map(|_| ())
                {
                    log::warn!("Fallback paste succeeded, but auto-submit failed: {error}");
                }
            }) {
                log::warn!("Failed to queue fallback auto-submit on the main thread: {error}");
            }
        }

        if clear_after_paste {
            thread::sleep(SETTLEMENT_TIMEOUT.saturating_sub(published_at.elapsed()));
            if let Err(error) = app_handle.run_on_main_thread(move || {
                if erase_fallback_transcript_if_owned(published_change_count) {
                    info!("Erased fallback transcription without restoring prior clipboard data");
                } else {
                    info!("Clipboard changed before fallback cleanup; leaving it untouched");
                }
            }) {
                log::warn!("Failed to queue fallback clipboard cleanup: {error}");
            }
        }
    });
}

/// Conservative fallback for a guarded pasteboard transaction that could not
/// start. It never snapshots or restores previous clipboard data. A failed
/// injection leaves the transcript available for recovery.
fn paste_via_clipboard_without_restore(
    text: &str,
    app_handle: &AppHandle,
    paste_delay_ms: u64,
    paste_delay_after_ms: u64,
    clipboard_handling: ClipboardHandling,
    auto_submit: bool,
    auto_submit_key: AutoSubmitKey,
) -> Result<(), String> {
    let target = frontmost_target().ok_or("No stable frontmost paste target")?;
    let published_change_count = publish_fallback_transcript(text)?;
    let published_at = Instant::now();
    std::thread::sleep(Duration::from_millis(paste_delay_ms));
    let paste_injected = send_paste_if_target_unchanged(Some(target.clone()))?;
    let action = fallback_clipboard_action(paste_injected, clipboard_handling);
    if action == FallbackClipboardAction::PasteSkipped {
        return Err("Paste skipped because the frontmost target changed".to_string());
    }
    std::thread::sleep(Duration::from_millis(paste_delay_after_ms));

    spawn_fallback_follow_up(
        app_handle.clone(),
        target,
        published_at,
        published_change_count,
        auto_submit.then_some(auto_submit_key),
        action == FallbackClipboardAction::ClearAfterSafetyWindow,
    );
    Ok(())
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
            if !pasted {
                return Err("Direct paste skipped because the frontmost target changed".to_string());
            }
            if settings.auto_submit {
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
                settings.clipboard_handling,
                settings.auto_submit,
                settings.auto_submit_key,
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

    #[test]
    fn fallback_only_succeeds_after_targeted_paste_injection() {
        assert_eq!(
            fallback_clipboard_action(false, ClipboardHandling::DontModify),
            FallbackClipboardAction::PasteSkipped
        );
        assert_eq!(
            fallback_clipboard_action(true, ClipboardHandling::DontModify),
            FallbackClipboardAction::ClearAfterSafetyWindow
        );
        assert_eq!(
            fallback_clipboard_action(true, ClipboardHandling::CopyToClipboard),
            FallbackClipboardAction::KeepTranscript
        );
    }

    #[test]
    fn fallback_cleanup_requires_unchanged_clipboard_ownership() {
        assert!(same_change_count(12, 12));
        assert!(!same_change_count(12, 13));
    }
}
