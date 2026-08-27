// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use murmur_app_lib::CliArgs;

fn main() {
    let cli_args = CliArgs::parse();

    murmur_app_lib::run(cli_args)
}
