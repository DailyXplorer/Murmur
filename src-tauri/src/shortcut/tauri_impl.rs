//! Tauri global-shortcut implementation
//!
//! This module provides shortcut functionality using Tauri's built-in
//! global-shortcut plugin.

use log::{debug, error, warn};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut, ShortcutState};

use crate::settings::{self, get_settings, AppSettings, ShortcutBinding};

use super::handler::handle_shortcut_event;
#[cfg(target_os = "windows")]
use super::modifier_chord::{self, Chord};

// Registration happens on the async runtime because global-shortcut callbacks
// must not register or unregister themselves. The epoch and mutex ensure an
// older queued operation cannot re-register Cancel after a newer one removed
// it, or remove it after a new recording started.
static CANCEL_SHORTCUT_EPOCH: AtomicU64 = AtomicU64::new(0);
static CANCEL_SHORTCUT_OPERATION_LOCK: Mutex<()> = Mutex::new(());
/// Under `CANCEL_SHORTCUT_OPERATION_LOCK`, this is the exact set of keys
/// owned by successful Cancel registrations. A global registration alone does
/// not prove Cancel owns it: it may belong to Transcribe or another feature.
/// Rebind reconciliation keeps every surviving owned key here; FinishGuard
/// drains this set instead of re-reading settings.
static CANCEL_SHORTCUT_BINDINGS: Mutex<Vec<ShortcutBinding>> = Mutex::new(Vec::new());

fn is_tracked_cancel_binding(bindings: &[ShortcutBinding], binding: &ShortcutBinding) -> bool {
    bindings
        .iter()
        .any(|tracked| tracked.current_binding == binding.current_binding)
}

fn track_cancel_binding(bindings: &mut Vec<ShortcutBinding>, binding: ShortcutBinding) {
    if !is_tracked_cancel_binding(bindings, &binding) {
        bindings.push(binding);
    }
}

fn untrack_cancel_binding(bindings: &mut Vec<ShortcutBinding>, binding: &ShortcutBinding) {
    bindings.retain(|tracked| tracked.current_binding != binding.current_binding);
}

/// Small adapter around Tauri's global shortcut manager. Keeping this private
/// lets the cancel lifecycle be tested without registering real macOS keys.
trait CancelShortcutRegistry {
    fn is_registered(&self, binding: &ShortcutBinding) -> bool;
    fn register(&self, binding: ShortcutBinding) -> Result<(), String>;
    fn unregister(&self, binding: ShortcutBinding) -> Result<(), String>;
}

struct TauriCancelShortcutRegistry<'a> {
    app: &'a AppHandle,
}

impl CancelShortcutRegistry for TauriCancelShortcutRegistry<'_> {
    fn is_registered(&self, binding: &ShortcutBinding) -> bool {
        #[cfg(target_os = "windows")]
        if let Some(chord) = Chord::parse(&binding.current_binding) {
            return modifier_chord::is_registered(chord);
        }
        binding
            .current_binding
            .parse::<Shortcut>()
            .is_ok_and(|shortcut| self.app.global_shortcut().is_registered(shortcut))
    }

    fn register(&self, binding: ShortcutBinding) -> Result<(), String> {
        register_shortcut(self.app, binding)
    }

    fn unregister(&self, binding: ShortcutBinding) -> Result<(), String> {
        unregister_shortcut(self.app, binding)
    }
}

#[derive(Clone, Copy)]
enum CancelShortcutLifecycleOperation {
    Register,
    Unregister,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CancelShortcutSettingsState {
    Previous,
    Updated,
}

trait CancelShortcutPersistence {
    fn persist(&self, state: CancelShortcutSettingsState) -> Result<(), String>;
    fn current_state(&self) -> CancelShortcutSettingsState;
}

struct TauriCancelShortcutPersistence<'a> {
    app: &'a AppHandle,
    previous_binding: &'a ShortcutBinding,
    updated_binding: &'a ShortcutBinding,
    current_state: Cell<CancelShortcutSettingsState>,
}

impl CancelShortcutPersistence for TauriCancelShortcutPersistence<'_> {
    fn persist(&self, state: CancelShortcutSettingsState) -> Result<(), String> {
        let binding = match state {
            CancelShortcutSettingsState::Previous => self.previous_binding,
            CancelShortcutSettingsState::Updated => self.updated_binding,
        };
        settings::update_settings_checked(self.app, |settings| {
            replace_cancel_binding(settings, binding);
            Ok(())
        })?;
        self.current_state.set(state);
        Ok(())
    }

    fn current_state(&self) -> CancelShortcutSettingsState {
        self.current_state.get()
    }
}

