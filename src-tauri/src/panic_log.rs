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
    // lines, with the dSYM build.yml uploads for that build. On Windows they
    // need this image base and the matching build's PDB.
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
    // SAFETY: NULL selects the main executable, whose module stays mapped
    // for the process lifetime. Do not panic again if the query fails.
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
        const CHILD: &str = "MURMUR_PANIC_LOG_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // The logger and panic hook are process-global. Keep them out of
            // other tests even under the ordinary cargo test runner.
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "panic_log::tests::panic_record_has_location_without_payload",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            return;
        }
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
        assert_ne!(load_address(), 0);
        assert!(
            record.contains(&format!("load address {:#x}", load_address())),
            "{record}"
        );
        assert_eq!(records.len(), 1);
        assert!(!record.contains("some dictated words"), "{record}");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn image_base_points_to_the_mapped_executable() {
        let base = load_address();
        assert_ne!(base, 0);
        // SAFETY: GetModuleHandleW(NULL) is the mapped executable's base,
        // whose DOS header remains readable for the process lifetime.
        let signature = unsafe { (base as *const u16).read() };
        assert_eq!(signature, 0x5a4d, "the PE image starts with MZ");
    }
}
