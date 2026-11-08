use super::{
    leave_update_relaunch_marker, normalize_generated_bindings_source, take_update_relaunch_marker,
};
use crate::headless::run_headless_guarded;

/// AppKit's settings window placement.
#[cfg(target_os = "macos")]
mod settings_window {
    use crate::app_windows::{
        centered_window_origin, screen_containing, settings_window_collection_behavior,
        settings_window_needs_order_out, SETTINGS_WINDOW_SIZE,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    #[test]
    fn settings_window_moves_to_active_space_without_joining_every_space() {
        use objc2_app_kit::NSWindowCollectionBehavior;

        let current =
            NSWindowCollectionBehavior::Managed | NSWindowCollectionBehavior::CanJoinAllSpaces;
        let configured = settings_window_collection_behavior(current);

        assert!(configured.contains(NSWindowCollectionBehavior::Managed));
        assert!(configured.contains(NSWindowCollectionBehavior::MoveToActiveSpace));
        assert!(!configured.contains(NSWindowCollectionBehavior::CanJoinAllSpaces));
    }

    #[test]
    fn settings_window_is_reordered_only_when_visible_on_another_space() {
        assert!(settings_window_needs_order_out(true, false));
        assert!(!settings_window_needs_order_out(false, false));
        assert!(!settings_window_needs_order_out(true, true));
        assert!(!settings_window_needs_order_out(false, true));
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }

    #[test]
    fn settings_window_is_centered_in_the_visible_frame() {
        // A 1512x982 screen whose visible frame drops a 33-point menu bar and
        // a 70-point Dock. The vertical center is the visible frame's, not the
        // slightly higher spot NSWindow.center() picks.
        let visible = rect(0.0, 70.0, 1512.0, 879.0);
        let origin = centered_window_origin(visible, frame(SETTINGS_WINDOW_SIZE));
        assert_eq!((origin.x, origin.y), (366.0, 135.0));
    }

    #[test]
    fn settings_window_is_centered_on_a_secondary_screen() {
        // A screen left of the main one has negative AppKit coordinates.
        let visible = rect(-1920.0, -200.0, 1920.0, 1055.0);
        let origin = centered_window_origin(visible, frame(SETTINGS_WINDOW_SIZE));
        assert_eq!((origin.x, origin.y), (-1350.0, -47.0));
    }

    #[test]
    fn a_window_larger_than_the_screen_keeps_its_title_bar_visible() {
        let visible = rect(0.0, 0.0, 600.0, 500.0);
        let size = frame(SETTINGS_WINDOW_SIZE);
        let origin = centered_window_origin(visible, size);
        // Left edge and top edge stay inside the visible frame.
        assert_eq!((origin.x, origin.y + size.height), (0.0, 500.0));
    }

    /// The frame size of a content size, with the 28-point title bar.
    fn frame(content: (f64, f64)) -> NSSize {
        NSSize::new(content.0, content.1 + 28.0)
    }

    #[test]
    fn the_screen_under_the_pointer_is_the_one_containing_it() {
        let screens = [
            rect(0.0, 0.0, 1512.0, 982.0),
            rect(1512.0, -100.0, 1920.0, 1080.0),
        ];
        assert_eq!(
            screen_containing(&screens, NSPoint::new(2000.0, 500.0)),
            Some(1)
        );
        assert_eq!(
            screen_containing(&screens, NSPoint::new(10.0, 982.0)),
            Some(0)
        );
        assert_eq!(screen_containing(&screens, NSPoint::new(-5.0, 10.0)), None);
    }
}

/// The settings window's size on a screen, the same on macOS and Windows.
mod settings_window_size {
    use crate::app_windows::{fitted_window_size, SETTINGS_WINDOW_MIN_SIZE, SETTINGS_WINDOW_SIZE};

    /// The frame size of a content size, with a macOS 28-point title bar.
    fn frame(content: (f64, f64)) -> (f64, f64) {
        (content.0, content.1 + 28.0)
    }

    #[test]
    fn settings_window_keeps_its_default_size_on_a_large_screen() {
        let size = fitted_window_size(
            (1512.0, 879.0),
            frame(SETTINGS_WINDOW_SIZE),
            frame(SETTINGS_WINDOW_MIN_SIZE),
        );
        assert_eq!(size, frame(SETTINGS_WINDOW_SIZE));
    }