/// Cancel persistence, including rollback, owns only its one binding. Other
/// preferences may have changed while macOS was registering the shortcut.
fn replace_cancel_binding(settings: &mut AppSettings, binding: &ShortcutBinding) {
    settings
        .bindings
        .insert("cancel".to_string(), binding.clone());
}

fn apply_cancel_shortcut_lifecycle_operation<R: CancelShortcutRegistry>(
    registry: &R,
    bindings: &mut Vec<ShortcutBinding>,
    current_epoch: u64,
    request_epoch: u64,
    operation: CancelShortcutLifecycleOperation,
    binding: Option<ShortcutBinding>,
) -> Result<(), String> {
    if current_epoch != request_epoch {
        return Ok(());
    }

    match operation {
        CancelShortcutLifecycleOperation::Register => {
            let binding =
                binding.ok_or_else(|| "Missing Cancel shortcut registration".to_string())?;
            if is_tracked_cancel_binding(bindings, &binding) && registry.is_registered(&binding) {
                return Ok(());
            }
            register_tracked_cancel_binding(registry, bindings, binding)
        }
        CancelShortcutLifecycleOperation::Unregister => {
            teardown_tracked_cancel_shortcuts(registry, bindings)
        }
    }
}

fn registered_cancel_state<R: CancelShortcutRegistry>(
    registry: &R,
    previous: &ShortcutBinding,
    updated: &ShortcutBinding,
) -> String {
    format!(
        "previous_registered={}, updated_registered={}",
        registry.is_registered(previous),
        registry.is_registered(updated)
    )
}

fn synchronize_tracked_cancel_binding<R: CancelShortcutRegistry>(
    registry: &R,
    bindings: &mut Vec<ShortcutBinding>,
    binding: ShortcutBinding,
) {
    if registry.is_registered(&binding) {
        track_cancel_binding(bindings, binding);
    } else {
        untrack_cancel_binding(bindings, &binding);
    }
}

fn register_tracked_cancel_binding<R: CancelShortcutRegistry>(
    registry: &R,
    bindings: &mut Vec<ShortcutBinding>,
    binding: ShortcutBinding,
) -> Result<(), String> {
    match registry.register(binding.clone()) {
        Ok(()) => {
            track_cancel_binding(bindings, binding);
            Ok(())
        }
        // A failed registration can be a collision with another shortcut. Do
        // not adopt a binding solely because it is globally registered.
        Err(error) => Err(error),
    }
}

fn unregister_tracked_cancel_binding<R: CancelShortcutRegistry>(
    registry: &R,
    bindings: &mut Vec<ShortcutBinding>,
    binding: ShortcutBinding,
) -> Result<(), String> {
    let result = registry.unregister(binding.clone());
    synchronize_tracked_cancel_binding(registry, bindings, binding);
    result
}

fn teardown_tracked_cancel_shortcuts<R: CancelShortcutRegistry>(
    registry: &R,
    bindings: &mut Vec<ShortcutBinding>,
) -> Result<(), String> {
    let bindings_to_remove = std::mem::take(bindings);
    let mut errors = Vec::new();

    for binding in bindings_to_remove {
        if let Err(error) = unregister_tracked_cancel_binding(registry, bindings, binding.clone()) {
            if registry.is_registered(&binding) {
                errors.push(error);
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" "))
    }
}

fn rollback_after_updated_settings_persist_failure<R: CancelShortcutRegistry>(
    registry: &R,
    bindings: &mut Vec<ShortcutBinding>,
    previous: &ShortcutBinding,
    updated: ShortcutBinding,
    persist_error: String,
) -> String {
    let unregister_error =
        unregister_tracked_cancel_binding(registry, bindings, updated.clone()).err();
    let state = registered_cancel_state(registry, previous, &updated);

    match unregister_error {
        Some(unregister_error) => format!(
            "Failed to persist the new Cancel shortcut: {persist_error}. Failed to remove the new Cancel shortcut during rollback: {unregister_error}. {state}"
        ),
        None => format!(
            "Failed to persist the new Cancel shortcut: {persist_error}. Rolled back the new Cancel shortcut. {state}"
        ),
    }
}

fn reconcile_failed_previous_cancel_unregistration<
    R: CancelShortcutRegistry,
    P: CancelShortcutPersistence,
