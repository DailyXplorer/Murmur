//! The Insights share card. The webview draws the PNG; the app puts it on
//! the clipboard or saves it. The app does both, not the webview: WebKit
//! before Safari 26.4 refuses a webview clipboard write that waited on
//! something, and the save dialog is the platform's own (`NSSavePanel`,
//! `IFileSaveDialog`). The picture holds numbers only and leaves the machine
//! only if the user shares the file or pastes it.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::path::PathBuf;
use tauri::{image::Image, AppHandle};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::meetings::save_panel;

/// The name the save dialog suggests.
const DEFAULT_FILE_NAME: &str = "murmur-insights.png";
/// The card is 1080 px square; this leaves room for a larger one later and
/// stops a decoder from allocating for a header that claims a huge image.
const MAX_SIDE: u32 = 4096;
/// A 1080 px card is about 150 KB.
const MAX_PNG_BYTES: usize = 8 * 1024 * 1024;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// The width and height an encoded PNG declares, read from its header
/// without decoding it. `None` when the bytes do not start like a PNG.
fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    // Signature, then the IHDR chunk: length, type, width, height.
    let header = png.get(..24)?;
    if header[..8] != PNG_SIGNATURE || &header[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(header[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(header[20..24].try_into().ok()?);
    Some((width, height))
}

/// Decodes the base64 text the webview sends and checks it is a PNG of a
/// sensible size. The bytes are what gets saved, so a caller cannot make
/// the app write anything else under that name.
fn checked_png(encoded: &str) -> Result<Vec<u8>, String> {
    if encoded.len() > MAX_PNG_BYTES.div_ceil(3) * 4 {
        return Err("The image is too large".to_string());
    }
    let png = STANDARD
        .decode(encoded)
        .map_err(|error| format!("The image is not valid base64: {error}"))?;
    let (width, height) = png_dimensions(&png).ok_or("The image is not a PNG")?;
    if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
        return Err(format!("The image size {width}x{height} is not allowed"));
    }
    Ok(png)
}

/// Puts the card, a PNG, on the clipboard as an image.
#[tauri::command]
#[specta::specta]
pub async fn copy_insights_card(app: AppHandle, png: String) -> Result<(), String> {
    let image = tauri::async_runtime::spawn_blocking(move || {
        let png = checked_png(&png)?;
        Image::from_bytes(&png).map_err(|error| format!("Cannot read the image: {error}"))
    })
    .await
    .map_err(|error| format!("Image task failed: {error}"))??;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    let clipboard_app = app.clone();
    app.run_on_main_thread(move || {
        let _ = sender.send(clipboard_app.clipboard().write_image(&image));
    })
    .map_err(|error| format!("Cannot reach the clipboard: {error}"))?;
    receiver
        .await
        .map_err(|_| "The clipboard write did not run".to_string())?
        .map_err(|error| format!("Cannot copy the image: {error}"))
}

/// Asks where to save the card, a PNG, and writes it there. Returns the
/// path, or `None` when the user cancels.
#[tauri::command]
#[specta::specta]
pub async fn save_insights_card(app: AppHandle, png: String) -> Result<Option<String>, String> {
    let png = tauri::async_runtime::spawn_blocking(move || checked_png(&png))
        .await
        .map_err(|error| format!("Image task failed: {error}"))??;

    let (sender, receiver) = tokio::sync::oneshot::channel::<Option<PathBuf>>();
    app.run_on_main_thread(move || {
        let _ = sender.send(save_panel::ask_save_path(DEFAULT_FILE_NAME));
    })
    .map_err(|error| format!("Cannot show the save dialog: {error}"))?;
    let Some(path) = receiver
        .await
        .map_err(|_| "The save dialog closed unexpectedly".to_string())?
    else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&path, png)
            .map(|()| Some(path.display().to_string()))
            .map_err(|error| format!("Cannot save the image: {error}"))
    })
    .await
    .map_err(|error| format!("Save task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1x1 transparent PNG.
    const ONE_PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    fn png_with_size(width: u32, height: u32) -> Vec<u8> {
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png
    }

    #[test]
    fn a_real_png_passes_and_decodes() {
        let png = checked_png(ONE_PIXEL).unwrap();
        assert_eq!(png_dimensions(&png), Some((1, 1)));
        let image = Image::from_bytes(&png).unwrap();
        assert_eq!((image.width(), image.height()), (1, 1));
    }

    #[test]
    fn the_declared_size_is_read_from_the_header() {
        assert_eq!(
            png_dimensions(&png_with_size(1080, 1080)),
            Some((1080, 1080))
        );
    }

    #[test]
    fn bytes_that_are_not_a_png_are_refused() {
        let text = STANDARD.encode("not a picture, however long it is........");
        assert_eq!(checked_png(&text).unwrap_err(), "The image is not a PNG");
        assert!(checked_png("***").unwrap_err().contains("base64"));
        assert!(checked_png("").is_err());
    }

    #[test]
    fn a_png_header_without_ihdr_is_refused() {
        let mut png = png_with_size(10, 10);
        png[12..16].copy_from_slice(b"IDAT");
        assert_eq!(png_dimensions(&png), None);
    }

    #[test]
    fn sizes_outside_the_limits_are_refused() {
        for (width, height) in [(0, 10), (10, 0), (MAX_SIDE + 1, 10), (10, u32::MAX)] {
            let encoded = STANDARD.encode(png_with_size(width, height));
            assert!(
                checked_png(&encoded).unwrap_err().contains("not allowed"),
                "{width}x{height}"
            );
        }
        assert!(checked_png(&STANDARD.encode(png_with_size(MAX_SIDE, MAX_SIDE))).is_ok());
    }

    #[test]
    fn an_oversized_payload_is_refused_before_decoding() {
        let encoded = "A".repeat(MAX_PNG_BYTES.div_ceil(3) * 4 + 4);
        assert_eq!(checked_png(&encoded).unwrap_err(), "The image is too large");
    }
}
