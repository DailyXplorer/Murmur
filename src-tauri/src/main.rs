// A Windows release build is a GUI app: without this, a console window opens
// beside it.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use clap::Parser;
use murmur::CliArgs;

fn main() {
    let cli_args = CliArgs::parse();

    murmur::run(cli_args)
}
