//! Tauri global-shortcut implementation
//!
//! This module provides shortcut functionality using Tauri's built-in
//! global-shortcut plugin.

use log::{debug, error, warn};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::settings::{self, get_settings, AppSettings, ShortcutBinding};

use super::handler::handle_shortcut_event;

// Registration happens on the async runtime because global-shortcut callbacks
// must not register or unregister themselves. The epoch and mutex ensure an
// older queued operation cannot re-register Cancel after a newer one removed
// it, or remove it after a new recording started.
static CANCEL_SHORTCUT_EPOCH: AtomicU64 = AtomicU64::new(0);
static CANCEL_SHORTCUT_OPERATION_LOCK: Mutex<()> = Mutex::new(());
/// Under `CANCEL_SHORTCUT_OPERATION_LOCK`, this is the exact set of Cancel
/// keys that may still be registered. Rebind reconciliation samples the
/// registry after every fallible cleanup and keeps every surviving key here;
/// FinishGuard-driven teardown drains this set instead of re-reading settings.
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
    previous_settings: &'a AppSettings,
    updated_settings: &'a AppSettings,
    current_state: Cell<CancelShortcutSettingsState>,
}

impl CancelShortcutPersistence for TauriCancelShortcutPersistence<'_> {
    fn persist(&self, state: CancelShortcutSettingsState) -> Result<(), String> {
        let settings = match state {
            CancelShortcutSettingsState::Previous => self.previous_settings,
            CancelShortcutSettingsState::Updated => self.updated_settings,
        };
        settings::write_settings_checked(self.app, settings.clone())?;
        self.current_state.set(state);
        Ok(())
    }

    fn current_state(&self) -> CancelShortcutSettingsState {
        self.current_state.get()
    }
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
    let result = registry.register(binding.clone());
    synchronize_tracked_cancel_binding(registry, bindings, binding);
    result
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

    let previous_settings: AppSettings = get_settings(app);
    let previous = previous_settings
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

    let mut updated_settings = previous_settings.clone();
    updated_settings
        .bindings
        .insert("cancel".to_string(), updated.clone());

    let registry = TauriCancelShortcutRegistry { app };
    let persistence = TauriCancelShortcutPersistence {
        app,
        previous_settings: &previous_settings,
        updated_settings: &updated_settings,
        current_state: Cell::new(CancelShortcutSettingsState::Previous),
    };
    let mut tracked_bindings = CANCEL_SHORTCUT_BINDINGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    replace_active_cancel_shortcut(
        &registry,
        &persistence,
        &mut tracked_bindings,
        previous,
        updated.clone(),
    )?;

    Ok(updated)
}

/// Initialize shortcuts using Tauri's global-shortcut plugin
pub fn init_shortcuts(app: &AppHandle) {
    let default_bindings = settings::get_default_settings().bindings;
    let user_settings = settings::load_or_create_app_settings(app);

    // Register all default shortcuts, applying user customizations
    for (id, default_binding) in default_bindings {
        if id == "cancel" {
            continue; // Skip cancel shortcut, it will be registered dynamically
        }
        let binding = user_settings
            .bindings
            .get(&id)
            .cloned()
            .unwrap_or(default_binding);

        if let Err(e) = register_shortcut(app, binding) {
            error!("Failed to register shortcut {} during init: {}", id, e);
        }
    }
}

/// Validate a shortcut string for the Tauri global-shortcut implementation.
/// Tauri requires at least one non-modifier key and doesn't support the fn key.
pub fn validate_shortcut(raw: &str) -> Result<(), String> {
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

    // Check for at least one non-modifier key
    let has_non_modifier = parts.iter().any(|part| !modifiers.contains(&part.as_str()));

    if has_non_modifier {
        Ok(())
    } else {
        Err("Tauri shortcuts must include a main key (letter, number, F-key, etc.) in addition to modifiers".into())
    }
}

