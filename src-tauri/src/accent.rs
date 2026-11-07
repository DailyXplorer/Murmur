use crate::settings::AccentColor;
use tauri::image::Image;
use tauri::AppHandle;
#[cfg(target_os = "macos")]
use {crate::settings, tauri::ActivationPolicy};

/// The app icon at 512 px (`icons/icon.png` downscaled): plenty for the Dock
/// and a window icon, and a quarter of the 1024 px source in memory.
const APP_ICON_PNG: &[u8] = include_bytes!("../icons/accent-source-512.png");
#[cfg(target_os = "macos")]
const TRAY_ICON_PNG: &[u8] = include_bytes!("../resources/tray.png");
/// Windows draws tray icons as they are, on light and dark taskbars alike,
/// so its tray shows the app's colored mark rather than a black template.
#[cfg(target_os = "windows")]
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/32x32.png");
const SOURCE_PINK_HUE: f32 = 325.0;
pub const TRAY_ICON_IS_TEMPLATE: bool = cfg!(target_os = "macos");

impl AccentColor {
    fn hue(self) -> f32 {
        match self {
            Self::Pink => SOURCE_PINK_HUE,
            Self::Blue => 212.0,
            Self::Green => 131.0,
            Self::Yellow => 47.0,
            Self::Orange => 25.0,
            Self::Red => 2.0,
        }
    }
}

pub fn app_icon(accent_color: AccentColor) -> Result<Image<'static>, String> {
    let source = Image::from_bytes(APP_ICON_PNG).map_err(|error| error.to_string())?;
    Ok(recolor_brand_image(source, accent_color))
}

pub fn tray_icon() -> Result<Image<'static>, String> {
    Image::from_bytes(TRAY_ICON_PNG).map_err(|error| error.to_string())
}

/// Whether setting the Dock icon changes anything. The bundle's signed icon is
/// the default pink one, so Pink matters only to take back a custom accent set
/// earlier in this run.
#[cfg(target_os = "macos")]
fn dock_icon_needs_update(accent_color: AccentColor, custom_icon_set: bool) -> bool {
    accent_color != AccentColor::Pink || custom_icon_set
}

/// True while the Dock icon shows an accent other than the bundle's.
#[cfg(target_os = "macos")]
static CUSTOM_DOCK_ICON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Sets the icon at launch. With the Dock tile hidden (`dock_hidden`) there is
/// nothing to draw it into: showing the tile later applies the accent then.
pub fn restore_accent_at_launch(
    app: &AppHandle,
    accent_color: AccentColor,
    dock_hidden: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if dock_hidden {
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = dock_hidden;
    apply_native_accent(app, accent_color)
}

/// Updates the icon shown for the running app. The installed bundle keeps its
/// signed pink icon; the chosen accent is restored each time Murmur launches
/// (`restore_accent_at_launch`), except for Pink, which the bundle already is.
pub fn apply_native_accent(app: &AppHandle, accent_color: AccentColor) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::sync::atomic::Ordering;
        if !dock_icon_needs_update(accent_color, CUSTOM_DOCK_ICON.load(Ordering::Relaxed)) {
            return Ok(());
        }
        let icon = app_icon(accent_color)?;
        set_macos_application_icon(app, icon)?;
        CUSTOM_DOCK_ICON.store(accent_color != AccentColor::Pink, Ordering::Relaxed);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let icon = app_icon(accent_color)?;
        // The taskbar shows the settings window's own icon.
        #[cfg(target_os = "windows")]
        if let Some(window) = tauri::Manager::get_webview_window(app, "main") {
            window.set_icon(icon).map_err(|error| error.to_string())?;
        }
        #[cfg(not(target_os = "windows"))]
        let _ = (app, icon);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockPresence {
    Shown,
    Hidden,
}

/// The only place Murmur changes its activation policy. Becoming Regular gives
/// the app a new Dock tile with the bundle's pink icon, and AppKit does not draw
/// the earlier `setApplicationIconImage` into it, so showing re-applies the accent.
#[cfg(target_os = "macos")]
pub fn set_dock_presence(app: &AppHandle, presence: DockPresence) {
    #[allow(clippy::disallowed_methods)]
    update_dock(
        presence,
        |policy| {
            app.set_activation_policy(policy)
                .map_err(|error| error.to_string())
        },
        || apply_native_accent(app, settings::get_settings(app).accent_color),
    );
}

#[cfg(target_os = "macos")]
fn update_dock(
    presence: DockPresence,
    set_activation_policy: impl FnOnce(ActivationPolicy) -> Result<(), String>,
    apply_accent: impl FnOnce() -> Result<(), String>,
) {
    let policy = match presence {
        DockPresence::Shown => ActivationPolicy::Regular,
        DockPresence::Hidden => ActivationPolicy::Accessory,
    };
    if let Err(error) = set_activation_policy(policy) {
        log::error!("Failed to set the activation policy for {presence:?}: {error}");
    }
    if presence == DockPresence::Shown {
        if let Err(error) = apply_accent() {
            log::warn!("Failed to apply the accent icon to the Dock tile: {error}");
        }
    }
}

fn recolor_brand_image(source: Image<'_>, accent_color: AccentColor) -> Image<'static> {
    if accent_color == AccentColor::Pink {
        return source.to_owned();
    }

    let hue_offset = accent_color.hue() - SOURCE_PINK_HUE;
    let mut rgba = source.rgba().to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }

        let (hue, saturation, lightness) = rgb_to_hsl(pixel[0], pixel[1], pixel[2]);
        if saturation < 0.06 {
            continue;
        }

        let target_hue = (hue + hue_offset).rem_euclid(360.0);
        let [red, green, blue] = hsl_to_rgb(target_hue, saturation, lightness);
        pixel[0] = red;
        pixel[1] = green;
        pixel[2] = blue;
    }

    Image::new_owned(rgba, source.width(), source.height())
}

