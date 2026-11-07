use crate::settings::SoundTheme;
use crate::settings::{self, AppSettings};
use cpal::traits::{DeviceTrait, HostTrait};
use log::{debug, error, warn};
use rodio::OutputStreamBuilder;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

pub enum SoundType {
    Start,
    Stop,
}

fn resolve_sound_path(
    app: &AppHandle,
    settings: &AppSettings,
    sound_file: String,
) -> Option<PathBuf> {
    let base_dir = get_sound_base_dir(settings);
    match base_dir {
        tauri::path::BaseDirectory::AppData => app
            .path()
            .app_data_dir()
            .ok()
            .map(|directory| directory.join(sound_file)),
        _ => app.path().resolve(&sound_file, base_dir).ok(),
    }
}

fn get_sound_path(settings: &AppSettings, sound_type: SoundType) -> String {
    match (settings.sound_theme, sound_type) {
        (SoundTheme::Custom, SoundType::Start) => "custom_start.wav".to_string(),
        (SoundTheme::Custom, SoundType::Stop) => "custom_stop.wav".to_string(),
        (_, SoundType::Start) => settings.sound_theme.to_start_path(),
        (_, SoundType::Stop) => settings.sound_theme.to_stop_path(),
    }
}

fn get_sound_base_dir(settings: &AppSettings) -> tauri::path::BaseDirectory {
    match settings.sound_theme {
        SoundTheme::Custom => tauri::path::BaseDirectory::AppData,
        _ => tauri::path::BaseDirectory::Resource,
    }
}

/// The feedback sound to play, `None` while feedback is off.
fn feedback_sound_file(settings: &AppSettings, sound_type: SoundType) -> Option<String> {
    settings
        .audio_feedback
        .then(|| get_sound_path(settings, sound_type))
}

/// How long the stop sound plays, `None` while feedback is off. A file that
/// cannot be measured is taken to play for a second, which the release tail
/// treats as a sound that never ends.
pub fn stop_sound_length(app: &AppHandle, settings: &AppSettings) -> Option<Duration> {
    let sound_file = feedback_sound_file(settings, SoundType::Stop)?;
    let length = resolve_sound_path(app, settings, sound_file)
        .and_then(|path| wav_length(&path))
        .unwrap_or(UNKNOWN_SOUND_LENGTH);
    Some(length)
}

const UNKNOWN_SOUND_LENGTH: Duration = Duration::from_secs(1);

fn wav_length(path: &Path) -> Option<Duration> {
    reader_length(&hound::WavReader::open(path).ok()?)
}

fn reader_length<R: std::io::Read>(reader: &hound::WavReader<R>) -> Option<Duration> {
    let rate = reader.spec().sample_rate;
    // `duration` counts the samples of one channel.
    (rate > 0).then(|| Duration::from_secs_f64(f64::from(reader.duration()) / f64::from(rate)))
}

pub fn play_feedback_sound(app: &AppHandle, sound_type: SoundType) {
    let settings = settings::get_settings(app);
    let Some(sound_file) = feedback_sound_file(&settings, sound_type) else {
        return;
    };
    if let Some(path) = resolve_sound_path(app, &settings, sound_file) {
        play_sound_async(app, path);
    }
}

pub fn play_feedback_sound_blocking(app: &AppHandle, sound_type: SoundType) {
    let settings = settings::get_settings(app);
    let Some(sound_file) = feedback_sound_file(&settings, sound_type) else {
        return;
    };
    if let Some(path) = resolve_sound_path(app, &settings, sound_file) {
        play_sound_blocking(app, &path);
    }
}

pub fn play_test_sound(app: &AppHandle, sound_type: SoundType) {
    let settings = settings::get_settings(app);
    let sound_file = get_sound_path(&settings, sound_type);
    if let Some(path) = resolve_sound_path(app, &settings, sound_file) {
        play_sound_blocking(app, &path);
    }
}

fn play_sound_async(app: &AppHandle, path: PathBuf) {
    let app_handle = app.clone();
    thread::spawn(move || {
        if let Err(e) = play_sound_at_path(&app_handle, path.as_path()) {
            error!("Failed to play sound '{}': {}", path.display(), e);
        }
    });
}