    #[test]
    fn settings_window_shrinks_to_a_small_screen() {
        // A 13" 1280x800 screen less a 25-point menu bar and a 70-point Dock.
        let size = fitted_window_size(
            (1280.0, 705.0),
            frame(SETTINGS_WINDOW_SIZE),
            frame(SETTINGS_WINDOW_MIN_SIZE),
        );
        assert_eq!(size, (SETTINGS_WINDOW_SIZE.0, 705.0));
    }

    #[test]
    fn settings_window_never_shrinks_below_its_minimum() {
        let size = fitted_window_size(
            (600.0, 500.0),
            frame(SETTINGS_WINDOW_SIZE),
            frame(SETTINGS_WINDOW_MIN_SIZE),
        );
        assert_eq!(size, frame(SETTINGS_WINDOW_MIN_SIZE));
    }
}

#[test]
fn an_update_relaunch_shows_the_window_once() {
    let dir = tempfile::tempdir().unwrap();
    let app_data_dir = dir.path().join("not-created-yet");
    assert!(!take_update_relaunch_marker(&app_data_dir));

    leave_update_relaunch_marker(&app_data_dir).unwrap();
    assert!(take_update_relaunch_marker(&app_data_dir));
    // The next start, at login for instance, follows "Start hidden" again.
    assert!(!take_update_relaunch_marker(&app_data_dir));
}

#[test]
fn marking_an_update_relaunch_twice_still_shows_the_window_once() {
    // `prepare_update_install` marks before the install and
    // `relaunch_after_update` marks again on macOS.
    let dir = tempfile::tempdir().unwrap();
    leave_update_relaunch_marker(dir.path()).unwrap();
    leave_update_relaunch_marker(dir.path()).unwrap();
    assert!(take_update_relaunch_marker(dir.path()));
    assert!(!take_update_relaunch_marker(dir.path()));
}

/// `MURMUR_WRITE_BINDINGS=1 cargo test bindings_are_current` rewrites the
/// file after a command or event changes, without launching the app.
#[test]
fn bindings_are_current() {
    let generated = super::typescript_bindings(&super::specta_builder());
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts");
    if std::env::var_os("MURMUR_WRITE_BINDINGS").is_some() {
        std::fs::write(path, &generated).unwrap();
    }
    assert!(
        std::fs::read_to_string(path).unwrap() == generated,
        "src/bindings.ts is stale: run with MURMUR_WRITE_BINDINGS=1"
    );
}

#[test]
fn preserves_normal_exit_codes() {
    assert_eq!(run_headless_guarded(|| 2), 2);
}

#[test]
fn converts_worker_panics_to_runtime_failures() {
    assert_eq!(run_headless_guarded(|| panic!("simulated failure")), 1);
}

#[test]
fn generated_result_errors_are_normalized_to_strings() {
    let generated = concat!(
        "async command(): Promise<Result<null, string>> { ",
        "else return { status: \"error\", error: e\t as   any }; }",
    );
    let normalized = normalize_generated_bindings_source(generated).unwrap();
    assert_eq!(
        normalized,
        concat!(
            "async command(): Promise<Result<null, string>> { ",
            r#"else return { status: "error", error: String(e) }; }"#,
        )
    );
}

#[test]
fn generated_enum_errors_keep_their_type() {
    let generated = concat!(
        "async first(): Promise<Result<Record<string, Foo>, string>> { error: e as any }\n",
        "async second(): Promise<Result<null, SignInError>> { error: e as any }",
    );
    assert_eq!(
        normalize_generated_bindings_source(generated).unwrap(),
        concat!(
            "async first(): Promise<Result<Record<string, Foo>, string>> { error: String(e) }\n",
            "async second(): Promise<Result<null, SignInError>> { error: e as SignInError }",
        )
    );
}

#[test]
fn unrecognized_generated_result_wrapper_fails_loudly() {
    let generated = "async command(): Promise<Result<null, string>> { error: e as unknown }";
    assert!(normalize_generated_bindings_source(generated).is_err());
}

#[test]
fn mixed_generated_result_wrappers_fail_loudly() {
    let generated = concat!(
        "async first(): Promise<Result<null, string>> { error: e as any }",
        "async second(): Promise<Result<null, string>> { error: e as unknown }",
    );
    assert!(normalize_generated_bindings_source(generated).is_err());
}
