use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::DynamicImage;
use notify_rust::Notification;
use wl_clipboard_rs::copy::{MimeType, Options, Source};

use crate::capture::encode_png;

pub fn copy_png_to_clipboard(bytes: Vec<u8>) -> Result<()> {
    Options::new()
        .copy(
            Source::Bytes(bytes.into()),
            MimeType::Specific("image/png".into()),
        )
        .context("failed to copy screenshot to the clipboard")
}

pub fn save_png(bytes: &[u8], path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }

    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

pub fn persist_capture(
    image: &DynamicImage,
    path: Option<PathBuf>,
    write_to_disk: bool,
) -> Result<Option<PathBuf>> {
    let png = encode_png(image)?;
    copy_png_to_clipboard(png.clone())?;

    let saved = if write_to_disk {
        if let Some(path) = path {
            save_png(&png, &path)?;
            Some(path)
        } else {
            None
        }
    } else {
        None
    };

    let _ = show_notification(saved.as_deref());
    Ok(saved)
}

fn show_notification(path: Option<&Path>) -> Result<()> {
    let mut notification = Notification::new();
    notification
        .summary("Screenshot captured")
        .body("You can paste the image from the clipboard.");

    if let Some(path) = path {
        notification.hint(notify_rust::Hint::ImagePath(
            path.to_string_lossy().into_owned(),
        ));
    }

    notification
        .show()
        .context("failed to send screenshot notification")?;
    Ok(())
}
