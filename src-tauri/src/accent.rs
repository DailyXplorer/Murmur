use crate::settings::AccentColor;
use tauri::image::Image;
use tauri::AppHandle;
#[cfg(not(target_os = "macos"))]
use tauri::Manager;

const APP_ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");
const TRAY_IDLE_PNG: &[u8] = include_bytes!("../resources/murmur.png");
const TRAY_RECORDING_PNG: &[u8] = include_bytes!("../resources/recording.png");
const TRAY_TRANSCRIBING_PNG: &[u8] = include_bytes!("../resources/transcribing.png");
const SOURCE_PINK_HUE: f32 = 332.0;

#[derive(Clone, Copy)]
pub enum NativeIconState {
    Idle,
    Recording,
    Transcribing,
}

impl AccentColor {
    fn hue(self) -> f32 {
        match self {
            Self::Pink => SOURCE_PINK_HUE,
            Self::Blue => 212.0,
            Self::Green => 145.0,
            Self::Yellow => 47.0,
            Self::Orange => 25.0,
            Self::Red => 2.0,
        }
    }

    fn native_rgb(self) -> [u8; 3] {
        match self {
            Self::Pink => [201, 63, 126],
            Self::Blue => [47, 115, 199],
            Self::Green => [36, 122, 72],
            Self::Yellow => [138, 106, 0],
            Self::Orange => [184, 90, 22],
            Self::Red => [184, 63, 74],
        }
    }
}

pub fn app_icon(accent_color: AccentColor) -> Result<Image<'static>, String> {
    let source = Image::from_bytes(APP_ICON_PNG).map_err(|error| error.to_string())?;
    Ok(recolor_brand_image(source, accent_color))
}

pub fn tray_icon(
    accent_color: AccentColor,
    state: NativeIconState,
) -> Result<Image<'static>, String> {
    let source = match state {
        NativeIconState::Idle => Image::from_bytes(TRAY_IDLE_PNG),
        NativeIconState::Recording => Image::from_bytes(TRAY_RECORDING_PNG),
        NativeIconState::Transcribing => Image::from_bytes(TRAY_TRANSCRIBING_PNG),
    }
    .map_err(|error| error.to_string())?;

    Ok(match state {
        NativeIconState::Idle => recolor_brand_image(source, accent_color),
        NativeIconState::Recording | NativeIconState::Transcribing => {
            tint_monochrome_image(source, accent_color.native_rgb())
        }
    })
}

/// Updates the icon shown for the running app. The installed bundle keeps its
/// signed pink icon; the chosen accent is restored each time Murmur launches.
pub fn apply_native_accent(app: &AppHandle, accent_color: AccentColor) -> Result<(), String> {
    let icon = app_icon(accent_color)?;

    #[cfg(not(target_os = "macos"))]
    if let Some(window) = app.get_webview_window("main") {
        window
            .set_icon(icon.clone())
            .map_err(|error| error.to_string())?;
    }

    #[cfg(target_os = "macos")]
    set_macos_application_icon(app, icon)?;

    Ok(())
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

fn tint_monochrome_image(source: Image<'_>, color: [u8; 3]) -> Image<'static> {
    let mut rgba = source.rgba().to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }
        pixel[0] = color[0];
        pixel[1] = color[1];
        pixel[2] = color[2];
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
    fn monochrome_tray_icons_use_the_requested_accent() {
        let source = Image::new_owned(vec![255, 255, 255, 128], 1, 1);
        let tinted = tint_monochrome_image(source, AccentColor::Orange.native_rgb());
        assert_eq!(tinted.rgba(), &[184, 90, 22, 128]);
    }

    #[test]
    fn every_accent_produces_native_icons() {
        for accent in [
            AccentColor::Pink,
            AccentColor::Blue,
            AccentColor::Green,
            AccentColor::Yellow,
            AccentColor::Orange,
            AccentColor::Red,
        ] {
            let app = app_icon(accent).expect("app icon should decode");
            let tray = tray_icon(accent, NativeIconState::Idle).expect("tray icon should decode");
            assert_eq!((app.width(), app.height()), (1024, 1024));
            assert_eq!((tray.width(), tray.height()), (32, 32));
        }
    }
}