/// Register a shortcut using Tauri's global-shortcut plugin
pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    // Validate for Tauri requirements
    if let Err(e) = validate_shortcut(&binding.current_binding) {
        warn!(
            "register_tauri_shortcut validation error for binding '{}': {}",
            binding.current_binding, e
        );
        return Err(e);
    }

    // Parse shortcut and return error if it fails
    let shortcut = match binding.current_binding.parse::<Shortcut>() {
        Ok(s) => s,
        Err(e) => {
            let error_msg = format!(
                "Failed to parse shortcut '{}': {}",
                binding.current_binding, e
            );
            error!("register_tauri_shortcut parse error: {}", error_msg);
            return Err(error_msg);
        }
    };

    // Prevent duplicate registrations that would silently shadow one another
    if app.global_shortcut().is_registered(shortcut) {
        let error_msg = format!("Shortcut '{}' is already in use", binding.current_binding);
        warn!("register_tauri_shortcut duplicate error: {}", error_msg);
        return Err(error_msg);
    }

    // Clone binding.id for use in the closure
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

/// Unregister the cancel shortcut (called when recording stops)
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
mod tests {
    use super::{
        apply_cancel_shortcut_lifecycle_operation, replace_active_cancel_shortcut,
        CancelShortcutLifecycleOperation, CancelShortcutPersistence, CancelShortcutRegistry,
        CancelShortcutSettingsState,
    };
    use crate::settings::ShortcutBinding;
    use std::cell::RefCell;
    use std::collections::{BTreeSet, VecDeque};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RegistryOperation {
        Register,
        Unregister,
    }

    struct RegistryFailure {
        operation: RegistryOperation,
        binding: String,
        error: String,
    }

    #[derive(Default)]
    struct FakeCancelShortcutRegistry {
        active: RefCell<BTreeSet<String>>,
        failures: RefCell<VecDeque<RegistryFailure>>,
        operations: RefCell<Vec<String>>,
    }

    impl FakeCancelShortcutRegistry {
        fn activate(&self, binding: &ShortcutBinding) {
            self.active
                .borrow_mut()
                .insert(binding.current_binding.clone());
        }

        fn is_active(&self, binding: &ShortcutBinding) -> bool {
            self.active.borrow().contains(&binding.current_binding)
        }

        fn fail_next(&self, operation: RegistryOperation, binding: &ShortcutBinding, error: &str) {
            self.failures.borrow_mut().push_back(RegistryFailure {
                operation,
                binding: binding.current_binding.clone(),
                error: error.to_string(),
            });
        }

        fn take_failure(
            &self,
            operation: RegistryOperation,
            binding: &ShortcutBinding,
        ) -> Option<String> {
            let mut failures = self.failures.borrow_mut();
            let failure = failures.front()?;
            if failure.operation != operation || failure.binding != binding.current_binding {
                return None;
            }
            failures.pop_front().map(|failure| failure.error)
        }
    }

    impl CancelShortcutRegistry for FakeCancelShortcutRegistry {
        fn is_registered(&self, binding: &ShortcutBinding) -> bool {
            self.is_active(binding)
        }

        fn register(&self, binding: ShortcutBinding) -> Result<(), String> {
            self.operations
                .borrow_mut()
                .push(format!("register:{}", binding.current_binding));
            if let Some(error) = self.take_failure(RegistryOperation::Register, &binding) {
                return Err(error);
            }
            self.active.borrow_mut().insert(binding.current_binding);
            Ok(())
        }

        fn unregister(&self, binding: ShortcutBinding) -> Result<(), String> {
            self.operations
                .borrow_mut()
                .push(format!("unregister:{}", binding.current_binding));
            if let Some(error) = self.take_failure(RegistryOperation::Unregister, &binding) {
                return Err(error);
            }
            if self.active.borrow_mut().remove(&binding.current_binding) {
                Ok(())
            } else {
                Err(format!(
                    "Shortcut '{}' is not registered",
                    binding.current_binding
                ))
            }
        }
    }

    struct PersistenceFailure {
        state: CancelShortcutSettingsState,
        error: String,
    }

    struct FakeCancelShortcutPersistence {
        state: RefCell<CancelShortcutSettingsState>,
        failures: RefCell<VecDeque<PersistenceFailure>>,
        writes: RefCell<Vec<CancelShortcutSettingsState>>,
    }

    impl FakeCancelShortcutPersistence {
        fn new(state: CancelShortcutSettingsState) -> Self {
            Self {
                state: RefCell::new(state),
                failures: RefCell::new(VecDeque::new()),
                writes: RefCell::new(Vec::new()),
            }
        }

