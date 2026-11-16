use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Subcommand, Debug, Clone)]
pub enum CliCommand {
    /// Serve your meetings read-only to an AI app over stdio (Model Context
    /// Protocol). The AI app launches this; it starts no window.
    Mcp,
}

#[derive(Parser, Debug, Clone, Default)]
#[command(name = "murmur", about = "Murmur - Speech to Text")]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Option<CliCommand>,

    /// Start with the main window hidden
    #[arg(long)]
    pub start_hidden: bool,

    /// Disable the system tray icon
    #[arg(long)]
    pub no_tray: bool,

    /// Toggle transcription on/off (sent to running instance)
    #[arg(long)]
    pub toggle_transcription: bool,

    /// Cancel the current operation (sent to running instance)
    #[arg(long)]
    pub cancel: bool,

    /// Enable debug mode with verbose logging
    #[arg(long)]
    pub debug: bool,

    /// Transcribe this audio file (WAV, MP3, M4A, AIFF, CAF or FLAC, up to 15
    /// minutes) headlessly and exit. Runs the same batch transcription path as
    /// the app, without the microphone.
    #[arg(short = 'f', long, value_name = "FILE")]
    pub transcribe_file: Option<PathBuf>,

    /// Emit --transcribe-file results as JSON.
    #[arg(long)]
    pub json: bool,
}
