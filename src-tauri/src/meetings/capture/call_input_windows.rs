//! The input devices the call apps record from on Windows, read from the
//! audio sessions of every active capture endpoint: an app that records has
//! an active session on the endpoint it records from, with its process id.
//! `call_input.rs` decides what that means. The same read happens once a
//! second while a meeting records, so it only lists and reads; it opens no
//! stream and changes no setting.

use std::collections::BTreeMap;

use windows::core::{Interface, GUID};
use windows::Win32::Foundation::{CloseHandle, PROPERTYKEY};
use windows::Win32::Media::Audio::{
    eCapture, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2, IMMDevice,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CLSCTX_ALL, STGM_READ};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

use super::call_input::{app_of, Proc, ProcessInput};
use super::wasapi::{enumerator, property_string, Com};

/// `PKEY_Device_FriendlyName`: the name cpal gives the endpoint.
const FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

/// The processes running now, by id. Taken only when a session of another
/// process is active.
fn processes() -> Option<BTreeMap<u32, Proc>> {
    // SAFETY: plain call; the handle is closed below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
    let mut table = BTreeMap::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: `entry` is a live local with its size set, and each call
    // writes only into it.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        let length = entry
            .szExeFile
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(entry.szExeFile.len());
        table.insert(
            entry.th32ProcessID,
            Proc {
                parent: entry.th32ParentProcessID,
                exe: String::from_utf16_lossy(&entry.szExeFile[..length]).to_lowercase(),
            },
        );
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: the handle came from CreateToolhelp32Snapshot.
    let _ = unsafe { CloseHandle(snapshot) };
    Some(table)
}

/// The processes with an active session on `device`, a capture endpoint.
fn active_processes(device: &IMMDevice) -> Vec<u32> {
    let mut pids = Vec::new();
    // SAFETY: plain calls on live COM interfaces; COM is initialised on this
    // thread by `sessions`.
    unsafe {
        let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
            return pids;
        };
        let Ok(sessions) = manager.GetSessionEnumerator() else {
            return pids;
        };
        let count = sessions.GetCount().unwrap_or(0);
        for index in 0..count {
            let Ok(control) = sessions.GetSession(index) else {
                continue;
            };
            if control.GetState().ok() != Some(AudioSessionStateActive) {
                continue;
            }
            let Ok(control) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            // 0: a session several processes share, whose owner Windows
            // does not name.
            if let Ok(pid) = control.GetProcessId() {
                if pid != 0 {
                    pids.push(pid);
                }
            }
        }
    }
    pids
}

/// The capture endpoints with an active session of a process other than
/// Murmur, each with that process: `None` where Windows would not list the
/// endpoints.
fn sessions() -> Option<Vec<(String, u32)>> {
    let own = std::process::id();
    let _com = Com::enter();
    let enumerator = enumerator().ok()?;
    // SAFETY: plain calls on live COM interfaces.
    let devices = unsafe { enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) }.ok()?;
    let count = unsafe { devices.GetCount() }.ok()?;
    let mut found = Vec::new();
    for index in 0..count {
        let Ok(device) = (unsafe { devices.Item(index) }) else {
            continue;
        };
        let pids: Vec<u32> = active_processes(&device)
            .into_iter()
            .filter(|pid| *pid != own)
            .collect();
        if pids.is_empty() {
            continue;
        }
        let Ok(store) = (unsafe { device.OpenPropertyStore(STGM_READ) }) else {
            continue;
        };
        let Some(name) = property_string(&store, &FRIENDLY_NAME) else {
            continue;
        };
        found.extend(pids.into_iter().map(|pid| (name.clone(), pid)));
    }
    Some(found)
}

/// The input device each call app on the microphone records from now.
pub fn inputs() -> Option<Vec<ProcessInput>> {
    let sessions = sessions()?;
    if sessions.is_empty() {
        return Some(Vec::new());
    }
    let table = processes()?;
    Some(
        sessions
            .into_iter()
            .filter_map(|(device, pid)| {
                Some(ProcessInput {
                    app_id: app_of(pid, &table)?,
                    device,
                })
            })
            .collect(),
    )
}