        fn fail_next_for(&self, state: CancelShortcutSettingsState, error: &str) {
            self.failures.borrow_mut().push_back(PersistenceFailure {
                state,
                error: error.to_string(),
            });
        }
    }

    impl CancelShortcutPersistence for FakeCancelShortcutPersistence {
        fn persist(&self, state: CancelShortcutSettingsState) -> Result<(), String> {
            self.writes.borrow_mut().push(state);
            let mut failures = self.failures.borrow_mut();
            if failures
                .front()
                .is_some_and(|failure| failure.state == state)
            {
                return Err(failures
                    .pop_front()
                    .expect("the checked failure remains queued")
                    .error);
            }
            drop(failures);
            *self.state.borrow_mut() = state;
            Ok(())
        }

        fn current_state(&self) -> CancelShortcutSettingsState {
            *self.state.borrow()
        }
    }

    fn cancel_binding(binding: &str) -> ShortcutBinding {
        ShortcutBinding {
            id: "cancel".to_string(),
            name: "Cancel".to_string(),
            description: "Cancels the current recording.".to_string(),
            default_binding: "escape".to_string(),
            current_binding: binding.to_string(),
        }
    }

    fn tracked_binding_names(bindings: &[ShortcutBinding]) -> Vec<&str> {
        bindings
            .iter()
            .map(|binding| binding.current_binding.as_str())
            .collect()
    }

    #[test]
    fn active_replace_then_finish_unregisters_the_new_cancel_shortcut() {
        let registry = FakeCancelShortcutRegistry::default();
        let persistence = FakeCancelShortcutPersistence::new(CancelShortcutSettingsState::Previous);
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        registry.activate(&previous);
        let mut bindings = vec![previous.clone()];

        replace_active_cancel_shortcut(
            &registry,
            &persistence,
            &mut bindings,
            previous.clone(),
            updated.clone(),
        )
        .unwrap();
        apply_cancel_shortcut_lifecycle_operation(
            &registry,
            &mut bindings,
            2,
            2,
            CancelShortcutLifecycleOperation::Unregister,
            None,
        )
        .unwrap();

        assert_eq!(
            *persistence.state.borrow(),
            CancelShortcutSettingsState::Updated
        );
        assert!(!registry.is_active(&previous));
        assert!(!registry.is_active(&updated));
        assert!(bindings.is_empty());
        assert_eq!(
            *registry.operations.borrow(),
            vec![
                "register:cmd+shift+c".to_string(),
                "unregister:escape".to_string(),
                "unregister:cmd+shift+c".to_string(),
            ]
        );
    }

    #[test]
    fn idle_replace_persists_without_registering_cancel() {
        let registry = FakeCancelShortcutRegistry::default();
        let persistence = FakeCancelShortcutPersistence::new(CancelShortcutSettingsState::Previous);
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        let mut bindings = Vec::new();

        replace_active_cancel_shortcut(
            &registry,
            &persistence,
            &mut bindings,
            previous,
            updated.clone(),
        )
        .unwrap();

        assert_eq!(
            *persistence.state.borrow(),
            CancelShortcutSettingsState::Updated
        );
        assert!(!registry.is_active(&updated));
        assert!(bindings.is_empty());
        assert!(registry.operations.borrow().is_empty());
        assert_eq!(
            *persistence.writes.borrow(),
            vec![CancelShortcutSettingsState::Updated]
        );
    }

    #[test]
    fn same_binding_is_a_no_op() {
        let registry = FakeCancelShortcutRegistry::default();
        let persistence = FakeCancelShortcutPersistence::new(CancelShortcutSettingsState::Previous);
        let previous = cancel_binding("escape");
        registry.activate(&previous);
        let mut bindings = vec![previous.clone()];

        replace_active_cancel_shortcut(
            &registry,
            &persistence,
            &mut bindings,
            previous.clone(),
            previous.clone(),
        )
        .unwrap();

        assert!(registry.is_active(&previous));
        assert_eq!(tracked_binding_names(&bindings), vec!["escape"]);
        assert!(registry.operations.borrow().is_empty());
        assert!(persistence.writes.borrow().is_empty());
    }