>(
    registry: &R,
    persistence: &P,
    bindings: &mut Vec<ShortcutBinding>,
    previous: ShortcutBinding,
    updated: ShortcutBinding,
    cleanup_error: String,
) -> Result<(), String> {
    if !registry.is_registered(&previous) && registry.is_registered(&updated) {
        synchronize_tracked_cancel_binding(registry, bindings, previous);
        synchronize_tracked_cancel_binding(registry, bindings, updated);
        return Ok(());
    }

    let rollback_error = persistence
        .persist(CancelShortcutSettingsState::Previous)
        .err();
    if persistence.current_state() == CancelShortcutSettingsState::Updated {
        let retry_error =
            unregister_tracked_cancel_binding(registry, bindings, previous.clone()).err();
        synchronize_tracked_cancel_binding(registry, bindings, updated.clone());

        if !registry.is_registered(&previous) && registry.is_registered(&updated) {
            return Ok(());
        }

        let mut errors = vec![format!(
            "Failed to unregister the previous Cancel shortcut: {cleanup_error}"
        )];
        if let Some(error) = rollback_error {
            errors.push(format!("Failed to restore the previous settings: {error}"));
        }
        if let Some(error) = retry_error {
            errors.push(format!(
                "Failed to retry removal of the previous Cancel shortcut: {error}"
            ));
        }
        errors.push(format!(
            "Settings remain Updated. {}",
            registered_cancel_state(registry, &previous, &updated)
        ));
        return Err(errors.join(" "));
    }

    let mut errors = vec![format!(
        "Failed to unregister the previous Cancel shortcut: {cleanup_error}"
    )];
    if let Some(error) = rollback_error {
        errors.push(format!("Failed to restore the previous settings: {error}"));
    }

    if registry.is_registered(&updated) {
        if let Err(error) = unregister_tracked_cancel_binding(registry, bindings, updated.clone()) {
            errors.push(format!(
                "Failed to unregister the new Cancel shortcut during rollback: {error}"
            ));
        }
    } else {
        untrack_cancel_binding(bindings, &updated);
    }

    if !registry.is_registered(&previous) {
        match register_tracked_cancel_binding(registry, bindings, previous.clone()) {
            Ok(()) => {}
            Err(error) => errors.push(format!(
                "Failed to restore the previous Cancel shortcut: {error}"
            )),
        }
    } else {
        track_cancel_binding(bindings, previous.clone());
    }

    let previous_registered = registry.is_registered(&previous);
    let updated_registered = registry.is_registered(&updated);
    if previous_registered && !updated_registered {
        return Err(format!(
            "{}. The Cancel shortcut change was rolled back.",
            errors.join(" ")
        ));
    }

    errors.push(format!(
        "Rollback could not guarantee a single active Cancel shortcut: previous_registered={previous_registered}, updated_registered={updated_registered}"
    ));
    Err(errors.join(" "))
}

fn replace_active_cancel_shortcut<R: CancelShortcutRegistry, P: CancelShortcutPersistence>(
    registry: &R,
    persistence: &P,
    bindings: &mut Vec<ShortcutBinding>,
    previous: ShortcutBinding,
    updated: ShortcutBinding,
) -> Result<(), String> {
    if previous.current_binding == updated.current_binding {
        return Ok(());
    }

    if !is_tracked_cancel_binding(bindings, &previous) {
        if registry.is_registered(&updated) {
            return Err(format!(
                "Shortcut '{}' is already in use",
                updated.current_binding
            ));
        }
        return persistence.persist(CancelShortcutSettingsState::Updated);
    }

    register_tracked_cancel_binding(registry, bindings, updated.clone())?;

    if let Err(error) = persistence.persist(CancelShortcutSettingsState::Updated) {
        return Err(rollback_after_updated_settings_persist_failure(
            registry, bindings, &previous, updated, error,
        ));
    }

    match unregister_tracked_cancel_binding(registry, bindings, previous.clone()) {
        Ok(()) => Ok(()),
        Err(error) => reconcile_failed_previous_cancel_unregistration(
            registry,
            persistence,
            bindings,
            previous,
            updated,
            error,
        ),
    }
}

