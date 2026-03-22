mod capture;
mod cli;
mod config;
mod geometry;
mod overlay;
mod overlay_renderer;
mod render;
mod save;
mod state;

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::{
    capture::CaptureBackend,
    cli::{Cli, Command},
    config::{AppConfig, config_path},
    overlay::select_region,
    save::persist_capture,
};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .without_time()
        .init();

    let cli = Cli::parse();
    let config = AppConfig::load()?;

    match cli.command {
        Command::ConfigPath => {
            println!("{}", config_path()?.display());
            Ok(())
        }
        Command::Screen(options) => {
            let backend = CaptureBackend::new()?;
            let image = backend
                .screenshot_output(options.output.as_deref(), options.capture.show_pointer)?;
            let path = if options.capture.write_to_disk {
                config.resolve_output_path(options.capture.path.as_deref())?
            } else {
                None
            };
            persist_capture(&image, path, options.capture.write_to_disk)?;
            Ok(())
        }
        Command::Area(options) => {
            let backend = CaptureBackend::new()?;
            let snapshot = backend.snapshot()?;
            let selection = select_region(snapshot, config.clone(), options.show_pointer)?;

            let Some(selection) = selection else {
                return Ok(());
            };

            let image = backend
                .screenshot_region(selection.region, selection.show_pointer)
                .context("failed to capture the selected region")?;
            let write_to_disk = options.write_to_disk && selection.write_to_disk;
            let path = if write_to_disk {
                config.resolve_output_path(options.path.as_deref())?
            } else {
                None
            };
            persist_capture(&image, path, write_to_disk)?;
            Ok(())
        }
    }
}
