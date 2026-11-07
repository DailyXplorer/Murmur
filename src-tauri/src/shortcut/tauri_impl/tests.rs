use super::{
    apply_cancel_shortcut_lifecycle_operation, conflicting_binding, parse_shortcut,
    register_tracked_cancel_binding, replace_active_cancel_shortcut, replace_cancel_binding,
    CancelShortcutLifecycleOperation, CancelShortcutPersistence, CancelShortcutRegistry,
    CancelShortcutSettingsState,
};
use crate::settings::ShortcutBinding;
use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};

#[test]
fn cancel_binding_rollback_preserves_intervening_settings_and_other_bindings() {
    let mut settings = crate::settings::get_default_settings();
    let previous = settings.bindings["cancel"].clone();
    let mut updated = previous.clone();
    updated.current_binding = "ctrl+escape".to_string();
    replace_cancel_binding(&mut settings, &updated);

    // These changes may happen while the native rebind is in progress.
    settings.window_names_enabled = false;
    settings.custom_words.push("Murmur".to_string());
    settings
        .bindings
        .get_mut("transcribe")
        .unwrap()
        .current_binding = "ctrl+space".to_string();
    let mut expected = serde_json::to_value(&settings).unwrap();
    expected["bindings"]["cancel"] = serde_json::to_value(&previous).unwrap();

    replace_cancel_binding(&mut settings, &previous);
    assert_eq!(serde_json::to_value(&settings).unwrap(), expected);
}

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
        if self.active.borrow().contains(&binding.current_binding) {
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

fn transcribe_binding(binding: &str) -> ShortcutBinding {
    ShortcutBinding {
        id: "transcribe".to_string(),
        name: "Transcribe".to_string(),
        description: "Converts your speech into text.".to_string(),
        default_binding: "option+space".to_string(),
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
fn a_shortcut_already_bound_to_another_action_is_a_conflict() {
    let mut settings = crate::settings::get_default_settings();
    settings.bindings = std::collections::HashMap::from([
        ("transcribe".to_string(), transcribe_binding("option+space")),
        ("cancel".to_string(), cancel_binding("escape")),
    ]);

    let conflict = |id: &str, raw: &str| {
        conflicting_binding(&settings, id, raw).map(|other| other.id.as_str())
    };
    assert_eq!(conflict("transcribe", "esc"), Some("cancel"));
    assert_eq!(conflict("cancel", "alt+space"), Some("transcribe"));
    assert_eq!(conflict("transcribe", "option+space"), None);
    assert_eq!(conflict("cancel", "ctrl+space"), None);
}

#[test]
fn only_shortcuts_the_hotkey_parser_accepts_are_valid() {
    let accepted = |raw: &str| parse_shortcut(raw).is_ok();
    assert!(accepted("option+space"));
    assert!(accepted("ctrl+command+v"));
    assert!(accepted("pageup"));
    assert!(accepted("numpad1"));
    assert!(accepted("escape"));
    assert!(!accepted("page up"));
    assert!(!accepted("caps lock"));
    assert!(!accepted("capslock"));
    assert!(!accepted("ctrl+capslock"));
    assert!(!accepted("numpad +"));
    assert!(!accepted("f5+f6"));
    assert!(!accepted("command+shift"));
    assert!(!accepted("fn+space"));
    assert!(!accepted(""));
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
fn lifecycle_register_is_a_no_op_for_an_owned_cancel_binding() {
    let registry = FakeCancelShortcutRegistry::default();
    let cancel = cancel_binding("escape");
    registry.activate(&cancel);
    let mut bindings = vec![cancel.clone()];

    apply_cancel_shortcut_lifecycle_operation(
        &registry,
        &mut bindings,
        2,
        2,
        CancelShortcutLifecycleOperation::Register,
        Some(cancel.clone()),
    )
    .unwrap();

    assert!(registry.is_active(&cancel));
    assert_eq!(tracked_binding_names(&bindings), vec!["escape"]);
    assert!(registry.operations.borrow().is_empty());
}

#[test]
fn failed_reregistration_preserves_an_owned_cancel_binding() {
    let registry = FakeCancelShortcutRegistry::default();
    let cancel = cancel_binding("escape");
    registry.activate(&cancel);
    let mut bindings = vec![cancel.clone()];
    registry.fail_next(
        RegistryOperation::Register,
        &cancel,
        "simulated re-registration failure",
    );

    let error =
        register_tracked_cancel_binding(&registry, &mut bindings, cancel.clone()).unwrap_err();

    assert!(error.contains("simulated re-registration failure"));
    assert!(registry.is_active(&cancel));
    assert_eq!(tracked_binding_names(&bindings), vec!["escape"]);
}

#[test]
fn lifecycle_collision_never_adopts_a_foreign_global_binding() {
    let registry = FakeCancelShortcutRegistry::default();
    let cancel = cancel_binding("cmd+shift+c");
    let transcribe = transcribe_binding("cmd+shift+c");
    registry.activate(&transcribe);
    let mut bindings = Vec::new();

    let error = apply_cancel_shortcut_lifecycle_operation(
        &registry,
        &mut bindings,
        2,
        2,
        CancelShortcutLifecycleOperation::Register,
        Some(cancel),
    )
    .unwrap_err();

    assert!(error.contains("already in use"));
    assert!(registry.is_active(&transcribe));
    assert!(bindings.is_empty());

    apply_cancel_shortcut_lifecycle_operation(
        &registry,
        &mut bindings,
        3,
        3,
        CancelShortcutLifecycleOperation::Unregister,
        None,
    )
    .unwrap();

    assert!(registry.is_active(&transcribe));
    assert_eq!(
        *registry.operations.borrow(),
        vec!["register:cmd+shift+c".to_string()]
    );
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