fn play_sound_blocking(app: &AppHandle, path: &Path) {
    if let Err(e) = play_sound_at_path(app, path) {
        error!("Failed to play sound '{}': {}", path.display(), e);
    }
}

fn play_sound_at_path(app: &AppHandle, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let settings = settings::get_settings(app);
    let volume = settings.audio_feedback_volume;
    let selected_device = settings.selected_output_device.clone();
    play_audio_file(path, selected_device, volume)
}

fn play_audio_file(
    path: &std::path::Path,
    selected_device: Option<String>,
    volume: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let stream_builder = if let Some(device_name) = selected_device {
        if device_name == "Default" {
            debug!("Using default device");
            OutputStreamBuilder::from_default_device()?
        } else {
            let host = cpal::default_host();
            let devices = host.output_devices()?;

            let mut found_device = None;
            for device in devices {
                if device.name()? == device_name {
                    found_device = Some(device);
                    break;
                }
            }

            match found_device {
                Some(device) => OutputStreamBuilder::from_device(device)?,
                None => {
                    warn!("Device '{}' not found, using default device", device_name);
                    OutputStreamBuilder::from_default_device()?
                }
            }
        }
    } else {
        debug!("Using default device");
        OutputStreamBuilder::from_default_device()?
    };

    let stream_handle = stream_builder.open_stream()?;
    let mixer = stream_handle.mixer();

    let file = File::open(path)?;
    let buf_reader = BufReader::new(file);

    let sink = rodio::play(mixer, buf_reader)?;
    sink.set_volume(volume);
    sink.sleep_until_end();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        feedback_sound_file, get_sound_base_dir, get_sound_path, reader_length, SoundType,
    };
    use crate::settings::{get_default_settings, AppSettings, SoundTheme};
    use std::time::Duration;
    use tauri::path::BaseDirectory;

    fn themed(theme: SoundTheme) -> AppSettings {
        let mut settings = get_default_settings();
        settings.sound_theme = theme;
        settings
    }

    #[test]
    fn built_in_themes_play_their_bundled_resources() {
        for (theme, name) in [(SoundTheme::Marimba, "marimba"), (SoundTheme::Pop, "pop")] {
            let settings = themed(theme);
            assert_eq!(
                get_sound_path(&settings, SoundType::Start),
                format!("resources/{name}_start.wav")
            );
            assert_eq!(
                get_sound_path(&settings, SoundType::Stop),
                format!("resources/{name}_stop.wav")
            );
            assert!(
                matches!(get_sound_base_dir(&settings), BaseDirectory::Resource),
                "{name}"
            );
        }
    }

    #[test]
    fn the_custom_theme_plays_its_files_from_app_data() {
        let settings = themed(SoundTheme::Custom);
        assert_eq!(
            get_sound_path(&settings, SoundType::Start),
            "custom_start.wav"
        );
        assert_eq!(
            get_sound_path(&settings, SoundType::Stop),
            "custom_stop.wav"
        );
        assert!(matches!(
            get_sound_base_dir(&settings),
            BaseDirectory::AppData
        ));
    }

    #[test]
    fn feedback_off_plays_nothing() {
        let mut settings = themed(SoundTheme::Marimba);
        settings.audio_feedback = false;
        assert!(feedback_sound_file(&settings, SoundType::Start).is_none());
        assert!(feedback_sound_file(&settings, SoundType::Stop).is_none());

        settings.audio_feedback = true;
        assert_eq!(
            feedback_sound_file(&settings, SoundType::Start).as_deref(),
            Some("resources/marimba_start.wav")
        );
        assert_eq!(
            feedback_sound_file(&settings, SoundType::Stop).as_deref(),
            Some("resources/marimba_stop.wav")
        );
    }

    #[test]
    fn a_sound_is_measured_in_time_not_in_samples() {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut bytes = Vec::new();
        {
            let mut writer = hound::WavWriter::new(std::io::Cursor::new(&mut bytes), spec).unwrap();
            // Half a second of stereo.
            for _ in 0..22_050 * 2 {
                writer.write_sample(0i16).unwrap();
            }
            writer.finalize().unwrap();
        }
        let reader = hound::WavReader::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(reader_length(&reader), Some(Duration::from_millis(500)));
    }
}
