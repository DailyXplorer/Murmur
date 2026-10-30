//! Windows reliable paste.
//!
//! Publishes the transcript as a delayed-render clipboard format
//! (`SetClipboardData(CF_UNICODETEXT, NULL)`) owned by a hidden message-only
//! window. Windows sends that window `WM_RENDERFORMAT` when a consumer reads
//! the text. Like the macOS promise callback it does not say who read, so it
//! is diagnostic only: it never governs timing, auto-submit, or restoration.
//!
//! Threading: clipboard ownership and delayed rendering belong to a window
//! whose messages need a pump, so each transaction runs on its own thread
//! with its own window. Every clipboard write goes through that window:
//! after `OpenClipboard(NULL)`, `EmptyClipboard` leaves the clipboard without
//! an owner and `SetClipboardData` fails. The paste chord is sent from the
//! calling (main) thread once the transcript is published.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, Once};
use std::time::{Duration, Instant};

use log::{error, info};
use tauri::AppHandle;
use tokio::sync::oneshot;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GlobalFree, SetLastError, ERROR_SUCCESS, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT,
    WPARAM,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardOwner,
    OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::{
    CF_BITMAP, CF_DSPBITMAP, CF_DSPENHMETAFILE, CF_DSPMETAFILEPICT, CF_ENHMETAFILE, CF_GDIOBJFIRST,
    CF_GDIOBJLAST, CF_METAFILEPICT, CF_OWNERDISPLAY, CF_PALETTE, CF_PRIVATEFIRST, CF_PRIVATELAST,
    CF_UNICODETEXT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, KillTimer, PostMessageW, PostQuitMessage, RegisterClassW, SetTimer,
    SetWindowLongPtrW, GWLP_USERDATA, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_DESTROYCLIPBOARD, WM_RENDERALLFORMATS, WM_RENDERFORMAT, WM_TIMER, WNDCLASSW,
};

use super::{
    claim_before_pasteboard_mutation, decide_settlement, evaluate, same_pending, CaptureBudget,
    CaptureLimit, ClipboardSettlement, PasteboardAdmission, ReliablePasteOutcome, TxState,
    WaitDecision, CAPTURE_BUDGET,
};
use crate::clipboard::{
    frontmost_target, send_paste_if_target_unchanged, send_return_if_target_unchanged,
    FrontmostTarget, PasteReleaseReceipt,
};
use crate::operation::ProcessingOperation;
use crate::settings::{AutoSubmitKey, ClipboardHandling};

const CLASS_NAME: PCWSTR = w!("MurmurPasteTransaction");
const TIMER_ID: usize = 1;
/// The polling period of the macOS waiter.
const TIMER_INTERVAL_MS: u32 = 15;
/// Asks a transaction's window to settle at once: a newer paste, or quitting.
const WM_SETTLE_NOW: u32 = WM_APP + 1;
/// How long a newer paste, or quitting, waits for an older transaction to
/// put the clipboard back.
const SETTLE_WAIT: Duration = Duration::from_secs(1);
/// How long the paste waits for its transaction to publish the transcript.
const PUBLISH_WAIT: Duration = Duration::from_secs(2);
/// Another app may hold the clipboard open for a moment.
const OPEN_ATTEMPTS: u32 = 10;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(10);

