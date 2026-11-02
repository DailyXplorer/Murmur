use clap::Parser;
use murmur::cli::CliCommand;
use murmur::CliArgs;

fn main() {
    let cli_args = CliArgs::parse();

    if let Some(CliCommand::Mcp) = cli_args.command {
        std::process::exit(murmur::mcp::run());
    }

    murmur::run(cli_args)
}