/// Update Cancel while preserving its dynamic lifecycle. Idle changes only
/// update settings; active changes atomically replace the registered shortcut.
pub fn change_cancel_binding(app: &AppHandle, binding: String) -> Result<ShortcutBinding, String> {
    let _operation = CANCEL_SHORTCUT_OPERATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let previous = get_settings(app)
        .bindings
        .get("cancel")
        .cloned()
        .or_else(|| {
            settings::get_default_settings()
                .bindings
                .get("cancel")
                .cloned()
        })
        .ok_or_else(|| "Binding 'cancel' does not exist".to_string())?;
    let mut updated = previous.clone();
    updated.current_binding = binding;

    let registry = TauriCancelShortcutRegistry { app };
    let persistence = TauriCancelShortcutPersistence {
        app,
        previous_binding: &previous,
        updated_binding: &updated,
        current_state: Cell::new(CancelShortcutSettingsState::Previous),
    };
    let mut tracked_bindings = CANCEL_SHORTCUT_BINDINGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    replace_active_cancel_shortcut(
        &registry,
        &persistence,
        &mut tracked_bindings,
        previous.clone(),
        updated.clone(),
    )?;

    Ok(updated)
}

/// Initialize shortcuts using Tauri's global-shortcut plugin. Returns the
/// bindings that could not be registered, keyed by binding id.
pub fn init_shortcuts(app: &AppHandle) -> Vec<(String, ShortcutBinding, String)> {
    let mut failures = Vec::new();
    let default_bindings = settings::get_default_settings().bindings;
    let user_settings = settings::load_or_create_app_settings(app);

    for (id, default_binding) in default_bindings {
        // Cancel is registered while recording; meetings only when enabled
        // and no other action uses their keys, so the order here is moot.
        if !super::binding_registers(&user_settings, &id) {
            continue;
        }
        let binding = user_settings
            .bindings
            .get(&id)
            .cloned()
            .unwrap_or(default_binding);

        if let Err(e) = register_shortcut(app, binding.clone()) {
            error!("Failed to register shortcut {} during init: {}", id, e);
            failures.push((id, binding, e));
        }
    }
    failures
}

/// Parses a shortcut string the way the global-shortcut plugin will when it
/// registers it, so a binding that could never register is refused before it
/// is saved. Friendlier messages cover the fn key and modifier-only input.
pub fn parse_shortcut(raw: &str) -> Result<Shortcut, String> {
    if raw.trim().is_empty() {
        return Err("Shortcut cannot be empty".into());
    }

    let modifiers = [
        "ctrl", "control", "shift", "alt", "option", "meta", "command", "cmd", "super", "win",
        "windows",
    ];

    // Check for fn key which Tauri doesn't support
    let parts: Vec<String> = raw.split('+').map(|p| p.trim().to_lowercase()).collect();
    for part in &parts {
        if part == "fn" || part == "function" {
            return Err("The 'fn' key is not supported by Tauri global shortcuts".into());
        }
    }

    let has_non_modifier = parts.iter().any(|part| !modifiers.contains(&part.as_str()));

    if !has_non_modifier {
        return Err("Tauri shortcuts must include a main key (letter, number, F-key, etc.) in addition to modifiers".into());
    }

    let shortcut = raw
        .parse::<Shortcut>()
        .map_err(|e| format!("Shortcut '{raw}' is not supported: {e}"))?;
    // macOS reports Caps Lock as a modifier-state change, never a key press,
    // so a hotkey on it registers but never fires.
    if shortcut.key == Code::CapsLock {
        return Err(
            "Caps Lock can't be used in a shortcut because macOS never reports it as a key press. Choose another key."
                .into(),
        );
    }
    Ok(shortcut)
}

/// Checks that `raw` can register: a global shortcut, or on Windows a chord
/// of modifiers alone (see `modifier_chord`).
pub fn validate_binding(raw: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    if Chord::parse(raw).is_some() {
        return Ok(());
    }
    parse_shortcut(raw).map(|_| ())
}

/// The other binding whose keys are the same shortcut as `raw`. Shortcuts are
/// compared parsed, so spellings such as "esc" and "escape" collide. While
/// the user edits a shortcut every other one is suspended, so registration
/// alone cannot catch this. A binding that is not in use (the meeting
/// shortcut while meetings are off) conflicts with nothing.
pub fn conflicting_binding<'a>(
    settings: &'a settings::AppSettings,
    id: &str,
    raw: &str,
) -> Option<&'a ShortcutBinding> {
    #[cfg(target_os = "windows")]
    if let Some(chord) = Chord::parse(raw) {
        return settings.bindings.iter().find_map(|(other_id, other)| {
            (other_id != id
                && super::binding_active(settings, other_id)
                && Chord::parse(&other.current_binding) == Some(chord))
            .then_some(other)
        });
    }
    let shortcut = raw.parse::<Shortcut>().ok()?;
    settings.bindings.iter().find_map(|(other_id, other)| {
        (other_id != id
            && super::binding_active(settings, other_id)
            && other.current_binding.parse::<Shortcut>().ok() == Some(shortcut))
        .then_some(other)
    })
}