/// Formats published beside the transcript so Windows keeps it out of
/// clipboard history and the cloud clipboard, as Microsoft documents them
/// ("Cloud Clipboard and Clipboard History Formats").
const EXCLUSION_FORMATS: [(PCWSTR, u32); 3] = [
    (w!("ExcludeClipboardContentFromMonitorProcessing"), 1),
    (w!("CanIncludeInClipboardHistory"), 0),
    (w!("CanUploadToCloudClipboard"), 0),
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct SavedFormat {
    format: u32,
    data: Vec<u8>,
}

/// Whether a format holds plain global memory that survives a byte copy.
/// Bitmaps, metafiles, palettes, owner-display and private formats are
/// handles to objects instead. A copied image is still saved: Windows
/// synthesizes its DIB, which is plain memory, and the bitmap from that DIB.
fn is_byte_copyable(format: u32) -> bool {
    let handles = [
        CF_BITMAP,
        CF_METAFILEPICT,
        CF_PALETTE,
        CF_ENHMETAFILE,
        CF_OWNERDISPLAY,
        CF_DSPBITMAP,
        CF_DSPMETAFILEPICT,
        CF_DSPENHMETAFILE,
    ];
    let within = |first: u16, last: u16| (u32::from(first)..=u32::from(last)).contains(&format);
    !handles.iter().any(|handle| u32::from(handle.0) == format)
        && !within(CF_PRIVATEFIRST.0, CF_PRIVATELAST.0)
        && !within(CF_GDIOBJFIRST.0, CF_GDIOBJLAST.0)
}

/// Reads every byte-copyable format through `read`, checking both budgets
/// between reads. A single read cannot be interrupted.
fn capture_with(
    formats: Vec<u32>,
    budget: CaptureBudget,
    mut read: impl FnMut(u32) -> Option<Vec<u8>>,
) -> Result<Vec<SavedFormat>, CaptureLimit> {
    let started = Instant::now();
    let mut total_bytes = 0usize;
    let mut saved = Vec::new();
    for format in formats
        .into_iter()
        .filter(|format| is_byte_copyable(*format))
    {
        if started.elapsed() >= budget.max_duration {
            return Err(CaptureLimit::Time);
        }
        let Some(data) = read(format) else {
            continue;
        };
        total_bytes = total_bytes.saturating_add(data.len());
        if total_bytes > budget.max_bytes {
            return Err(CaptureLimit::Bytes);
        }
        saved.push(SavedFormat { format, data });
    }
    Ok(saved)
}

/// `CF_UNICODETEXT` contents: UTF-16LE with a terminating NUL.
fn unicode_text_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

const PUBLICATION_WAITING: u8 = 0;
const PUBLICATION_STARTED: u8 = 1;
const PUBLICATION_ABANDONED: u8 = 2;

/// The timeout and the worker claim publication atomically. A slow clipboard
/// read must never publish after the caller has abandoned it. Once publication
/// starts, its transaction owns recovery even if the acknowledgement is late.
#[derive(Default)]
struct PublicationGate(AtomicU8);

impl PublicationGate {
    fn begin(&self) -> bool {
        self.0
            .compare_exchange(
                PUBLICATION_WAITING,
                PUBLICATION_STARTED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// True when publication has not started and can no longer start.
    fn abandon_before_publication(&self) -> bool {
        matches!(
            self.0.compare_exchange(
                PUBLICATION_WAITING,
                PUBLICATION_ABANDONED,
                Ordering::AcqRel,
                Ordering::Acquire,
            ),
            Ok(_) | Err(PUBLICATION_ABANDONED)
        )
    }
}

/// One paste transaction, shared by the paste caller and the window that
/// owns the clipboard. The window procedure can run inside our own clipboard
/// calls (`EmptyClipboard` sends `WM_DESTROYCLIPBOARD`), so each field locks
/// on its own and no lock is held across a clipboard call.
struct Transaction {
    publication: PublicationGate,
    state: Mutex<TxState>,
    transcript: String,
    /// ClipboardHandling::CopyToClipboard settles by leaving the transcript
    /// as plain text. Otherwise the previous clipboard is put back.
    preserve_transcript: bool,
    /// The user's clipboard from before the paste. When it was over budget,
    /// settlement erases the transcript instead.
    previous: Mutex<Result<Vec<SavedFormat>, CaptureLimit>>,
    /// The owner window, as an integer so it can cross threads.
    window: AtomicIsize,
    settled: AtomicBool,
    /// Set once the window is gone and the clipboard settled.
    finished: (Mutex<bool>, Condvar),
}

impl Transaction {
    fn new(transcript: &str, preserve_transcript: bool) -> Self {
        Self {
            publication: PublicationGate::default(),
            state: Mutex::new(TxState::new()),
            transcript: transcript.to_string(),
            preserve_transcript,
            previous: Mutex::new(Ok(Vec::new())),
            window: AtomicIsize::new(0),
            settled: AtomicBool::new(false),
            finished: (Mutex::new(false), Condvar::new()),
        }
    }

    fn finish(&self) {
        let (finished, signal) = &self.finished;
        *finished
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        signal.notify_all();
    }

    fn wait_finished(&self, timeout: Duration) {
        let (finished, signal) = &self.finished;
        let guard = finished
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = signal.wait_timeout_while(guard, timeout, |finished| !*finished);
    }
}

/// The transaction preparing or holding the clipboard, if any. A new paste
/// settles it before publishing its own transcript (see `flush_pending`).
static PENDING: Mutex<Option<Arc<Transaction>>> = Mutex::new(None);

fn clear_pending(candidate: &Arc<Transaction>) {
    let mut pending = lock(&PENDING);
    if same_pending(pending.as_ref(), candidate) {
        *pending = None;
    }
}

fn is_current_pending(candidate: &Arc<Transaction>) -> bool {
    same_pending(lock(&PENDING).as_ref(), candidate)
}

/// Opens the clipboard for `window`, runs `body`, and closes it on every path.
fn with_clipboard<T>(window: HWND, body: impl FnOnce() -> T) -> Result<T, String> {
    let mut attempt = 0;
    // SAFETY: `window` is the live transaction window of this thread.
    while let Err(error) = unsafe { OpenClipboard(Some(window)) } {
        attempt += 1;
        if attempt >= OPEN_ATTEMPTS {
            return Err(format!("the clipboard stayed busy: {error}"));
        }
        std::thread::sleep(OPEN_RETRY_DELAY);
    }
    let result = body();
    // SAFETY: the clipboard was opened above by this thread.
    let _ = unsafe { CloseClipboard() };
    Ok(result)
}

/// Copies one format off the open clipboard.
fn read_format(format: u32) -> Option<Vec<u8>> {
    // SAFETY: the caller has the clipboard open; byte-copyable formats are
    // global memory, read only while locked.
    unsafe {
        let memory = HGLOBAL(GetClipboardData(format).ok()?.0);
        let size = GlobalSize(memory);
        let pointer = GlobalLock(memory) as *const u8;
        if size == 0 || pointer.is_null() {
            return None;
        }
        let data = std::slice::from_raw_parts(pointer, size).to_vec();
        let _ = GlobalUnlock(memory);
        Some(data)
    }
}

/// Puts one format on the open clipboard. The clipboard owns the memory once
/// `SetClipboardData` accepts it.
fn write_format(format: u32, data: &[u8]) -> bool {
    // SAFETY: the caller has the clipboard open for its window; the copy
    // stays within the allocation of `data.len()` bytes.
    unsafe {
        let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)) else {
            return false;
        };
        let pointer = GlobalLock(memory) as *mut u8;
        if pointer.is_null() {
            let _ = GlobalFree(Some(memory));
            return false;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), pointer, data.len());
        let _ = GlobalUnlock(memory);
        if SetClipboardData(format, Some(HANDLE(memory.0))).is_ok() {
            true
        } else {
            let _ = GlobalFree(Some(memory));
            false
        }
    }
}

