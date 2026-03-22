use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::DynamicImage;
use notify_rust::Notification;
use tracing::warn;

use crate::{capture::encode_png, clipboard::copy_png_to_clipboard};

#[derive(Debug, Clone)]
pub struct PersistOutcome {
    pub saved_path: Option<PathBuf>,
    pub copied_to_clipboard: bool,
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
) -> Result<PersistOutcome> {
    let png = encode_png(image)?;
    let outcome = persist_png(png, path, write_to_disk, copy_png_to_clipboard)?;
    let _ = show_notification(&outcome);
    Ok(outcome)
}

fn persist_png(
    png: Vec<u8>,
    path: Option<PathBuf>,
    write_to_disk: bool,
    copy_to_clipboard: impl FnOnce(&[u8]) -> Result<()>,
) -> Result<PersistOutcome> {
    let clipboard_error = copy_to_clipboard(&png).err();
    let saved_path = if write_to_disk {
        if let Some(path) = path {
            save_png(&png, &path)?;
            Some(path)
        } else {
            None
        }
    } else {
        None
    };

    let copied_to_clipboard = clipboard_error.is_none();

    if let Some(err) = clipboard_error {
        if saved_path.is_some() {
            warn!("failed to copy screenshot to the clipboard: {err:#}");
        } else {
            return Err(err);
        }
    }

    Ok(PersistOutcome {
        saved_path,
        copied_to_clipboard,
    })
}

fn show_notification(outcome: &PersistOutcome) -> Result<()> {
    let mut notification = Notification::new();
    notification
        .summary("Screenshot captured")
        .body(notification_body(outcome));

    if let Some(path) = outcome.saved_path.as_deref() {
        notification.hint(notify_rust::Hint::ImagePath(
            path.to_string_lossy().into_owned(),
        ));
    }

    notification
        .show()
        .context("failed to send screenshot notification")?;
    Ok(())
}

fn notification_body(outcome: &PersistOutcome) -> &'static str {
    match (outcome.saved_path.is_some(), outcome.copied_to_clipboard) {
        (true, true) => "Saved the screenshot and copied it to the clipboard.",
        (true, false) => "Saved the screenshot, but copying it to the clipboard failed.",
        (false, true) => "You can paste the image from the clipboard.",
        (false, false) => "Clipboard copy failed.",
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn unique_test_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("snappers-{name}-{unique}.png"))
    }

    #[test]
    fn clipboard_failure_is_non_fatal_when_file_is_saved() {
        let path = unique_test_path("saved");
        let outcome = persist_png(vec![1, 2, 3], Some(path.clone()), true, |_| {
            anyhow::bail!("clipboard unavailable")
        })
        .expect("save should succeed");

        assert_eq!(outcome.saved_path.as_deref(), Some(path.as_path()));
        assert!(!outcome.copied_to_clipboard);
        assert!(path.exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn clipboard_failure_is_fatal_without_saved_file() {
        let err = persist_png(vec![1, 2, 3], None, false, |_| {
            anyhow::bail!("clipboard unavailable")
        })
        .expect_err("copy-only clipboard failure should bubble up");

        assert!(err.to_string().contains("clipboard unavailable"));
    }

    #[test]
    fn requested_save_failure_still_errors() {
        let path = unique_test_path("dir");
        std::fs::create_dir_all(&path).expect("create directory path");

        let err = persist_png(vec![1, 2, 3], Some(path.clone()), true, |_| Ok(()))
            .expect_err("writing into a directory should fail");

        assert!(err.to_string().contains("failed to write"));
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn notification_body_reflects_outcome() {
        assert_eq!(
            notification_body(&PersistOutcome {
                saved_path: Some(PathBuf::from("/tmp/example.png")),
                copied_to_clipboard: true,
            }),
            "Saved the screenshot and copied it to the clipboard."
        );
        assert_eq!(
            notification_body(&PersistOutcome {
                saved_path: Some(PathBuf::from("/tmp/example.png")),
                copied_to_clipboard: false,
            }),
            "Saved the screenshot, but copying it to the clipboard failed."
        );
    }
}