/// Register a shortcut using Tauri's global-shortcut plugin
pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    if let Some(chord) = Chord::parse(&binding.current_binding) {
        return modifier_chord::register(app, &binding.id, &binding.current_binding, chord);
    }
    let shortcut = parse_shortcut(&binding.current_binding).inspect_err(|e| {
        warn!("register_tauri_shortcut validation error: {e}");
    })?;

    // Prevent duplicate registrations that would silently shadow one another
    if app.global_shortcut().is_registered(shortcut) {
        let error_msg = format!("Shortcut '{}' is already in use", binding.current_binding);
        warn!("register_tauri_shortcut duplicate error: {}", error_msg);
        return Err(error_msg);
    }

    let binding_id_for_closure = binding.id.clone();

    app.global_shortcut()
        .on_shortcut(shortcut, move |app_handle, scut, event| {
            if scut == &shortcut {
                let shortcut_string = scut.into_string();
                let is_pressed = event.state == ShortcutState::Pressed;
                debug!(
                    "tauri global-shortcut event: binding={}, shortcut={}, state={:?}",
                    binding_id_for_closure, shortcut_string, event.state
                );
                handle_shortcut_event(
                    app_handle,
                    &binding_id_for_closure,
                    &shortcut_string,
                    is_pressed,
                );
            }
        })
        .map_err(|e| {
            let error_msg = format!(
                "Couldn't register shortcut '{}': {}",
                binding.current_binding, e
            );
            error!("register_tauri_shortcut registration error: {}", error_msg);
            error_msg
        })?;

    Ok(())
}

/// Unregister a shortcut from Tauri's global-shortcut plugin
pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    if let Some(chord) = Chord::parse(&binding.current_binding) {
        return modifier_chord::unregister(chord);
    }
    let shortcut = match binding.current_binding.parse::<Shortcut>() {
        Ok(s) => s,
        Err(e) => {
            let error_msg = format!(
                "Failed to parse shortcut '{}' for unregistration: {}",
                binding.current_binding, e
            );
            error!("unregister_tauri_shortcut parse error: {}", error_msg);
            return Err(error_msg);
        }
    };

    app.global_shortcut().unregister(shortcut).map_err(|e| {
        let error_msg = format!(
            "Failed to unregister shortcut '{}': {}",
            binding.current_binding, e
        );
        error!("unregister_tauri_shortcut error: {}", error_msg);
        error_msg
    })?;

    Ok(())
}

/// Register the cancel shortcut (called when recording starts)
pub fn register_cancel_shortcut(app: &AppHandle) {
    let app_clone = app.clone();
    let request_epoch = CANCEL_SHORTCUT_EPOCH.fetch_add(1, Ordering::AcqRel) + 1;
    tauri::async_runtime::spawn(async move {
        let _operation = CANCEL_SHORTCUT_OPERATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cancel_binding) = get_settings(&app_clone).bindings.get("cancel").cloned() {
            let registry = TauriCancelShortcutRegistry { app: &app_clone };
            let mut tracked_bindings = CANCEL_SHORTCUT_BINDINGS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Err(e) = apply_cancel_shortcut_lifecycle_operation(
                &registry,
                &mut tracked_bindings,
                CANCEL_SHORTCUT_EPOCH.load(Ordering::Acquire),
                request_epoch,
                CancelShortcutLifecycleOperation::Register,
                Some(cancel_binding),
            ) {
                error!("Failed to register cancel shortcut: {}", e);
            }
        }
    });
}

/// Unregister the cancel shortcut (called when the pipeline finishes or is cancelled)
pub fn unregister_cancel_shortcut(app: &AppHandle) {
    let app_clone = app.clone();
    let request_epoch = CANCEL_SHORTCUT_EPOCH.fetch_add(1, Ordering::AcqRel) + 1;
    tauri::async_runtime::spawn(async move {
        let _operation = CANCEL_SHORTCUT_OPERATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let registry = TauriCancelShortcutRegistry { app: &app_clone };
        let mut tracked_bindings = CANCEL_SHORTCUT_BINDINGS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Err(error) = apply_cancel_shortcut_lifecycle_operation(
            &registry,
            &mut tracked_bindings,
            CANCEL_SHORTCUT_EPOCH.load(Ordering::Acquire),
            request_epoch,
            CancelShortcutLifecycleOperation::Unregister,
            None,
        ) {
            error!("Failed to unregister Cancel shortcuts: {error}");
        }
    });
}

#[cfg(test)]
mod tests;