    #[test]
    fn new_registration_failure_leaves_the_previous_cancel_shortcut_active() {
        let registry = FakeCancelShortcutRegistry::default();
        let persistence = FakeCancelShortcutPersistence::new(CancelShortcutSettingsState::Previous);
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        registry.activate(&previous);
        let mut bindings = vec![previous.clone()];
        registry.fail_next(RegistryOperation::Register, &updated, "simulated conflict");

        let result = replace_active_cancel_shortcut(
            &registry,
            &persistence,
            &mut bindings,
            previous.clone(),
            updated.clone(),
        );

        assert!(result.is_err());
        assert!(registry.is_active(&previous));
        assert!(!registry.is_active(&updated));
        assert_eq!(tracked_binding_names(&bindings), vec!["escape"]);
        assert_eq!(
            *persistence.state.borrow(),
            CancelShortcutSettingsState::Previous
        );
        assert!(persistence.writes.borrow().is_empty());
        assert_eq!(
            *registry.operations.borrow(),
            vec!["register:cmd+shift+c".to_string()]
        );
    }

    #[test]
    fn persistence_failure_removes_the_new_shortcut_and_keeps_previous_settings() {
        let registry = FakeCancelShortcutRegistry::default();
        let persistence = FakeCancelShortcutPersistence::new(CancelShortcutSettingsState::Previous);
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        registry.activate(&previous);
        let mut bindings = vec![previous.clone()];
        persistence.fail_next_for(CancelShortcutSettingsState::Updated, "disk full");

        let result = replace_active_cancel_shortcut(
            &registry,
            &persistence,
            &mut bindings,
            previous.clone(),
            updated.clone(),
        );

        assert!(result.is_err());
        assert!(registry.is_active(&previous));
        assert!(!registry.is_active(&updated));
        assert_eq!(tracked_binding_names(&bindings), vec!["escape"]);
        assert_eq!(
            *persistence.state.borrow(),
            CancelShortcutSettingsState::Previous
        );
        assert_eq!(
            *registry.operations.borrow(),
            vec![
                "register:cmd+shift+c".to_string(),
                "unregister:cmd+shift+c".to_string(),
            ]
        );
    }

    #[test]
    fn failed_previous_cleanup_rolls_back_settings_and_new_shortcut() {
        let registry = FakeCancelShortcutRegistry::default();
        let persistence = FakeCancelShortcutPersistence::new(CancelShortcutSettingsState::Previous);
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        registry.activate(&previous);
        let mut bindings = vec![previous.clone()];
        registry.fail_next(
            RegistryOperation::Unregister,
            &previous,
            "simulated old cleanup failure",
        );

        let result = replace_active_cancel_shortcut(
            &registry,
            &persistence,
            &mut bindings,
            previous.clone(),
            updated.clone(),
        );

        assert!(result.is_err());
        assert!(registry.is_active(&previous));
        assert!(!registry.is_active(&updated));
        assert_eq!(tracked_binding_names(&bindings), vec!["escape"]);
        assert_eq!(
            *persistence.state.borrow(),
            CancelShortcutSettingsState::Previous
        );
        assert_eq!(
            *persistence.writes.borrow(),
            vec![
                CancelShortcutSettingsState::Updated,
                CancelShortcutSettingsState::Previous,
            ]
        );
    }

    #[test]
    fn failed_settings_rollback_keeps_updated_binding_until_previous_cleanup_retries() {
        let registry = FakeCancelShortcutRegistry::default();
        let persistence = FakeCancelShortcutPersistence::new(CancelShortcutSettingsState::Previous);
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        registry.activate(&previous);
        let mut bindings = vec![previous.clone()];
        persistence.fail_next_for(
            CancelShortcutSettingsState::Previous,
            "simulated previous settings rollback failure",
        );
        registry.fail_next(
            RegistryOperation::Unregister,
            &previous,
            "simulated old cleanup failure",
        );

        replace_active_cancel_shortcut(
            &registry,
            &persistence,
            &mut bindings,
            previous.clone(),
            updated.clone(),
        )
        .unwrap();

        assert_eq!(
            *persistence.state.borrow(),
            CancelShortcutSettingsState::Updated
        );
        assert!(!registry.is_active(&previous));
        assert!(registry.is_active(&updated));
        assert_eq!(tracked_binding_names(&bindings), vec!["cmd+shift+c"]);
        assert_eq!(
            *registry.operations.borrow(),
            vec![
                "register:cmd+shift+c".to_string(),
                "unregister:escape".to_string(),
                "unregister:escape".to_string(),
            ]
        );
    }

