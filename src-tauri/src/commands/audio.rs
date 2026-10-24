use crate::audio_feedback;
use crate::audio_toolkit::audio::{list_input_devices, list_output_devices, AudioRecorder};
use crate::managers::audio::{AudioRecordingManager, MicrophoneMode};
use crate::settings::update_settings;
use log::warn;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

#[derive(Serialize, Type)]
pub struct CustomSounds {
    start: bool,
    stop: bool,
}

fn custom_sound_exists(app: &AppHandle, sound_type: &str) -> bool {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join(format!("custom_{sound_type}.wav")))
        .is_ok_and(|path| path.exists())
}

#[tauri::command]
#[specta::specta]
pub fn check_custom_sounds(app: AppHandle) -> CustomSounds {
    CustomSounds {
        start: custom_sound_exists(&app, "start"),
        stop: custom_sound_exists(&app, "stop"),
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct AudioDevice {
    pub index: String,
    pub name: String,
    pub is_default: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn update_microphone_mode(app: AppHandle, always_on: bool) -> Result<(), String> {
    // `update_mode` commits the mode setting and emits `settings-changed` only
    // after the runtime transition succeeds. It can stop/start the cpal stream
    // and takes the manager std mutexes, so run it on a blocking thread rather
    // than the webview/main run loop.
    let rm = app.state::<Arc<AudioRecordingManager>>().inner().clone();
    let new_mode = if always_on {
        MicrophoneMode::AlwaysOn
    } else {
        MicrophoneMode::OnDemand
    };

    tokio::task::spawn_blocking(move || rm.update_mode(new_mode))
        .await
        .map_err(|e| format!("audio task join failed: {}", e))?
        .map_err(|e| format!("Failed to update microphone mode: {}", e))
}

#[tauri::command]
#[specta::specta]
pub async fn get_available_microphones() -> Result<Vec<AudioDevice>, String> {
    // cpal device enumeration can stall — run it off the webview/main run loop.
    tokio::task::spawn_blocking(|| {
        let devices =
            list_input_devices().map_err(|e| format!("Failed to list audio devices: {}", e))?;

        let mut result = vec![AudioDevice {
            index: "default".to_string(),
            name: "Default".to_string(),
            is_default: true,
        }];

        result.extend(devices.into_iter().map(|d| AudioDevice {
            index: d.index,
            name: d.name,
            is_default: false,
        }));

        Ok::<_, String>(result)
    })
    .await
    .map_err(|e| format!("audio task join failed: {}", e))?
}

/// The `"default"` entry clears the choice and follows the system device.
fn chosen_device(device_name: String) -> Option<String> {
    if device_name == "default" {
        None
    } else {
        Some(device_name)
    }
}

#[tauri::command]
#[specta::specta]
pub async fn set_selected_microphone(app: AppHandle, device_name: String) -> Result<(), String> {
    let selected_microphone = chosen_device(device_name);

    // The manager holds the recording-state lock across the CoreAudio restart
    // and only persists a setting after the runtime switch succeeds. Run that
    // blocking work off the webview/main run loop.
    let rm = app.state::<Arc<AudioRecordingManager>>().inner().clone();
    tokio::task::spawn_blocking(move || rm.update_selected_device(selected_microphone))
        .await
        .map_err(|e| format!("audio task join failed: {}", e))?
        .map_err(|e| format!("Failed to update selected device: {}", e))
}

#[tauri::command]
#[specta::specta]
pub async fn get_available_output_devices() -> Result<Vec<AudioDevice>, String> {
    // cpal device enumeration can stall — run it off the webview/main run loop.
    tokio::task::spawn_blocking(|| {
        let devices =
            list_output_devices().map_err(|e| format!("Failed to list output devices: {}", e))?;

        let mut result = vec![AudioDevice {
            index: "default".to_string(),
            name: "Default".to_string(),
            is_default: true,
        }];

        result.extend(devices.into_iter().map(|d| AudioDevice {
            index: d.index,
            name: d.name,
            is_default: false,
        }));

        Ok::<_, String>(result)
    })
    .await
    .map_err(|e| format!("audio task join failed: {}", e))?
}

#[tauri::command]
#[specta::specta]
pub fn set_selected_output_device(app: AppHandle, device_name: String) -> Result<(), String> {
    update_settings(&app, |settings| {
        settings.selected_output_device = chosen_device(device_name);
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn play_test_sound(app: AppHandle, sound_type: String) {
    let Some(sound) = test_sound(&sound_type) else {
        warn!("Unknown sound type: {}", sound_type);
        return;
    };
    audio_feedback::play_test_sound(&app, sound);
}

fn test_sound(sound_type: &str) -> Option<audio_feedback::SoundType> {
    match sound_type {
        "start" => Some(audio_feedback::SoundType::Start),
        "stop" => Some(audio_feedback::SoundType::Stop),
        _ => None,
    }
}

#[tauri::command]
#[specta::specta]
pub fn set_clamshell_microphone(app: AppHandle, device_name: String) -> Result<(), String> {
    update_settings(&app, |settings| {
        settings.clamshell_microphone = chosen_device(device_name);
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_microphone_channels(device_name: String) -> Result<u16, String> {
    // cpal device enumeration and config queries can stall, so keep them off
    // the webview/main run loop.
    tokio::task::spawn_blocking(move || {
        use cpal::traits::HostTrait;

        let device = if device_name.eq_ignore_ascii_case("default") {
            cpal::default_host().default_input_device()
        } else {
            list_input_devices()
                .map_err(|e| format!("Failed to list audio devices: {e}"))?
                .into_iter()
                .find(|device| device.name == device_name)
                .map(|device| device.device)
        };

        match device {
            Some(device) => AudioRecorder::preferred_input_channel_count(&device)
                .map_err(|e| format!("Failed to get microphone config: {e}")),
            None => Ok(1),
        }
    })
    .await
    .map_err(|e| format!("audio task join failed: {e}"))?
}

#[tauri::command]
#[specta::specta]
pub async fn set_selected_channel(app: AppHandle, channel: Option<u16>) -> Result<(), String> {
    // Restarting cpal can block, so keep it off the webview/main run loop. The
    // manager persists only after it has applied the runtime change.
    let manager = app.state::<Arc<AudioRecordingManager>>().inner().clone();
    tokio::task::spawn_blocking(move || manager.update_selected_channel(channel))
        .await
        .map_err(|e| format!("audio task join failed: {e}"))?
        .map_err(|e| format!("Failed to update channel selection: {e}"))
}

#[cfg(test)]
mod tests {
    use super::{chosen_device, test_sound};
    use crate::audio_feedback::SoundType;

    #[test]
    fn the_default_entry_clears_the_device_and_a_name_is_kept() {
        assert_eq!(chosen_device("default".to_string()), None);
        assert_eq!(
            chosen_device("MacBook Pro Microphone".to_string()).as_deref(),
            Some("MacBook Pro Microphone")
        );
    }

    #[test]
    fn only_start_and_stop_are_test_sounds() {
        assert!(matches!(test_sound("start"), Some(SoundType::Start)));
        assert!(matches!(test_sound("stop"), Some(SoundType::Stop)));
        for unknown in ["beep", ""] {
            assert!(test_sound(unknown).is_none(), "{unknown:?}");
        }
    }
}
