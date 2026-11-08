use super::*;

/// Shows a short error message in the overlay for a few seconds. Returns
/// false when the overlay is disabled, so the caller can fall back to a
/// notification.
pub fn show_error_overlay(app_handle: &AppHandle, message: String) -> bool {
    if settings::get_settings(app_handle).overlay_style == OverlayStyle::None {
        return false;
    }
    let generation = {
        let mut held = notice();
        held.current = Some(Notice::Error);
        OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
    };
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if OVERLAY_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Some(window) = present_overlay_window(
            &handle,
            "error",
            ERROR_OVERLAY_WIDTH,
            ERROR_OVERLAY_HEIGHT,
            false,
        ) {
            let _ = window.emit("show-overlay-error", message);
        }
    });
    dismiss_notice_after(app_handle, generation, ERROR_DISPLAY_TIME, false);
    true
}

/// Keeps `text` as the copy notice's, for its Copy click to hand to `write`.
/// With `mic_lost`, the microphone warning follows the pill.
pub(crate) fn hold_copy_notice(text: String, write: ClipboardWriter, mic_lost: bool) -> u64 {
    let mut held = notice();
    held.current = Some(Notice::Copy {
        text,
        write,
        mic_lost_next: mic_lost,
    });
    OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

/// Shows the copy pill, a short note and a "Copy" action, because no text
/// field had the focus. A click hands the text to `write`, Escape dismisses
/// it. With `mic_lost`, the microphone warning shows once the pill goes, so
/// the two never share a pill. Returns false when the overlay is disabled,
/// so there is no pill to click. Only macOS Accessibility tells that no text
/// field has the focus, so Windows never shows it.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn show_copy_notice(
    app_handle: &AppHandle,
    text: String,
    write: ClipboardWriter,
    mic_lost: bool,
) -> bool {
    let settings = settings::get_settings(app_handle);
    if settings.overlay_style == OverlayStyle::None {
        return false;
    }
    let strings = get_tray_translations(Some(settings.app_language));
    let labels = CopyNoticeLabels {
        copy: strings.overlay_copy,
        copied: strings.overlay_copied,
        note: strings.overlay_not_pasted,
    };
    let generation = hold_copy_notice(text, write, mic_lost);
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if OVERLAY_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Some(window) = present_overlay_window(
            &handle,
            "copy",
            COPY_OVERLAY_WIDTH,
            COPY_OVERLAY_HEIGHT,
            true,
        ) {
            let _ = window.emit("show-overlay-copy", labels);
        }
    });
    dismiss_notice_after(app_handle, generation, COPY_DISPLAY_TIME, true);
    true
}

/// Holds the microphone warning as `operation`'s notice, unless the copy
/// pill shows: the warning follows it already, and its Copy must keep its
/// text. A cancelled operation holds nothing: a newer dictation may own the
/// overlay since its cancel.
pub(super) fn hold_mic_lost_notice(
    notice: &mut Option<Notice>,
    operation: &ProcessingOperation,
) -> bool {
    if matches!(notice, Some(Notice::Copy { .. })) || operation.is_cancelled() {
        return false;
    }
    *notice = Some(Notice::MicLost);
    true
}

/// Says the microphone went away during `operation`'s dictation, in a pill
/// shown and dismissed like the copy pill, without its action. Nothing shows
/// when the overlay is disabled or the operation was cancelled.
pub fn show_mic_lost_notice(app_handle: &AppHandle, operation: &ProcessingOperation) {
    let settings = settings::get_settings(app_handle);
    if settings.overlay_style == OverlayStyle::None {
        return;
    }
    // Held until the pill is queued, so a recording that starts after the
    // cancel check queues its own state after it, with a later generation.
    let mut held = notice();
    if !hold_mic_lost_notice(&mut held.current, operation) {
        return;
    }
    present_mic_lost_notice(app_handle, held);
}

/// Shows the microphone warning `held` holds, and lets the lock go once the
/// pill is queued.
pub(super) fn present_mic_lost_notice(app_handle: &AppHandle, held: MutexGuard<'static, Notices>) {
    let settings = settings::get_settings(app_handle);
    let message = get_tray_translations(Some(settings.app_language)).overlay_mic_disconnected;
    let generation = OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) = present_overlay_window(
            &handle,
            "mic-lost",
            COPY_OVERLAY_WIDTH,
            COPY_OVERLAY_HEIGHT,
            true,
        ) {
            let _ = window.emit("show-overlay-warning", message);
        }
    });
    drop(held);
    dismiss_notice_after(app_handle, generation, COPY_DISPLAY_TIME, true);
}

/// Holds the cancelled pill as the notice, unless a newer dictation took the
/// overlay since the cancelled one showed at `shown`. The pill comes from a
/// pipeline that unwinds after the cancel, so a newer dictation may have
/// shown its own since: its recorder is not open yet, or it already
/// transcribes, and the cancelled pill would cover it. The cancelled dictation
/// itself may still own the overlay, until the coordinator hides it, and keeps
/// its pill.
pub(super) fn hold_cancelled_notice(
    notice: &mut Option<Notice>,
    shown: DictationEpoch,
    now: DictationEpoch,
) -> bool {
    if now != shown {
        return false;
    }
    *notice = Some(Notice::Cancelled);
    true
}