    #[test]
    fn failed_settings_rollback_and_second_cleanup_preserve_all_bindings_for_finish() {
        let registry = FakeCancelShortcutRegistry::default();
        let persistence = FakeCancelShortcutPersistence::new(CancelShortcutSettingsState::Previous);
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        registry.activate(&previous);
        let mut bindings = vec![previous.clone()];
        persistence.fail_next_for(
            CancelShortcutSettingsState::Previous,
            "simulated previous settings rollback failure",
        );
        registry.fail_next(
            RegistryOperation::Unregister,
            &previous,
            "simulated old cleanup failure",
        );
        registry.fail_next(
            RegistryOperation::Unregister,
            &previous,
            "simulated retry cleanup failure",
        );

        let error = replace_active_cancel_shortcut(
            &registry,
            &persistence,
            &mut bindings,
            previous.clone(),
            updated.clone(),
        )
        .unwrap_err();

        assert!(error.contains("simulated old cleanup failure"));
        assert!(error.contains("simulated previous settings rollback failure"));
        assert!(error.contains("simulated retry cleanup failure"));
        assert!(error.contains("Settings remain Updated"));
        assert!(registry.is_active(&previous));
        assert!(registry.is_active(&updated));
        assert_eq!(
            tracked_binding_names(&bindings),
            vec!["escape", "cmd+shift+c"]
        );

        apply_cancel_shortcut_lifecycle_operation(
            &registry,
            &mut bindings,
            3,
            3,
            CancelShortcutLifecycleOperation::Unregister,
            None,
        )
        .unwrap();

        assert_eq!(
            *persistence.state.borrow(),
            CancelShortcutSettingsState::Updated
        );
        assert!(!registry.is_active(&previous));
        assert!(!registry.is_active(&updated));
        assert!(bindings.is_empty());
    }

    #[test]
    fn failed_cleanup_and_rollback_return_composed_error_with_real_state() {
        let registry = FakeCancelShortcutRegistry::default();
        let persistence = FakeCancelShortcutPersistence::new(CancelShortcutSettingsState::Previous);
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        registry.activate(&previous);
        let mut bindings = vec![previous.clone()];
        registry.fail_next(
            RegistryOperation::Unregister,
            &previous,
            "simulated old cleanup failure",
        );
        registry.fail_next(
            RegistryOperation::Unregister,
            &updated,
            "simulated rollback failure",
        );

        let error = replace_active_cancel_shortcut(
            &registry,
            &persistence,
            &mut bindings,
            previous.clone(),
            updated.clone(),
        )
        .unwrap_err();

        assert!(error.contains("simulated old cleanup failure"));
        assert!(error.contains("simulated rollback failure"));
        assert!(error.contains("previous_registered=true, updated_registered=true"));
        assert!(registry.is_active(&previous));
        assert!(registry.is_active(&updated));
        assert_eq!(
            tracked_binding_names(&bindings),
            vec!["escape", "cmd+shift+c"]
        );
        assert_eq!(
            *persistence.state.borrow(),
            CancelShortcutSettingsState::Previous
        );
    }

    #[test]
    fn stale_lifecycle_request_cannot_register_an_old_cancel_shortcut() {
        let registry = FakeCancelShortcutRegistry::default();
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        registry.activate(&updated);
        let mut bindings = vec![updated.clone()];

        apply_cancel_shortcut_lifecycle_operation(
            &registry,
            &mut bindings,
            2,
            1,
            CancelShortcutLifecycleOperation::Register,
            Some(previous.clone()),
        )
        .unwrap();

        assert!(registry.is_active(&updated));
        assert!(!registry.is_active(&previous));
        assert!(registry.operations.borrow().is_empty());
    }
}
