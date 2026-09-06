use crate::input::{self, TargetedModifier};
use crate::operation::ProcessingOperation;
use crate::settings::{get_settings, AutoSubmitKey, ClipboardHandling, PasteMethod};
use log::{info, warn};
use objc2_app_kit::NSWorkspace;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tokio::sync::oneshot;

// Some systems drop Cmd+V when Command is released too quickly. Releasing it
// is scheduled separately so the macOS main thread never waits for this hold.
pub(crate) const PASTE_CHORD_HOLD: Duration = Duration::from_millis(100);
const AUTO_SUBMIT_DELAY: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PasteOutcome {
    /// Cancellation won before an irreversible output action.
    Cancelled,
    /// Text or Cmd+V was posted to the validated target.
    Injected,
    /// Requested output was not delivered; the transcript remains recoverable.
    Skipped,
    /// The user selected PasteMethod::None; history may still be committed.
    IntentionalNone,
}

fn write_text_to_clipboard(app_handle: &AppHandle, text: &str) -> Result<(), String> {
    app_handle
        .clipboard()
        .write_text(text)
        .map_err(|e| format!("Failed to write to clipboard: {e}"))
}

fn complete_clipboard_only_output(
    operation: &ProcessingOperation,
    clipboard_handling: ClipboardHandling,
    write: impl FnOnce() -> Result<(), String>,
) -> PasteOutcome {
    let Some(_permit) = operation.try_enter_paste() else {
        return PasteOutcome::Cancelled;
    };
    if clipboard_handling == ClipboardHandling::CopyToClipboard {
        if let Err(error) = write() {
            warn!("Failed to copy transcription without pasting: {error}");
            return PasteOutcome::Skipped;
        }
    }
    PasteOutcome::IntentionalNone
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

pub(crate) struct PasteReleaseReceipt {
    receiver: oneshot::Receiver<Result<(), String>>,
}

impl PasteReleaseReceipt {
    /// Waits for the release callback unconditionally: cancellation may stop a
    /// later Return, but it must never strand Command down in the paste target.
    pub(crate) async fn wait(self) -> Result<(), String> {
        self.receiver
            .await
            .map_err(|_| "Targeted paste modifier release was dropped".to_string())?
    }
}

fn schedule_paste_modifier_release_with<F, R>(
    hold: Duration,
    pid: i32,
    dispatch: F,
    release: R,
) -> PasteReleaseReceipt
where
    F: FnOnce(Box<dyn FnOnce() + Send>) -> Result<(), String> + Send + 'static,
    R: FnOnce(i32) -> Result<(), String> + Send + 'static,
{
    let (sender, receiver) = oneshot::channel();
    let completion = Arc::new(Mutex::new(Some(sender)));
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(hold).await;
        let callback_completion = completion.clone();
        let queued = dispatch(Box::new(move || {
            let result = release(pid);
            if let Ok(mut slot) = callback_completion.lock() {
                if let Some(sender) = slot.take() {
                    let _ = sender.send(result);
                }
            }
        }));
        if let Err(error) = queued {
            if let Ok(mut slot) = completion.lock() {
                if let Some(sender) = slot.take() {
                    let _ = sender.send(Err(error));
                }
            }
        }
    });
    PasteReleaseReceipt { receiver }
}

fn schedule_paste_modifier_release(app_handle: AppHandle, pid: i32) -> PasteReleaseReceipt {
    schedule_paste_modifier_release_with(
        PASTE_CHORD_HOLD,
        pid,
        move |callback| {
            app_handle.run_on_main_thread(callback).map_err(|error| {
                format!("Failed to queue targeted paste modifier release: {error}")
            })
        },
        input::release_paste_modifier_to_pid,
    )
}

