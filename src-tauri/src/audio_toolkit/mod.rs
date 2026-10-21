pub mod audio;
pub mod constants;
pub mod gain;
// The local engine cuts long dictations with it; it runs nothing yet.
#[cfg_attr(not(test), allow(dead_code))]
pub mod pauses;
pub mod text;

pub use audio::{
    is_microphone_access_denied, is_no_input_device_error, list_input_devices, list_output_devices,
    read_wav_samples, save_wav_file, verify_wav_file, AudioRecorder, CpalDeviceInfo,
};
pub use text::{apply_custom_words, apply_custom_words_counted, normalize_transcription_output};
