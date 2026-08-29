use clap::Parser;
use murmur::CliArgs;

fn main() {
    let cli_args = CliArgs::parse();

    murmur::run(cli_args)
}