/// Says the dictation that showed at `shown` was cancelled, with a Restore
/// button, in a pill shown and dismissed like the copy pill. Nothing shows
/// when the overlay is disabled or a newer dictation shows: the tray item
/// still restores it.
pub fn show_cancelled_notice(app_handle: &AppHandle, shown: DictationEpoch) {
    let settings = settings::get_settings(app_handle);
    if settings.overlay_style == OverlayStyle::None {
        return;
    }
    let strings = get_tray_translations(Some(settings.app_language));
    let labels = CancelledNoticeLabels {
        note: strings.overlay_cancelled,
        restore: strings.overlay_restore,
    };
    // Held until the pill is queued, so a dictation that shows after the
    // check queues its own state after it, with a later generation.
    let mut held = notice();
    if !hold_cancelled_notice(&mut held.current, shown, dictation_epoch()) {
        log::debug!("Leaving the new dictation's overlay over a late cancelled notice");
        return;
    }
    let generation = OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) = present_overlay_window(
            &handle,
            "cancelled",
            COPY_OVERLAY_WIDTH,
            COPY_OVERLAY_HEIGHT,
            true,
        ) {
            let _ = window.emit("show-overlay-cancelled", labels);
        }
    });
    drop(held);
    dismiss_notice_after(app_handle, generation, CANCELLED_DISPLAY_TIME, true);
}

/// The copy notice's window around a measured capsule: CARD_MARGIN on each
/// side, never smaller than the meeting pill's window, and never taller or wider
/// than the work area left below the top offset, so the capsule stays on
/// screen whichever edge it sits at.
pub(super) fn copy_window_size(capsule: (f64, f64), work_area: (f64, f64)) -> (f64, f64) {
    let fit = |measured: f64, least: f64, most: f64| {
        let wanted = if measured.is_finite() {
            measured.ceil() + 2.0 * CARD_MARGIN
        } else {
            least
        };
        wanted.min(most).max(least)
    };
    (
        fit(capsule.0, MEETING_OVERLAY_WIDTH, work_area.0),
        fit(
            capsule.1,
            MEETING_OVERLAY_HEIGHT,
            work_area.1 - OVERLAY_TOP_OFFSET,
        ),
    )
}

/// Hands the copy notice's text to its writer, outside the notice lock.
pub(crate) fn copy_held_notice() -> Result<(), String> {
    let (text, write) = match &notice().current {
        Some(Notice::Copy { text, write, .. }) => (text.clone(), Arc::clone(write)),
        _ => return Err("The copy notice is no longer showing".to_string()),
    };
    write(&text)
}

/// Whether the copy pill a Copy click came from still shows, so "Copied"
/// may take its place.
pub(super) fn copy_pill_showing(notice: &Option<Notice>) -> bool {
    matches!(notice, Some(Notice::Copy { .. }))
}

/// With `hover_holds`, the notice outlasts `delay` while the pointer is over
/// it, so it does not vanish under a click on its way.
pub(super) fn dismiss_notice_after(
    app_handle: &AppHandle,
    generation: u64,
    delay: Duration,
    hover_holds: bool,
) {
    let handle = app_handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        while OVERLAY_GENERATION.load(Ordering::SeqCst) == generation {
            if !(hover_holds && pointer_over_overlay(&handle)) {
                dismiss_notice(&handle, Dismissal::Timer(generation));
                return;
            }
            std::thread::sleep(COPY_HOVER_RECHECK);
        }
    });
}

/// What ends a notice: its own timer, started at a generation, or Escape on
/// a fitted pill.
#[derive(Clone, Copy)]
pub(super) enum Dismissal {
    Timer(u64),
    /// Windows watches no key, so only its tests dismiss with Escape.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    Escape,
}

/// What the overlay shows once a notice is dismissed.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum AfterDismissal {
    /// The microphone warning the copy pill held back.
    Warning,
    /// The meeting pill, or nothing.
    Settled,
}

/// Replaces `notice` with the one that follows it, when `dismissal` still
/// applies at `generation`, read under the notice lock: a timer the overlay
/// moved past, such as the copy pill's after Escape showed the warning that
/// follows it, or Escape with no fitted pill showing, leaves it alone and
/// gives `None`. With the overlay turned off since the copy pill showed,
/// nothing follows it.
pub(super) fn dismiss(
    notice: &mut Option<Notice>,
    dismissal: Dismissal,
    generation: u64,
    style: OverlayStyle,
) -> Option<AfterDismissal> {
    let applies = match dismissal {
        Dismissal::Timer(started) => started == generation,
        Dismissal::Escape => notice.as_ref().is_some_and(Notice::is_fitted),
    };
    if !applies {
        return None;
    }
    *notice = notice_after(notice.take()).filter(|_| style != OverlayStyle::None);
    Some(if matches!(notice, Some(Notice::MicLost)) {
        AfterDismissal::Warning
    } else {
        AfterDismissal::Settled
    })
}

pub(super) fn dismiss_notice(app_handle: &AppHandle, dismissal: Dismissal) {
    let style = settings::get_settings(app_handle).overlay_style;
    let mut held = notice();
    match dismiss(
        &mut held.current,
        dismissal,
        OVERLAY_GENERATION.load(Ordering::SeqCst),
        style,
    ) {
        None => {}
        Some(AfterDismissal::Warning) => present_mic_lost_notice(app_handle, held),
        Some(AfterDismissal::Settled) => {
            DICTATION_SHOWING.store(false, Ordering::SeqCst);
            OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst);
            drop(held);
            settle_overlay(app_handle);
        }
    }
}

/// The notice that takes the place of `gone`: the microphone warning a copy
/// pill held back, or none.
pub(super) fn notice_after(gone: Option<Notice>) -> Option<Notice> {
    match gone {
        Some(Notice::Copy {
            mic_lost_next: true,
            ..
        }) => Some(Notice::MicLost),
        _ => None,
    }
}
