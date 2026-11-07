// A Windows release build is a GUI app: without this, a console window opens
// beside it.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use clap::Parser;
use murmur::cli::CliCommand;
use murmur::CliArgs;

fn main() {
    // Joins the terminal's console for the commands that print.
    #[cfg(target_os = "windows")]
    murmur::console_windows::attach_for_cli();

    let cli_args = CliArgs::parse();

    if let Some(CliCommand::Mcp) = cli_args.command {
        std::process::exit(murmur::mcp::run());
    }

    murmur::run(cli_args)
}