fn rgb_to_hsl(red: u8, green: u8, blue: u8) -> (f32, f32, f32) {
    let red = red as f32 / 255.0;
    let green = green as f32 / 255.0;
    let blue = blue as f32 / 255.0;
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let lightness = (max + min) / 2.0;
    let delta = max - min;

    if delta <= f32::EPSILON {
        return (0.0, 0.0, lightness);
    }

    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue_sector = if (max - red).abs() <= f32::EPSILON {
        ((green - blue) / delta).rem_euclid(6.0)
    } else if (max - green).abs() <= f32::EPSILON {
        (blue - red) / delta + 2.0
    } else {
        (red - green) / delta + 4.0
    };

    (hue_sector * 60.0, saturation, lightness)
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [u8; 3] {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let hue_sector = hue / 60.0;
    let secondary = chroma * (1.0 - (hue_sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match hue_sector {
        value if value < 1.0 => (chroma, secondary, 0.0),
        value if value < 2.0 => (secondary, chroma, 0.0),
        value if value < 3.0 => (0.0, chroma, secondary),
        value if value < 4.0 => (0.0, secondary, chroma),
        value if value < 5.0 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let offset = lightness - chroma / 2.0;
    [
        ((red + offset) * 255.0).round() as u8,
        ((green + offset) * 255.0).round() as u8,
        ((blue + offset) * 255.0).round() as u8,
    ]
}

#[cfg(target_os = "macos")]
fn set_macos_application_icon(app: &AppHandle, icon: Image<'static>) -> Result<(), String> {
    use image::codecs::png::PngEncoder;
    use image::{ExtendedColorType, ImageEncoder};

    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            icon.rgba(),
            icon.width(),
            icon.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| error.to_string())?;

    app.run_on_main_thread(move || {
        use objc2::{AllocAnyThread, MainThreadMarker};
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::NSData;

        let Some(main_thread) = MainThreadMarker::new() else {
            log::error!("Native accent update did not run on the macOS main thread");
            return;
        };
        let data = NSData::with_bytes(&png);
        let Some(app_icon) = NSImage::initWithData(NSImage::alloc(), &data) else {
            log::error!("Failed to decode the recolored macOS application icon");
            return;
        };
        let application = NSApplication::sharedApplication(main_thread);
        unsafe { application.setApplicationIconImage(Some(&app_icon)) };
    })
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba_at(image: &Image<'_>, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * image.width() + x) * 4) as usize;
        image.rgba()[offset..offset + 4]
            .try_into()
            .expect("pixel should contain four channels")
    }

    fn alpha_at(image: &Image<'_>, x: u32, y: u32) -> u8 {
        rgba_at(image, x, y)[3]
    }

    fn circular_hue_distance(first: f32, second: f32) -> f32 {
        (first - second + 180.0).rem_euclid(360.0) - 180.0
    }

    #[cfg(target_os = "macos")]
    fn visible_bounds(image: &Image<'_>) -> (u32, u32, u32, u32) {
        let mut min_x = image.width();
        let mut min_y = image.height();
        let mut max_x = 0;
        let mut max_y = 0;
        let mut found_visible_pixel = false;

        for y in 0..image.height() {
            for x in 0..image.width() {
                if alpha_at(image, x, y) == 0 {
                    continue;
                }
                found_visible_pixel = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }

        assert!(found_visible_pixel, "image should contain visible pixels");
        (min_x, min_y, max_x, max_y)
    }

    #[test]
    fn recoloring_preserves_transparent_and_neutral_pixels() {
        let source = Image::new_owned(
            vec![0, 0, 0, 0, 255, 255, 255, 255, 250, 162, 202, 255],
            3,
            1,
        );
        let recolored = recolor_brand_image(source, AccentColor::Blue);

        assert_eq!(&recolored.rgba()[0..4], &[0, 0, 0, 0]);
        assert_eq!(&recolored.rgba()[4..8], &[255, 255, 255, 255]);
        assert!(recolored.rgba()[10] > recolored.rgba()[8]);
    }

    #[test]
    fn every_accent_keeps_the_app_icon_invariants() {
        for (accent, expected_hue) in [
            (AccentColor::Pink, 325.0),
            (AccentColor::Blue, 212.0),
            (AccentColor::Green, 131.0),
            (AccentColor::Yellow, 47.0),
            (AccentColor::Orange, 25.0),
            (AccentColor::Red, 2.0),
        ] {
            let app = app_icon(accent).expect("app icon should decode");
            assert_eq!((app.width(), app.height()), (512, 512));
            assert_eq!(alpha_at(&app, 0, 0), 0);
            assert_eq!(rgba_at(&app, 256, 40), [255, 255, 255, 255]);
            assert_eq!(rgba_at(&app, 256, 470), [255, 255, 255, 255]);

            let pixel = rgba_at(&app, 150, 175);
            let (hue, saturation, _) = rgb_to_hsl(pixel[0], pixel[1], pixel[2]);
            assert!(saturation > 0.7, "mark pixel should remain saturated");
            assert!(
                circular_hue_distance(hue, expected_hue).abs() <= 5.0,
                "expected hue {expected_hue}, got {hue} for {accent:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_default_accent_leaves_the_bundle_icon_alone() {
        assert!(!dock_icon_needs_update(AccentColor::Pink, false));
        for accent in [
            AccentColor::Blue,
            AccentColor::Green,
            AccentColor::Yellow,
            AccentColor::Orange,
            AccentColor::Red,
        ] {
            assert!(dock_icon_needs_update(accent, false), "{accent:?}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pink_takes_back_a_custom_icon_set_earlier() {
        assert!(dock_icon_needs_update(AccentColor::Pink, true));
    }

    #[test]
    fn the_accent_source_is_the_app_icon_at_half_size() {
        let full = Image::from_bytes(include_bytes!("../icons/icon.png")).unwrap();
        let half = Image::from_bytes(APP_ICON_PNG).unwrap();
        assert_eq!((full.width(), full.height()), (1024, 1024));
        assert_eq!((half.width(), half.height()), (512, 512));
        // Same shape and colors: compare the 2x2 block at the center and a
        // corner of the plate.
        for (x, y) in [(256_u32, 256_u32), (100, 400), (0, 0), (256, 10)] {
            let block: Vec<[u8; 4]> = [(0, 0), (1, 0), (0, 1), (1, 1)]
                .iter()
                .map(|(dx, dy)| rgba_at(&full, x * 2 + dx, y * 2 + dy))
                .collect();
            let mean =
                |channel: usize| block.iter().map(|px| u32::from(px[channel])).sum::<u32>() / 4;
            let small = rgba_at(&half, x, y);
            for (channel, value) in small.iter().enumerate() {
                let diff = (i64::from(*value) - i64::from(mean(channel))).abs();
                assert!(diff <= 24, "pixel ({x},{y}) channel {channel}: {diff}");
            }
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_tray_icon_is_the_colored_app_mark() {
        let tray = tray_icon().expect("tray icon should decode");

        assert_eq!((tray.width(), tray.height()), (32, 32));
        assert!(tray.rgba().chunks_exact(4).any(|pixel| {
            let (_, saturation, _) = rgb_to_hsl(pixel[0], pixel[1], pixel[2]);
            pixel[3] > 0 && saturation > 0.5
        }));
    }

    #[cfg(target_os = "macos")]
    fn dock_calls(presence: DockPresence) -> Vec<String> {
        let calls = std::cell::RefCell::new(Vec::new());
        update_dock(
            presence,
            |policy| {
                let name = match policy {
                    ActivationPolicy::Regular => "policy Regular",
                    ActivationPolicy::Accessory => "policy Accessory",
                    _ => "policy other",
                };
                calls.borrow_mut().push(name.to_string());
                Ok(())
            },
            || {
                calls.borrow_mut().push("accent icon".to_string());
                Ok(())
            },
        );
        calls.into_inner()
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn showing_the_dock_tile_applies_the_accent_icon_last() {
        assert_eq!(
            dock_calls(DockPresence::Shown),
            ["policy Regular", "accent icon"]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn hiding_the_dock_tile_only_changes_the_policy() {
        assert_eq!(dock_calls(DockPresence::Hidden), ["policy Accessory"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_tray_icon_is_a_monochrome_template() {
        let tray = tray_icon().expect("tray icon should decode");

        assert_eq!((tray.width(), tray.height()), (64, 64));
        assert_eq!(alpha_at(&tray, 0, 0), 0);
        assert_eq!(visible_bounds(&tray), (5, 9, 58, 53));
        assert!(tray.rgba().chunks_exact(4).any(|pixel| pixel[3] == 0));
        assert!(tray
            .rgba()
            .chunks_exact(4)
            .filter(|pixel| pixel[3] > 0)
            .all(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0));
    }
}