/// Every format on the open clipboard, in the order it was placed there.
fn clipboard_formats() -> Vec<u32> {
    let mut formats = Vec::new();
    let mut format = 0;
    loop {
        // SAFETY: the caller has the clipboard open.
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            return formats;
        }
        formats.push(format);
    }
}

/// Replaces the open clipboard with the transcript promise and the
/// exclusion formats.
fn publish_promise() -> Result<(), String> {
    // SAFETY: the caller has the clipboard open for its window, which owns
    // it after EmptyClipboard.
    unsafe {
        EmptyClipboard().map_err(|e| format!("EmptyClipboard failed: {e}"))?;
        for (name, value) in EXCLUSION_FORMATS {
            let format = RegisterClipboardFormatW(name);
            if format != 0 {
                write_format(format, &value.to_le_bytes());
            }
        }
        // A NULL handle is delayed rendering. SetClipboardData returns the
        // handle it was given, so success is also NULL and the windows crate
        // reports it as an error carrying GetLastError: only a nonzero code,
        // cleared beforehand, is a real failure.
        SetLastError(ERROR_SUCCESS);
        if let Err(e) = SetClipboardData(u32::from(CF_UNICODETEXT.0), None) {
            if e.code().is_err() {
                return Err(format!("SetClipboardData failed: {e}"));
            }
        }
        Ok(())
    }
}

/// Saves the previous clipboard and publishes the transcript in one opening
/// of the clipboard, so no other app can write in between.
fn capture_and_publish(window: HWND, transaction: &Transaction) -> Result<(), String> {
    with_clipboard(window, || {
        let previous = if transaction.preserve_transcript {
            Ok(Vec::new())
        } else {
            capture_with(clipboard_formats(), CAPTURE_BUDGET, read_format)
        };
        publish_captured(transaction, previous, publish_promise, |format| {
            write_format(format.format, &format.data);
        })
    })?
}

