//! Where Murmur's windows go on Windows: the monitor under the cursor, inside
//! its work area (the monitor without the taskbar, `rcWork` of
//! `GetMonitorInfoW`). Everything is in that monitor's physical pixels, so a
//! window moving between monitors of different scales lands where intended.

use crate::settings::OverlayPosition;
use tauri::{AppHandle, LogicalSize, Monitor, PhysicalPosition, PhysicalRect, PhysicalSize};

/// The monitor holding the cursor, else the primary one.
pub fn monitor_under_cursor(app: &AppHandle) -> Option<Monitor> {
    app.cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
}

/// The position that centers a window of `size` in `area`. A window larger
/// than the area keeps its left edge and title bar inside it.
pub fn centered_in(
    area: &PhysicalRect<i32, u32>,
    size: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    let center = |origin: i32, available: u32, length: u32| {
        origin
            + (i64::from(available)
                .saturating_sub(i64::from(length))
                .max(0)
                / 2) as i32
    };
    PhysicalPosition::new(
        center(area.position.x, area.size.width, size.width),
        center(area.position.y, area.size.height, size.height),
    )
}

/// The inner size and position of a window of `outer` and `inner` logical
/// size moved to a monitor of `scale` with work area `area`: its outer size
/// fitted to the area, its inner size never under `min_inner`, centered.
pub fn fitted_and_centered(
    area: &PhysicalRect<i32, u32>,
    scale: f64,
    outer: LogicalSize<f64>,
    inner: LogicalSize<f64>,
    min_inner: (f64, f64),
) -> (LogicalSize<f64>, PhysicalPosition<i32>) {
    let border = (outer.width - inner.width, outer.height - inner.height);
    let visible = area.size.to_logical::<f64>(scale);
    let (width, height) = crate::fitted_window_size(
        (visible.width, visible.height),
        (outer.width, outer.height),
        (min_inner.0 + border.0, min_inner.1 + border.1),
    );
    let size = LogicalSize::new(width, height).to_physical::<u32>(scale);
    (
        LogicalSize::new(width - border.0, height - border.1),
        centered_in(area, size),
    )
}

/// The overlay's rectangle: `logical_size` at the monitor's `scale`,
/// centered horizontally, `edge_gap` logical pixels from the work area's top
/// or bottom edge.
pub fn overlay_bounds(
    area: &PhysicalRect<i32, u32>,
    scale: f64,
    logical_size: (f64, f64),
    edge_gap: f64,
    position: OverlayPosition,
) -> PhysicalRect<i32, u32> {
    let to_physical = |logical: f64| (logical * scale).round().max(1.0) as u32;
    let size = PhysicalSize::new(to_physical(logical_size.0), to_physical(logical_size.1));
    let gap = (edge_gap * scale).round() as i32;
    let x = centered_in(area, size).x;
    let y = match position {
        OverlayPosition::Top => area.position.y + gap,
        OverlayPosition::Bottom => {
            area.position.y + area.size.height as i32 - size.height as i32 - gap
        }
    };
    PhysicalRect {
        position: PhysicalPosition::new(x, y),
        size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect<i32, u32> {
        PhysicalRect {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(width, height),
        }
    }

    #[test]
    fn a_window_is_centered_in_the_work_area() {
        // 1920x1080 with a 48-pixel taskbar at the bottom.
        let area = rect(0, 0, 1920, 1032);
        assert_eq!(
            centered_in(&area, PhysicalSize::new(680, 570)),
            PhysicalPosition::new(620, 231)
        );
    }

    #[test]
    fn a_window_is_centered_on_a_monitor_left_of_the_primary_one() {
        let area = rect(-2560, -200, 2560, 1400);
        assert_eq!(
            centered_in(&area, PhysicalSize::new(1020, 855)),
            PhysicalPosition::new(-1790, 72)
        );
    }

    #[test]
    fn a_window_larger_than_the_work_area_keeps_its_title_bar_visible() {
        let area = rect(100, 50, 600, 500);
        assert_eq!(
            centered_in(&area, PhysicalSize::new(680, 570)),
            PhysicalPosition::new(100, 50)
        );
    }

    /// The settings window at its default size, with Windows 11 borders: 8
    /// pixels left, right and bottom, and a 31-pixel title bar.
    fn settings_window() -> (LogicalSize<f64>, LogicalSize<f64>) {
        (
            LogicalSize::new(796.0, 759.0),
            LogicalSize::new(780.0, 720.0),
        )
    }

    const MIN_INNER: (f64, f64) = (680.0, 570.0);

    #[test]
    fn the_settings_window_fits_above_the_taskbar_of_a_1024x768_screen() {
        // The CI runner's desktop, taskbar 48 pixels high.
        let (outer, inner) = settings_window();
        let (size, position) =
            fitted_and_centered(&rect(0, 0, 1024, 720), 1.0, outer, inner, MIN_INNER);
        assert_eq!(size, LogicalSize::new(780.0, 681.0));
        assert_eq!(position, PhysicalPosition::new(114, 0));
    }

    #[test]
    fn the_settings_window_fits_a_150_percent_monitor_in_its_logical_size() {
        // 1920x1080 at 150% is 1280x720 logical; the taskbar takes 72 pixels.
        let (outer, inner) = settings_window();
        let (size, position) =
            fitted_and_centered(&rect(0, 0, 1920, 1008), 1.5, outer, inner, MIN_INNER);
        assert_eq!(size, LogicalSize::new(780.0, 633.0));
        assert_eq!(position, PhysicalPosition::new(363, 0));
    }

    #[test]
    fn the_settings_window_keeps_its_size_on_a_large_monitor() {
        let (outer, inner) = settings_window();
        let (size, position) =
            fitted_and_centered(&rect(0, 0, 1920, 1032), 1.0, outer, inner, MIN_INNER);
        assert_eq!(size, inner);
        assert_eq!(position, PhysicalPosition::new(562, 136));
    }

    #[test]
    fn the_settings_window_never_shrinks_below_its_minimum() {
        let (outer, inner) = settings_window();
        let (size, position) =
            fitted_and_centered(&rect(0, 0, 800, 560), 1.0, outer, inner, MIN_INNER);
        assert_eq!(size, LogicalSize::new(780.0, 570.0));
        // Too tall still: the title bar stays inside the work area.
        assert_eq!(position, PhysicalPosition::new(2, 0));
    }

    #[test]
    fn the_bottom_overlay_sits_above_the_taskbar_at_the_monitor_scale() {
        // A 150% monitor right of the primary one, taskbar 72 pixels high.
        let area = rect(1920, 0, 3840, 2088);
        let bounds = overlay_bounds(&area, 1.5, (158.0, 62.0), 0.0, OverlayPosition::Bottom);
        assert_eq!(bounds.size, PhysicalSize::new(237, 93));
        assert_eq!(bounds.position, PhysicalPosition::new(3721, 1995));
    }

    #[test]
    fn the_top_overlay_follows_a_taskbar_docked_at_the_top() {
        let area = rect(0, 40, 1920, 1040);
        let bounds = overlay_bounds(&area, 1.0, (158.0, 62.0), 8.0, OverlayPosition::Top);
        assert_eq!(bounds.position, PhysicalPosition::new(881, 48));
    }
}
