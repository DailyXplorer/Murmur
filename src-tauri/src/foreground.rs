//! The app in front on Windows: the foreground top-level window and the
//! process that owns it.

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForegroundWindow {
    /// The window handle, as an integer so it can cross threads.
    pub window: isize,
    pub pid: u32,
}

/// The foreground window, unless there is none (the desktop is switching,
/// or the secure desktop is up).
pub fn window() -> Option<ForegroundWindow> {
    // SAFETY: both calls take no pointers but the local `pid`.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return None;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid != 0).then_some(ForegroundWindow {
        window: hwnd.0 as isize,
        pid,
    })
}

/// A process handle that closes itself. Limited query access is granted for
/// any process, elevated ones included.
struct Process(HANDLE);

impl Process {
    fn open(pid: u32) -> Option<Self> {
        // SAFETY: plain call; the handle is owned by the returned value.
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .ok()
            .map(Self)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcess and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// When the process started, in FILETIME ticks. Together with the pid it
/// names one process: a pid can be reused, a pid and start time cannot.
pub fn process_start_time(pid: u32) -> Option<u64> {
    let process = Process::open(pid)?;
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: every out pointer is a live local FILETIME.
    unsafe { GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user) }.ok()?;
    Some((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

/// The full path of the process's executable.
pub fn process_image_path(pid: u32) -> Option<String> {
    let process = Process::open(pid)?;
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    // SAFETY: `buffer` holds `length` UTF-16 units, and Windows writes at
    // most that many, reporting the count back in `length`.
    unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .ok()?;
    String::from_utf16(&buffer[..length as usize]).ok()
}

/// The lower-cased file name of an executable path, such as `chrome.exe`:
/// what names an app on Windows, as a bundle identifier does on macOS.
pub fn executable_name(path: &str) -> Option<String> {
    let name = path.rsplit(['\\', '/']).next()?.trim();
    (!name.is_empty()).then(|| name.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_executable_is_named_by_its_lower_cased_file_name() {
        assert_eq!(
            executable_name(r"C:\Program Files\Google\Chrome\Application\chrome.exe").as_deref(),
            Some("chrome.exe")
        );
        assert_eq!(
            executable_name(r"C:\Users\someone\AppData\Local\Programs\Microsoft VS Code\Code.exe")
                .as_deref(),
            Some("code.exe")
        );
        assert_eq!(
            executable_name("WINWORD.EXE").as_deref(),
            Some("winword.exe")
        );
        assert_eq!(executable_name(r"C:\folder\"), None);
        assert_eq!(executable_name(""), None);
    }

    #[test]
    fn this_process_has_a_start_time_and_an_executable() {
        let pid = std::process::id();
        assert!(process_start_time(pid).is_some());
        let path = process_image_path(pid).expect("own image path");
        assert!(executable_name(&path).is_some_and(|name| name.ends_with(".exe")));
    }
}
