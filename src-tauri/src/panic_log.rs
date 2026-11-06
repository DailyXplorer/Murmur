//! Every panic, on any thread, becomes one `error` record in `murmur.log`:
//! the thread, where it panicked and a backtrace. The panic message is left
//! out, since it can quote dictated text.

use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;

/// Call once the log plugin is registered. The previous hook still runs.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!(target: "panic", "{}", record(info));
        previous(info);
    }));
}

fn record(info: &PanicHookInfo<'_>) -> String {
    let thread = std::thread::current();
    let location = info.location().map_or_else(
        || "an unknown location".to_owned(),
        |location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        },
    );
    // Release builds are stripped: the frames are addresses, which
    // `atos -o murmur.dSYM -l <load address> <address>…` turns back into
    // lines, with the dSYM build.yml uploads for that build.
    format!(
        "thread '{}' panicked at {location} (load address {:#x})\n{:#}",
        thread.name().unwrap_or("unnamed"),
        load_address(),
        Backtrace::force_capture()
    )
}

/// Where the executable's image starts in memory.
#[cfg(target_os = "macos")]
fn load_address() -> usize {
    unsafe extern "C" {
        fn _dyld_get_image_header(image_index: u32) -> *const std::ffi::c_void;
    }
    // SAFETY: dyld always has image 0, the main executable.
    unsafe { _dyld_get_image_header(0) as usize }
}

/// Where the executable's image starts in memory: its module handle.
#[cfg(target_os = "windows")]
fn load_address() -> usize {
    use windows::core::PCWSTR;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    // SAFETY: a null name asks for the module of the calling process.
    unsafe { GetModuleHandleW(PCWSTR::null()) }.map_or(0, |module| module.0 as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Capture(Mutex<Vec<String>>);

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }

        fn log(&self, record: &log::Record<'_>) {
            if record.target() == "panic" {
                self.0.lock().unwrap().push(record.args().to_string());
            }
        }

        fn flush(&self) {}
    }

    static CAPTURED: Capture = Capture(Mutex::new(Vec::new()));

    #[test]
    fn panic_record_has_location_without_payload() {
        log::set_logger(&CAPTURED).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
        install();

        let dictated = String::from("some dictated words");
        let joined = std::thread::Builder::new()
            .name("panic-log-test".into())
            .spawn(move || panic!("{dictated}"))
            .unwrap()
            .join();
        drop(std::panic::take_hook());

        assert!(joined.is_err());
        let records = CAPTURED.0.lock().unwrap();
        let record = records
            .iter()
            .find(|record| record.contains("thread 'panic-log-test'"))
            .expect("one record for the panic");
        assert!(record.contains("panic_log.rs:"), "{record}");
        assert!(record.contains("load address 0x"), "{record}");
        assert!(!record.contains("some dictated words"), "{record}");
    }
}
