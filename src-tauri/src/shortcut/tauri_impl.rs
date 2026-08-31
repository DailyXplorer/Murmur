//! Tauri global-shortcut implementation
//!
//! This module provides shortcut functionality using Tauri's built-in
//! global-shortcut plugin.

use log::{debug, error, warn};
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

fn apply_cancel_shortcut_lifecycle_operation<R: CancelShortcutRegistry>(
    registry: &R,
    current_epoch: u64,
    request_epoch: u64,
    operation: CancelShortcutLifecycleOperation,
    binding: ShortcutBinding,
) -> Result<(), String> {
    if current_epoch != request_epoch {
        return Ok(());
    }

    match operation {
        CancelShortcutLifecycleOperation::Register => registry.register(binding),
        CancelShortcutLifecycleOperation::Unregister => registry.unregister(binding),
    }
}

fn replace_active_cancel_shortcut<R: CancelShortcutRegistry, P: FnOnce()>(
    registry: &R,
    previous: ShortcutBinding,
    updated: ShortcutBinding,
    persist: P,
) -> Result<(), String> {
    if registry.is_registered(&previous) {
        registry.unregister(previous.clone())?;

        if let Err(error) = registry.register(updated.clone()) {
            if let Err(restore_error) = registry.register(previous) {
                return Err(format!(
                    "Failed to register the new Cancel shortcut: {error}. Failed to restore the previous Cancel shortcut: {restore_error}"
                ));
            }
            return Err(error);
        }
    }

    persist();
    Ok(())
}

/// Update Cancel while preserving its dynamic lifecycle. Idle changes only
/// update settings; active changes atomically replace the registered shortcut.
pub fn change_cancel_binding(app: &AppHandle, binding: String) -> Result<ShortcutBinding, String> {
    let _operation = CANCEL_SHORTCUT_OPERATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let mut settings: AppSettings = get_settings(app);
    let previous = settings
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
    let updated_for_store = updated.clone();
    replace_active_cancel_shortcut(&registry, previous, updated.clone(), || {
        settings
            .bindings
            .insert("cancel".to_string(), updated_for_store);
        settings::write_settings(app, settings);
    })?;

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
            if let Err(e) = apply_cancel_shortcut_lifecycle_operation(
                &registry,
                CANCEL_SHORTCUT_EPOCH.load(Ordering::Acquire),
                request_epoch,
                CancelShortcutLifecycleOperation::Register,
                cancel_binding,
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
        if let Some(cancel_binding) = get_settings(&app_clone).bindings.get("cancel").cloned() {
            let registry = TauriCancelShortcutRegistry { app: &app_clone };
            let _ = apply_cancel_shortcut_lifecycle_operation(
                &registry,
                CANCEL_SHORTCUT_EPOCH.load(Ordering::Acquire),
                request_epoch,
                CancelShortcutLifecycleOperation::Unregister,
                cancel_binding,
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        apply_cancel_shortcut_lifecycle_operation, replace_active_cancel_shortcut,
        CancelShortcutLifecycleOperation, CancelShortcutRegistry,
    };
    use crate::settings::ShortcutBinding;
    use std::cell::RefCell;
    use std::collections::BTreeSet;

    #[derive(Default)]
    struct FakeCancelShortcutRegistry {
        active: RefCell<BTreeSet<String>>,
        blocked_registration: RefCell<Option<String>>,
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
    }

    impl CancelShortcutRegistry for FakeCancelShortcutRegistry {
        fn is_registered(&self, binding: &ShortcutBinding) -> bool {
            self.is_active(binding)
        }

        fn register(&self, binding: ShortcutBinding) -> Result<(), String> {
            self.operations
                .borrow_mut()
                .push(format!("register:{}", binding.current_binding));
            if self.blocked_registration.borrow().as_deref()
                == Some(binding.current_binding.as_str())
            {
                return Err(format!(
                    "Shortcut '{}' is already in use",
                    binding.current_binding
                ));
            }
            self.active.borrow_mut().insert(binding.current_binding);
            Ok(())
        }

        fn unregister(&self, binding: ShortcutBinding) -> Result<(), String> {
            self.operations
                .borrow_mut()
                .push(format!("unregister:{}", binding.current_binding));
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

    fn cancel_binding(binding: &str) -> ShortcutBinding {
        ShortcutBinding {
            id: "cancel".to_string(),
            name: "Cancel".to_string(),
            description: "Cancels the current recording.".to_string(),
            default_binding: "escape".to_string(),
            current_binding: binding.to_string(),
        }
    }

    #[test]
    fn active_replace_then_finish_unregisters_the_new_cancel_shortcut() {
        let registry = FakeCancelShortcutRegistry::default();
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        let persisted = RefCell::new(previous.current_binding.clone());
        registry.activate(&previous);

        replace_active_cancel_shortcut(&registry, previous.clone(), updated.clone(), || {
            *persisted.borrow_mut() = updated.current_binding.clone();
        })
        .unwrap();
        apply_cancel_shortcut_lifecycle_operation(
            &registry,
            2,
            2,
            CancelShortcutLifecycleOperation::Unregister,
            updated.clone(),
        )
        .unwrap();

        assert_eq!(*persisted.borrow(), "cmd+shift+c");
        assert!(!registry.is_active(&previous));
        assert!(!registry.is_active(&updated));
        assert_eq!(
            *registry.operations.borrow(),
            vec![
                "unregister:escape".to_string(),
                "register:cmd+shift+c".to_string(),
                "unregister:cmd+shift+c".to_string(),
            ]
        );
    }

    #[test]
    fn idle_replace_persists_without_registering_cancel() {
        let registry = FakeCancelShortcutRegistry::default();
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        let persisted = RefCell::new(previous.current_binding.clone());

        replace_active_cancel_shortcut(&registry, previous, updated.clone(), || {
            *persisted.borrow_mut() = updated.current_binding.clone();
        })
        .unwrap();

        assert_eq!(*persisted.borrow(), "cmd+shift+c");
        assert!(!registry.is_active(&updated));
        assert!(registry.operations.borrow().is_empty());
    }

    #[test]
    fn collision_restores_active_cancel_and_does_not_persist_the_change() {
        let registry = FakeCancelShortcutRegistry::default();
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        let persisted = RefCell::new(previous.current_binding.clone());
        registry.activate(&previous);
        *registry.blocked_registration.borrow_mut() = Some(updated.current_binding.clone());

        let result =
            replace_active_cancel_shortcut(&registry, previous.clone(), updated.clone(), || {
                *persisted.borrow_mut() = updated.current_binding.clone();
            });

        assert!(result.is_err());
        assert_eq!(*persisted.borrow(), "escape");
        assert!(registry.is_active(&previous));
        assert!(!registry.is_active(&updated));
        assert_eq!(
            *registry.operations.borrow(),
            vec![
                "unregister:escape".to_string(),
                "register:cmd+shift+c".to_string(),
                "register:escape".to_string(),
            ]
        );
    }

    #[test]
    fn stale_lifecycle_request_cannot_register_an_old_cancel_shortcut() {
        let registry = FakeCancelShortcutRegistry::default();
        let previous = cancel_binding("escape");
        let updated = cancel_binding("cmd+shift+c");
        registry.activate(&updated);

        apply_cancel_shortcut_lifecycle_operation(
            &registry,
            2,
            1,
            CancelShortcutLifecycleOperation::Register,
            previous.clone(),
        )
        .unwrap();

        assert!(registry.is_active(&updated));
        assert!(!registry.is_active(&previous));
        assert!(registry.operations.borrow().is_empty());
    }
}