/// Sends the press portion of Cmd+V only after revalidating the captured app.
/// Its release is deliberately sent to the original PID even if cancellation
/// or a focus change follows the press.
pub(crate) fn send_paste_if_target_unchanged(
    captured: Option<FrontmostTarget>,
    app_handle: &AppHandle,
) -> Result<Option<PasteReleaseReceipt>, String> {
    let Some(target) = captured else {
        info!("Skipping paste because there is no stable target");
        return Ok(None);
    };
    if !same_frontmost_target(Some(target.clone()), frontmost_target()) {
        info!("Skipping paste because the frontmost application changed");
        return Ok(None);
    }
    input::send_paste_to_pid(target.pid)?;
    Ok(Some(schedule_paste_modifier_release(
        app_handle.clone(),
        target.pid,
    )))
}

#[derive(PartialEq, Eq)]
enum DirectStart {
    Cancelled,
    Injected,
    Skipped,
}

fn validated_direct_target() -> Option<FrontmostTarget> {
    let Some(target) = frontmost_target() else {
        info!("Skipping direct paste because there is no stable target");
        return None;
    };
    if !same_frontmost_target(Some(target.clone()), frontmost_target()) {
        info!("Skipping direct paste because the frontmost application changed");
        return None;
    }
    Some(target)
}

async fn complete_direct_auto_submit(
    app_handle: AppHandle,
    operation: ProcessingOperation,
    target: FrontmostTarget,
    auto_submit_key: AutoSubmitKey,
) -> Result<(), String> {
    let operation_for_submit = operation.clone();
    await_auto_submit_receipt(&app_handle, &operation, AUTO_SUBMIT_DELAY, move || {
        if operation_for_submit.cancellation_requested() {
            return;
        }
        if let Err(error) = send_return_if_target_unchanged(auto_submit_key, Some(target)) {
            warn!("Paste succeeded, but auto-submit failed: {error}");
        }
    })
    .await
}

async fn await_auto_submit_receipt<F>(
    app_handle: &AppHandle,
    operation: &ProcessingOperation,
    delay: Duration,
    action: F,
) -> Result<(), String>
where
    F: FnOnce() + Send + 'static,
{
    if wait_or_cancel(delay, operation).await {
        return Ok(());
    }

    let (sender, mut receiver) = oneshot::channel();
    app_handle
        .run_on_main_thread(move || {
            action();
            let _ = sender.send(());
        })
        .map_err(|error| format!("Failed to queue auto-submit: {error}"))?;

    tokio::select! {
        result = &mut receiver => result
            .map_err(|_| "Queued auto-submit was dropped before execution".to_string()),
        _ = operation.cancelled() => Ok(()),
    }
}

/// Runs `action` on AppKit's main thread but lets cancellation return without
/// waiting for a queued callback. If the callback already claimed output,
/// cancellation waits for its receipt so the caller learns whether it injected.
async fn await_main_or_cancel<T, F>(
    app_handle: &AppHandle,
    operation: &ProcessingOperation,
    action: F,
) -> Result<Option<T>, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if operation.cancellation_requested() {
        return Ok(None);
    }
    let (sender, mut receiver) = oneshot::channel();
    app_handle
        .run_on_main_thread(move || {
            let _ = sender.send(action());
        })
        .map_err(|error| format!("Failed to queue main-thread paste work: {error}"))?;

    tokio::select! {
        value = &mut receiver => value
            .map(Some)
            .map_err(|_| "Main-thread paste work was dropped before execution".to_string()),
        _ = operation.cancelled() => {
            if operation.is_cancelled() {
                Ok(None)
            } else {
                (&mut receiver).await
                    .map(Some)
                    .map_err(|_| "Main-thread paste work was dropped before execution".to_string())
            }
        }
    }
}

async fn wait_or_cancel(delay: Duration, operation: &ProcessingOperation) -> bool {
    if operation.cancellation_requested() {
        return true;
    }
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        _ = operation.cancelled() => true,
    }
}

