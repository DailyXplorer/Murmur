//! A console for `murmur.exe` run by hand in a terminal.
//!
//! The release build is a GUI program (`windows_subsystem = "windows"`), so
//! Windows gives it no console: `murmur.exe --help`, `--transcribe-file` and
//! `murmur.exe mcp` printed nothing when typed in cmd or PowerShell. The fix
//! is to join the console of the terminal that started it, and only for the
//! commands that print and end there. Starting the app itself never does, so
//! closing the terminal that launched it cannot close the app.
//!
//! A program started by an AI assistant has pipes for standard input and
//! output. They are valid handles and stay untouched: only a handle that is
//! missing is replaced by the console's.

use clap::Parser;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, GENERIC_READ, GENERIC_WRITE, HANDLE, NO_ERROR};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetFileType, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_TYPE_UNKNOWN, OPEN_EXISTING,
};
use windows::Win32::System::Console::{
    AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_HANDLE,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};

use crate::cli::{CliArgs, CliCommand};

/// Gives the process the console of its terminal when the command line asks
/// for a command that prints, or is wrong, or asks for help. The caller parses
/// the command line again afterwards; that is where help and errors print.
pub fn attach_for_cli() {
    let prints = match CliArgs::try_parse() {
        Ok(cli_args) => prints_to_terminal(&cli_args),
        // `--help`, `--version` and a mistake in the arguments all print,
        // then end the process.
        Err(_) => true,
    };
    if prints {
        attach_parent_console();
    }
}

/// Whether the command talks through the terminal and then ends, which the
/// app itself (window, tray, shortcuts) does not.
fn prints_to_terminal(cli_args: &CliArgs) -> bool {
    matches!(cli_args.command, Some(CliCommand::Mcp)) || cli_args.transcribe_file.is_some()
}

/// Which of the three standard handles the process lacks.
#[derive(Debug, PartialEq, Eq)]
struct Missing {
    input: bool,
    output: bool,
    error: bool,
}

impl Missing {
    fn any(&self) -> bool {
        self.input || self.output || self.error
    }
}

fn missing(usable: impl Fn(STD_HANDLE) -> bool) -> Missing {
    Missing {
        input: !usable(STD_INPUT_HANDLE),
        output: !usable(STD_OUTPUT_HANDLE),
        error: !usable(STD_ERROR_HANDLE),
    }
}

/// Joins the console of the parent process and gives it to each standard
/// handle that has none. Does nothing when the handles are all usable (pipes
/// or files from a parent that redirected them), or when the parent has no
/// console (started from Explorer, a shortcut or a service).
fn attach_parent_console() {
    let lacking = missing(is_usable);
    if !lacking.any() {
        return;
    }
    // SAFETY: no pointer is involved. It fails, harmlessly, without a parent
    // console.
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_err() {
        return;
    }
    for (needed, which, name) in [
        (lacking.input, STD_INPUT_HANDLE, w!("CONIN$")),
        (lacking.output, STD_OUTPUT_HANDLE, w!("CONOUT$")),
        (lacking.error, STD_ERROR_HANDLE, w!("CONOUT$")),
    ] {
        if !needed {
            continue;
        }
        if let Some(handle) = open_console(name) {
            // SAFETY: `handle` is open and stays open for the life of the
            // process, which is what a standard handle is.
            let _ = unsafe { SetStdHandle(which, handle) };
        }
    }
}

/// Whether the process has a working handle for `which`.
fn is_usable(which: STD_HANDLE) -> bool {
    // SAFETY: both calls only read the handle table of this process.
    unsafe {
        let Ok(handle) = GetStdHandle(which) else {
            return false;
        };
        if handle.is_invalid() || handle.0.is_null() {
            return false;
        }
        // An unknown type with no error is a valid handle of a kind
        // Windows does not name; with an error, the handle is closed.
        GetFileType(handle) != FILE_TYPE_UNKNOWN || GetLastError() == NO_ERROR
    }
}

/// Opens the console's input (`CONIN$`) or screen buffer (`CONOUT$`).
fn open_console(name: PCWSTR) -> Option<HANDLE> {
    // SAFETY: `name` is a static NUL-terminated string.
    unsafe {
        CreateFileW(
            name,
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> CliArgs {
        CliArgs::try_parse_from(std::iter::once("murmur").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn the_commands_that_print_and_end_get_a_console() {
        assert!(prints_to_terminal(&parse(&["mcp"])));
        assert!(prints_to_terminal(&parse(&["--transcribe-file", "a.wav"])));
        assert!(prints_to_terminal(&parse(&["-f", "a.wav", "--json"])));
    }

    #[test]
    fn starting_the_app_never_joins_the_terminal() {
        for args in [
            &[][..],
            &["--start-hidden"],
            &["--no-tray"],
            &["--debug"],
            &["--toggle-transcription"],
            &["--cancel"],
        ] {
            assert!(!prints_to_terminal(&parse(args)), "{args:?}");
        }
    }

    #[test]
    fn only_a_missing_handle_is_replaced() {
        let none_missing = missing(|_| true);
        assert!(!none_missing.any());

        // An assistant that pipes input and output but not the error stream.
        let only_error = missing(|which| which != STD_ERROR_HANDLE);
        assert_eq!(
            only_error,
            Missing {
                input: false,
                output: false,
                error: true
            }
        );

        // Started from a terminal by hand: nothing was inherited.
        let all = missing(|_| false);
        assert!(all.input && all.output && all.error);
    }
}
