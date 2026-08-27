/// Returns the CPAL host for the current platform (`default_host()`, CoreAudio on macOS).
pub fn get_cpal_host() -> cpal::Host {
    cpal::default_host()
}