pub(crate) async fn paste(
    text: String,
    app_handle: AppHandle,
    operation: ProcessingOperation,
) -> Result<PasteOutcome, String> {
    if operation.cancellation_requested() {
        return Ok(PasteOutcome::Cancelled);
    }

    let settings = get_settings(&app_handle);
    let paste_method = settings.paste_method;
    let text = if settings.append_trailing_space {
        format!("{text} ")
    } else {
        text
    };

    info!("Using paste method: {paste_method:?}");

    match paste_method {
        PasteMethod::None => {
            let text_for_main = text.clone();
            let app_for_main = app_handle.clone();
            let operation_for_main = operation.clone();
            let completed = await_main_or_cancel(&app_handle, &operation, move || {
                complete_clipboard_only_output(
                    &operation_for_main,
                    settings.clipboard_handling,
                    || write_text_to_clipboard(&app_for_main, &text_for_main),
                )
            })
            .await?;
            Ok(completed.unwrap_or(PasteOutcome::Cancelled))
        }
        PasteMethod::Direct => {
            let operation_for_main = operation.clone();
            let text_for_main = text.clone();
            let app_for_main = app_handle.clone();
            let start = await_main_or_cancel(&app_handle, &operation, move || {
                let Some(target) = validated_direct_target() else {
                    return (DirectStart::Skipped, None);
                };
                let Some(_permit) = operation_for_main.try_enter_paste() else {
                    return (DirectStart::Cancelled, None);
                };
                let result = match input::send_text_to_pid(target.pid, &text_for_main) {
                    Ok(()) => DirectStart::Injected,
                    Err(error) => {
                        warn!("Failed to paste transcription directly: {error}");
                        DirectStart::Skipped
                    }
                };
                if result == DirectStart::Injected
                    && settings.clipboard_handling == ClipboardHandling::CopyToClipboard
                {
                    if let Err(error) = write_text_to_clipboard(&app_for_main, &text_for_main) {
                        warn!("Failed to copy directly pasted transcription: {error}");
                    }
                }
                (result, Some(target))
            })
            .await?;

            match start {
                None | Some((DirectStart::Cancelled, _)) => Ok(PasteOutcome::Cancelled),
                Some((DirectStart::Skipped, _)) => Ok(PasteOutcome::Skipped),
                Some((DirectStart::Injected, Some(target))) => {
                    if settings.auto_submit {
                        complete_direct_auto_submit(
                            app_handle,
                            operation,
                            target,
                            settings.auto_submit_key,
                        )
                        .await?;
                    }
                    Ok(PasteOutcome::Injected)
                }
                Some((DirectStart::Injected, None)) => Ok(PasteOutcome::Skipped),
            }
        }
        PasteMethod::CtrlV => {
            let text_for_main = text.clone();
            let app_for_main = app_handle.clone();
            let operation_for_main = operation.clone();
            let reliable = await_main_or_cancel(&app_handle, &operation, move || {
                crate::paste_tx::try_reliable_paste(
                    &text_for_main,
                    &app_for_main,
                    settings.auto_submit,
                    settings.auto_submit_key,
                    settings.clipboard_handling,
                    operation_for_main,
                )
            })
            .await?;

            match reliable {
                None => Ok(PasteOutcome::Cancelled),
                Some(crate::paste_tx::ReliablePasteOutcome::Cancelled) => {
                    Ok(PasteOutcome::Cancelled)
                }
                Some(crate::paste_tx::ReliablePasteOutcome::Injected(receipt)) => {
                    receipt.complete_auto_submit(app_handle).await?;
                    Ok(PasteOutcome::Injected)
                }
                Some(crate::paste_tx::ReliablePasteOutcome::Skipped) => Ok(PasteOutcome::Skipped),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::OperationId;

    fn target(pid: i32) -> FrontmostTarget {
        FrontmostTarget {
            pid,
            bundle_identifier: format!("com.example.{pid}"),
            launch_time_bits: pid as u64,
        }
    }

    #[test]
    fn frontmost_target_comparison_fails_closed() {
        let first = Some(target(42));
        assert!(same_frontmost_target(first.clone(), first.clone()));
        assert!(!same_frontmost_target(first.clone(), Some(target(43))));
        assert!(!same_frontmost_target(first.clone(), None));
        assert!(!same_frontmost_target(None, first));
    }

    #[test]
    fn clipboard_only_failure_is_reported_as_recoverable_output() {
        let operation = ProcessingOperation::new(OperationId(10));
        let outcome =
            complete_clipboard_only_output(&operation, ClipboardHandling::CopyToClipboard, || {
                Err("clipboard unavailable".to_string())
            });

        assert_eq!(outcome, PasteOutcome::Skipped);
        assert!(!operation.cancel());
    }

    #[test]
    fn clipboard_only_success_completes_without_paste() {
        let operation = ProcessingOperation::new(OperationId(11));
        let mut wrote_text = false;
        let outcome =
            complete_clipboard_only_output(&operation, ClipboardHandling::CopyToClipboard, || {
                wrote_text = true;
                Ok(())
            });

        assert_eq!(outcome, PasteOutcome::IntentionalNone);
        assert!(wrote_text);
    }

    #[test]
    fn clipboard_only_cancel_and_history_only_mode_never_write() {
        let cancelled = ProcessingOperation::new(OperationId(12));
        assert!(cancelled.cancel());
        assert_eq!(
            complete_clipboard_only_output(
                &cancelled,
                ClipboardHandling::CopyToClipboard,
                || panic!("cancelled output must not touch the clipboard"),
            ),
            PasteOutcome::Cancelled
        );

        assert_eq!(
            complete_clipboard_only_output(
                &ProcessingOperation::new(OperationId(13)),
                ClipboardHandling::DontModify,
                || panic!("history-only output must not touch the clipboard"),
            ),
            PasteOutcome::IntentionalNone
        );
    }

    #[test]
    fn late_cancel_keeps_the_paste_but_marks_auto_submit_ineligible() {
        let operation = ProcessingOperation::new(OperationId(1));
        let _permit = operation
            .try_enter_paste()
            .expect("paste should claim output");

        assert!(!operation.cancel());
        assert!(!operation.is_cancelled());
        assert!(operation.cancellation_requested());
    }

    #[test]
    fn fake_main_queue_releases_before_late_cancel_can_reach_auto_submit() {
        type MainCallback = Box<dyn FnOnce() + Send>;

        struct FakeMainQueue {
            callbacks: Arc<Mutex<Vec<MainCallback>>>,
        }

        impl FakeMainQueue {
            fn enqueue(&self, callback: MainCallback) {
                self.callbacks.lock().unwrap().push(callback);
            }

            fn run_all(&self) {
                let callbacks = std::mem::take(&mut *self.callbacks.lock().unwrap());
                for callback in callbacks {
                    callback();
                }
            }
        }

        let queue = FakeMainQueue {
            callbacks: Arc::new(Mutex::new(Vec::new())),
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let (scheduled, scheduled_receipt) = oneshot::channel();
        let queue_for_dispatch = queue.callbacks.clone();
        let events_for_release = events.clone();
        let release = schedule_paste_modifier_release_with(
            Duration::ZERO,
            42,
            move |callback| {
                FakeMainQueue {
                    callbacks: queue_for_dispatch,
                }
                .enqueue(callback);
                let _ = scheduled.send(());
                Ok(())
            },
            move |_| {
                events_for_release.lock().unwrap().push("release");
                Ok(())
            },
        );

        tauri::async_runtime::block_on(async { scheduled_receipt.await.unwrap() });
        let operation = ProcessingOperation::new(OperationId(2));
        let _permit = operation.try_enter_paste().unwrap();
        assert!(!operation.cancel());

        queue.run_all();
        tauri::async_runtime::block_on(release.wait()).unwrap();
        if !operation.cancellation_requested() {
            events.lock().unwrap().push("return");
        }
        assert_eq!(events.lock().unwrap().as_slice(), &["release"]);
    }

    #[test]
    fn clipboard_source_has_no_blocking_thread_sleep() {
        let forbidden = ["thread", "::sleep"].concat();
        assert!(!include_str!("clipboard.rs").contains(&forbidden));
    }
}