/// Runs after the potentially uninterruptible capture, immediately before the
/// first clipboard mutation. The atomic gate needs no lock across clipboard
/// calls, which can re-enter the window procedure.
fn publish_captured(
    transaction: &Transaction,
    previous: Result<Vec<SavedFormat>, CaptureLimit>,
    publish: impl FnOnce() -> Result<(), String>,
    mut restore: impl FnMut(&SavedFormat),
) -> Result<(), String> {
    if !transaction.publication.begin() {
        return Err("the clipboard transaction was abandoned before publication".to_string());
    }
    if let Err(limit) = &previous {
        info!("[reliable-paste] previous clipboard not saved ({limit:?} budget exceeded); it will be erased after the paste");
    }
    let published = publish();
    if published.is_err() {
        // EmptyClipboard may have run: put back what was saved.
        if let Ok(saved) = &previous {
            for format in saved {
                restore(format);
            }
        }
    }
    *lock(&transaction.previous) = previous;
    published
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Settles a transaction exactly once by keeping the transcript or putting
/// the previous clipboard back. Nothing is written once someone else owns
/// the clipboard. Runs on the transaction's thread.
fn settle(window: HWND, transaction: &Transaction) {
    if transaction.settled.swap(true, Ordering::SeqCst) {
        return;
    }
    let (ownership_lost, injection_failed) = {
        let state = lock(&transaction.state);
        (state.ownership_lost, state.injection_failed)
    };
    // If the chord never reached the target, keeping the transcript is the
    // only non-destructive outcome.
    let decision = decide_settlement(
        !ownership_lost && owns_clipboard(window),
        transaction.preserve_transcript || injection_failed,
    );
    if decision == ClipboardSettlement::LeaveUntouched {
        info!("[reliable-paste] clipboard changed externally; leaving it untouched");
        return;
    }
    let outcome = with_clipboard(window, || {
        // The clipboard is now open by this thread, so nothing can change
        // between this check and the writes below.
        if lock(&transaction.state).ownership_lost || !owns_clipboard(window) {
            return "clipboard changed before settlement; leaving it untouched";
        }
        // SAFETY: the clipboard is open for this thread's window.
        let _ = unsafe { EmptyClipboard() };
        if decision == ClipboardSettlement::KeepTranscript {
            write_format(
                u32::from(CF_UNICODETEXT.0),
                &unicode_text_bytes(&transaction.transcript),
            );
            return "left the transcript on the clipboard";
        }
        match &*lock(&transaction.previous) {
            Ok(saved) => {
                for format in saved {
                    write_format(format.format, &format.data);
                }
                "restored the previous clipboard"
            }
            Err(_) => "erased the transcript; the previous clipboard was not saved",
        }
    });
    match outcome {
        Ok(outcome) => info!("[reliable-paste] {outcome}"),
        Err(e) => error!("[reliable-paste] could not settle the clipboard: {e}"),
    }
}

/// Whether `window` still owns the clipboard. Writing to the clipboard takes
/// `EmptyClipboard`, which makes the writer the owner and sends the previous
/// owner `WM_DESTROYCLIPBOARD`; a reader leaves the owner in place. (The
/// sequence number is no guide: rendering for a reader changes it too.)
fn owns_clipboard(window: HWND) -> bool {
    // SAFETY: plain query.
    unsafe { GetClipboardOwner() }.is_ok_and(|owner| owner == window)
}

/// The transcript, rendered for whoever asked: the system has the clipboard
/// open on the reader's behalf.
fn render_on_request(transaction: &Transaction) {
    lock(&transaction.state).record_receipt(Instant::now());
    write_format(
        u32::from(CF_UNICODETEXT.0),
        &unicode_text_bytes(&transaction.transcript),
    );
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the pointer is set right after the window is created and the
    // transaction outlives the window (see `transaction_thread`).
    let transaction = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const Transaction;
    let Some(transaction) = (unsafe { transaction.as_ref() }) else {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    };
    match message {
        WM_RENDERFORMAT => {
            if wparam.0 == usize::from(CF_UNICODETEXT.0) {
                render_on_request(transaction);
            }
            LRESULT(0)
        }
        // Settlement already replaced the promise with real data, which
        // must not be overwritten when the window goes.
        WM_RENDERALLFORMATS if transaction.settled.load(Ordering::SeqCst) => LRESULT(0),
        WM_RENDERALLFORMATS => {
            // The window is being destroyed with the promise still up, which
            // is not a read. The system does not open the clipboard here.
            let _ = with_clipboard(window, || {
                // SAFETY: plain query on the clipboard opened above.
                if unsafe { GetClipboardOwner() }.is_ok_and(|owner| owner == window) {
                    write_format(
                        u32::from(CF_UNICODETEXT.0),
                        &unicode_text_bytes(&transaction.transcript),
                    );
                }
            });
            LRESULT(0)
        }
        WM_DESTROYCLIPBOARD => {
            lock(&transaction.state).ownership_lost = true;
            LRESULT(0)
        }
        WM_TIMER => {
            if matches!(
                evaluate(&lock(&transaction.state), Instant::now()),
                WaitDecision::Finish
            ) {
                settle(window, transaction);
                unsafe { PostQuitMessage(0) };
            }
            LRESULT(0)
        }
        WM_SETTLE_NOW => {
            settle(window, transaction);
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn create_window() -> Result<HWND, String> {
    // SAFETY: registers this module's window class once and creates a
    // message-only window of it.
    unsafe {
        let module =
            GetModuleHandleW(PCWSTR::null()).map_err(|e| format!("GetModuleHandle failed: {e}"))?;
        let instance = HINSTANCE(module.0);
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| {
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            RegisterClassW(&class);
        });
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS_NAME,
            w!("Murmur paste"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance),
            None,
        )
        .map_err(|e| format!("CreateWindowEx failed: {e}"))
    }
}

/// Owns the clipboard for one transaction: publishes, then pumps messages
/// until the transaction settles.
fn transaction_thread(transaction: Arc<Transaction>, ready: mpsc::Sender<Result<(), String>>) {
    let window = match create_window() {
        Ok(window) => window,
        Err(e) => {
            let _ = ready.send(Err(e));
            transaction.finish();
            clear_pending(&transaction);
            return;
        }
    };
    let raw = Arc::into_raw(transaction.clone());
    // SAFETY: the window procedure reads this pointer until the window is
    // destroyed; the reference is released only after DestroyWindow.
    unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, raw as isize) };
    transaction
        .window
        .store(window.0 as isize, Ordering::SeqCst);

    match capture_and_publish(window, &transaction) {
        Ok(()) => {
            lock(&transaction.state).published_at = Instant::now();
            // SAFETY: the timer belongs to this thread's window.
            unsafe { SetTimer(Some(window), TIMER_ID, TIMER_INTERVAL_MS, None) };
            if ready.send(Ok(())).is_err() {
                // The caller did not receive a publication receipt, so no
                // paste chord was sent. Never restore over its recovery text.
                lock(&transaction.state).injection_failed = true;
            }
            let mut message = MSG::default();
            // SAFETY: a standard message loop for this thread's window.
            while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
                unsafe { DispatchMessageW(&message) };
            }
            let _ = unsafe { KillTimer(Some(window), TIMER_ID) };
        }
        Err(e) => {
            let _ = ready.send(Err(e));
        }
    }

    // SAFETY: the window is this thread's; `raw` came from Arc::into_raw
    // above and is released exactly once, after the last window message.
    unsafe {
        let _ = DestroyWindow(window);
        drop(Arc::from_raw(raw));
    }
    transaction.finish();
    clear_pending(&transaction);
}

