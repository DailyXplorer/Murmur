use clap::Parser;
use murmur_app_lib::CliArgs;

fn main() {
    let cli_args = CliArgs::parse();

    murmur_app_lib::run(cli_args)
}
