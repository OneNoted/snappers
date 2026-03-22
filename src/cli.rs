use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "snappers",
    version,
    about = "Standalone niri-style screenshot tool for wlroots compositors"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the interactive region selection UI.
    Area(CaptureOptions),
    /// Capture a whole output directly.
    Screen(ScreenOptions),
    /// Print the resolved config path.
    ConfigPath,
}

#[derive(Debug, Clone, Args)]
pub struct CaptureOptions {
    /// Save to the configured screenshot path after copying to the clipboard.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub write_to_disk: bool,
    /// Include the pointer in the initial screenshot.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub show_pointer: bool,
    /// Override the output path for this capture.
    #[arg(long)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct ScreenOptions {
    #[command(flatten)]
    pub capture: CaptureOptions,
    /// Capture a specific output by name.
    #[arg(long)]
    pub output: Option<String>,
}