enum PublicationReceipt {
    Published(Arc<Transaction>),
    /// Publication won the timeout race, but was not acknowledged in time.
    /// It is tracked and kept for recovery; never inject a chord for it.
    Retained,
}

fn wait_for_publication(
    transaction: Arc<Transaction>,
    published: mpsc::Receiver<Result<(), String>>,
    wait: Duration,
) -> Result<PublicationReceipt, String> {
    match published.recv_timeout(wait) {
        Ok(Ok(())) => Ok(PublicationReceipt::Published(transaction)),
        Ok(Err(e)) => Err(e),
        Err(_) if transaction.publication.abandon_before_publication() => {
            Err("the clipboard transaction did not publish in time".to_string())
        }
        Err(_) => {
            // The native mutation already won. Returning an ordinary error
            // would orphan it, and injecting now could paste the old value.
            // Keep it tracked and mark it non-injected before returning.
            lock(&transaction.state).injection_failed = true;
            Ok(PublicationReceipt::Retained)
        }
    }
}

/// Starts a tracked transaction and waits until its transcript is published.
fn publish(text: &str, preserve_transcript: bool) -> Result<PublicationReceipt, String> {
    let transaction = Arc::new(Transaction::new(text, preserve_transcript));
    let (ready, published) = mpsc::channel();
    let for_thread = transaction.clone();
    *lock(&PENDING) = Some(transaction.clone());
    if let Err(e) = std::thread::Builder::new()
        .name("paste-transaction".into())
        .spawn(move || transaction_thread(for_thread, ready))
    {
        clear_pending(&transaction);
        return Err(format!("could not start the clipboard transaction: {e}"));
    }
    wait_for_publication(transaction, published, PUBLISH_WAIT)
}

/// Asks a transaction to settle now and waits, bounded, until it has.
fn settle_now(transaction: &Transaction) {
    // A flush or shutdown can arrive before the owner window exists. Stop
    // that worker from publishing after the caller has moved on.
    transaction.publication.abandon_before_publication();
    let window = HWND(transaction.window.load(Ordering::SeqCst) as *mut c_void);
    if window.is_invalid() {
        return;
    }
    // A failed post means the window is gone: the transaction settled.
    // SAFETY: posting to a window handle that may already be destroyed is
    // harmless; Windows rejects it.
    if unsafe { PostMessageW(Some(window), WM_SETTLE_NOW, WPARAM(0), LPARAM(0)) }.is_ok() {
        transaction.wait_finished(SETTLE_WAIT);
    }
}

/// If a previous transaction is still holding the clipboard, settle it before
/// publishing a new transcript or before the app exits.
pub(super) fn flush_pending() {
    let previous = lock(&PENDING).take();
    if let Some(previous) = previous {
        settle_now(&previous);
    }
}

/// Initial reliable-paste receipt. Its asynchronous method covers only the
/// cancellable auto-submit delay; clipboard settlement stays independent.
pub(crate) struct ReliablePasteReceipt {
    pending: Arc<Transaction>,
    operation: ProcessingOperation,
    release: PasteReleaseReceipt,
    auto_submit: Option<(AutoSubmitKey, FrontmostTarget)>,
}

impl ReliablePasteReceipt {
    /// Keeps the pipeline cancellable until the delayed auto-submit has either
    /// run or been suppressed. It intentionally does not wait for settlement.
    pub(crate) async fn complete_auto_submit(self, app_handle: AppHandle) -> Result<(), String> {
        self.release.wait().await?;
        let Some((auto_submit_key, target)) = self.auto_submit else {
            return Ok(());
        };
        if self.operation.cancellation_requested() {
            return Ok(());
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            _ = self.operation.cancelled() => return Ok(()),
        }
        if self.operation.cancellation_requested() {
            return Ok(());
        }

        let (sender, mut receiver) = oneshot::channel();
        let pending = self.pending.clone();
        let operation = self.operation.clone();
        app_handle
            .run_on_main_thread(move || {
                if !operation.cancellation_requested() {
                    // A newer paste may have replaced this transaction while
                    // the delay elapsed. Focus identity alone cannot detect
                    // that when both pastes target the same window.
                    if !is_current_pending(&pending) {
                        info!("[reliable-paste] skipping auto-submit for superseded transaction");
                    } else if let Err(error) =
                        send_return_if_target_unchanged(auto_submit_key, Some(target)).map(|_| ())
                    {
                        error!("[reliable-paste] paste succeeded, but auto-submit failed: {error}");
                    }
                }
                let _ = sender.send(());
            })
            .map_err(|error| format!("[reliable-paste] failed to queue auto-submit: {error}"))?;

        tokio::select! {
            result = &mut receiver => result
                .map_err(|_| "[reliable-paste] queued auto-submit was dropped".to_string()),
            _ = self.operation.cancelled() => Ok(()),
        }
    }
}

pub(super) fn run(
    text: &str,
    app_handle: &AppHandle,
    auto_submit: bool,
    auto_submit_key: AutoSubmitKey,
    clipboard_handling: ClipboardHandling,
    operation: ProcessingOperation,
) -> ReliablePasteOutcome {
    let Some(target) = frontmost_target() else {
        return ReliablePasteOutcome::Skipped;
    };

    match claim_before_pasteboard_mutation(
        &operation,
        || frontmost_target().as_ref() == Some(&target),
        flush_pending,
    ) {
        PasteboardAdmission::Claimed => {}
        PasteboardAdmission::Cancelled => return ReliablePasteOutcome::Cancelled,
        PasteboardAdmission::TargetChanged => return ReliablePasteOutcome::Skipped,
    }

    // After flush_pending, so an earlier Murmur paste has already put the
    // user's own clipboard back and that is what gets saved.
    let transaction = match publish(
        text,
        clipboard_handling == ClipboardHandling::CopyToClipboard,
    ) {
        Ok(PublicationReceipt::Published(transaction)) => transaction,
        Ok(PublicationReceipt::Retained) => {
            info!("[reliable-paste] late publication retained without a paste chord");
            return ReliablePasteOutcome::Skipped;
        }
        Err(e) => {
            error!("[reliable-paste] {e}");
            return ReliablePasteOutcome::Skipped;
        }
    };
    info!("[reliable-paste] published transcript as delayed render");

    // The target may read while Control is still held.
    lock(&transaction.state).injected_at = Some(Instant::now());
    let release = match send_paste_if_target_unchanged(Some(target.clone()), app_handle) {
        Ok(Some(receipt)) => {
            info!("[reliable-paste] Ctrl+V sent");
            Some(receipt)
        }
        Ok(None) => {
            lock(&transaction.state).injection_failed = true;
            info!("[reliable-paste] target changed before Ctrl+V; paste skipped");
            None
        }
        Err(e) => {
            // The transaction keeps the transcript on the clipboard: a failed
            // chord never authorizes restoring the previous clipboard.
            lock(&transaction.state).injection_failed = true;
            error!("[reliable-paste] failed to send paste chord: {e}");
            None
        }
    };

    match release {
        Some(release) => ReliablePasteOutcome::Injected(ReliablePasteReceipt {
            pending: transaction,
            operation,
            release,
            auto_submit: auto_submit.then_some((auto_submit_key, target)),
        }),
        None => ReliablePasteOutcome::Skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_finishing_after_timeout_never_publishes_over_a_new_copy() {
        let transaction = Arc::new(Transaction::new("old transcript", false));
        let clipboard = Arc::new(Mutex::new("original copy".to_string()));
        let (capturing, captured) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        let worker_transaction = transaction.clone();
        let worker_clipboard = clipboard.clone();
        let worker = std::thread::spawn(move || {
            // The native renderer can block at this point with the clipboard
            // open. No timer or sleep is needed to force that interleaving.
            capturing.send(()).unwrap();
            resumed.recv().unwrap();
            publish_captured(
                &worker_transaction,
                Ok(Vec::new()),
                || {
                    *lock(&worker_clipboard) = "old transcript".to_string();
                    Ok(())
                },
                |_| panic!("an abandoned capture must not restore anything"),
            )
        });
        captured.recv().unwrap();
        let (_ready, published) = mpsc::channel();
        assert!(wait_for_publication(transaction, published, Duration::ZERO).is_err());
        *lock(&clipboard) = "new user copy".to_string();
        resume.send(()).unwrap();
        assert!(worker.join().unwrap().is_err());
        assert_eq!(*lock(&clipboard), "new user copy");
    }

    #[test]
    fn publication_winning_the_timeout_is_retained_without_injection() {
        let transaction = Arc::new(Transaction::new("transcript", false));
        let (publishing, started) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        let (ready, published) = mpsc::channel();
        let worker_transaction = transaction.clone();
        let worker = std::thread::spawn(move || {
            let result = publish_captured(
                &worker_transaction,
                Ok(Vec::new()),
                || {
                    publishing.send(()).unwrap();
                    resumed.recv().unwrap();
                    Ok(())
                },
                |_| panic!("publication succeeded"),
            );
            assert!(result.is_ok());
            assert!(ready.send(Ok(())).is_err(), "the caller has moved on");
        });
        started.recv().unwrap();
        assert!(matches!(
            wait_for_publication(transaction.clone(), published, Duration::ZERO),
            Ok(PublicationReceipt::Retained)
        ));
        assert!(lock(&transaction.state).injection_failed);
        assert!(lock(&transaction.state).injected_at.is_none());
        resume.send(()).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn an_acknowledged_publication_still_allows_the_normal_paste() {
        let transaction = Arc::new(Transaction::new("transcript", false));
        publish_captured(&transaction, Ok(Vec::new()), || Ok(()), |_| {}).unwrap();
        let (ready, published) = mpsc::channel();
        ready.send(Ok(())).unwrap();
        let received = wait_for_publication(transaction.clone(), published, Duration::ZERO);
        let Ok(PublicationReceipt::Published(received)) = received else {
            panic!("the published transcript must be acknowledged");
        };
        assert!(Arc::ptr_eq(&received, &transaction));
        assert!(!lock(&transaction.state).injection_failed);
    }

    #[test]
    fn settling_before_the_window_exists_prevents_later_publication() {
        let transaction = Transaction::new("transcript", false);
        settle_now(&transaction);
        assert!(publish_captured(
            &transaction,
            Ok(Vec::new()),
            || panic!("a settled pending transaction must not publish"),
            |_| panic!("a settled pending transaction must not restore"),
        )
        .is_err());
    }

    #[test]
    fn a_native_publication_error_restores_the_captured_copy() {
        let transaction = Transaction::new("transcript", false);
        let clipboard = Arc::new(Mutex::new("original copy".to_string()));
        let to_clear = clipboard.clone();
        let to_restore = clipboard.clone();
        let result = publish_captured(
            &transaction,
            Ok(vec![SavedFormat {
                format: 13,
                data: b"original copy".to_vec(),
            }]),
            || {
                *lock(&to_clear) = String::new();
                Err("publication refused".to_string())
            },
            |format| {
                *lock(&to_restore) = String::from_utf8(format.data.clone()).unwrap();
            },
        );
        assert!(result.is_err());
        assert_eq!(*lock(&clipboard), "original copy");
    }

    #[test]
    fn only_formats_held_in_plain_memory_are_saved() {
        let registered_html = 0xC0F3;
        for format in [
            u32::from(CF_UNICODETEXT.0),
            8,  /* CF_DIB */
            15, /* CF_HDROP */
            registered_html,
        ] {
            assert!(is_byte_copyable(format), "{format:#x}");
        }
        for format in [
            u32::from(CF_BITMAP.0),
            u32::from(CF_ENHMETAFILE.0),
            u32::from(CF_METAFILEPICT.0),
            0x0200,
            0x02FF,
            0x0300,
            0x03FF,
        ] {
            assert!(!is_byte_copyable(format), "{format:#x}");
        }
    }

    #[test]
    fn the_byte_budget_counts_every_format_and_skips_handles() {
        let budget = CaptureBudget {
            max_bytes: 10,
            ..CAPTURE_BUDGET
        };
        let formats = vec![13, u32::from(CF_BITMAP.0), 8];
        let mut read_handles = false;
        let fits = capture_with(formats.clone(), budget, |format| {
            read_handles |= format == u32::from(CF_BITMAP.0);
            Some(vec![0; 5])
        });
        assert_eq!(fits.map(|saved| saved.len()), Ok(2));
        assert!(!read_handles);
        let over = capture_with(formats, budget, |_| Some(vec![0; 6]));
        assert_eq!(over, Err(CaptureLimit::Bytes));
    }

    #[test]
    fn an_exhausted_time_budget_reads_nothing() {
        let budget = CaptureBudget {
            max_duration: Duration::ZERO,
            ..CAPTURE_BUDGET
        };
        let mut reads = 0;
        let captured = capture_with(vec![13], budget, |_| {
            reads += 1;
            Some(b"text".to_vec())
        });
        assert_eq!(captured, Err(CaptureLimit::Time));
        assert_eq!(reads, 0);
    }

    #[test]
    fn unicode_text_is_nul_terminated_utf16() {
        assert_eq!(unicode_text_bytes("é"), vec![0xE9, 0x00, 0x00, 0x00]);
        assert_eq!(unicode_text_bytes(""), vec![0x00, 0x00]);
    }

    /// Reads the clipboard text the way a paste target does, which makes
    /// Windows ask the owner to render a delayed format.
    fn read_clipboard_text() -> Option<String> {
        // SAFETY: reading needs no owner window; the clipboard is closed on
        // every path once opened.
        unsafe {
            OpenClipboard(None).ok()?;
            let text = read_format(u32::from(CF_UNICODETEXT.0)).map(|bytes| {
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .take_while(|unit| *unit != 0)
                    .collect();
                String::from_utf16_lossy(&units)
            });
            let _ = CloseClipboard();
            text
        }
    }

    /// The whole Windows transaction on the real clipboard: the transcript
    /// is rendered on read, then the user's clipboard comes back.
    #[test]
    fn a_paste_renders_the_transcript_then_restores_the_previous_clipboard() {
        let PublicationReceipt::Published(previous) =
            publish("copied by the user", true).expect("publish the first text")
        else {
            panic!("the first publication was not acknowledged");
        };
        settle_now(&previous);
        assert_eq!(read_clipboard_text().as_deref(), Some("copied by the user"));

        let PublicationReceipt::Published(transaction) =
            publish("dictated transcript", false).expect("publish the transcript")
        else {
            panic!("the transcript publication was not acknowledged");
        };
        assert!(lock(&transaction.previous)
            .as_ref()
            .is_ok_and(|saved| !saved.is_empty()));
        assert_eq!(
            read_clipboard_text().as_deref(),
            Some("dictated transcript")
        );
        // Reading leaves the clipboard Murmur's.
        let window = HWND(transaction.window.load(Ordering::SeqCst) as *mut c_void);
        assert!(!lock(&transaction.state).ownership_lost);
        assert!(owns_clipboard(window));
        settle_now(&transaction);
        assert!(*lock(&transaction.finished.0));
        assert_eq!(read_clipboard_text().as_deref(), Some("copied by the user"));
    }
}
