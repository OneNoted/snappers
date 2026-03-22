use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, bail};
use wl_clipboard_rs::copy::{MimeType, Options, Source};

const READY_SIGNAL: &str = "ready";

pub fn copy_png_to_clipboard(bytes: &[u8]) -> Result<()> {
    let exe = env::current_exe()
        .context("failed to resolve the snappers executable for clipboard copy")?;
    let mut child = Command::new(exe)
        .arg("clipboard-serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn the clipboard helper process")?;

    {
        let mut stdin = child
            .stdin
            .take()
            .context("clipboard helper did not accept stdin")?;
        stdin
            .write_all(bytes)
            .context("failed to send screenshot data to the clipboard helper")?;
    }

    wait_for_helper_ready(&mut child)
}

pub fn serve_png_clipboard() -> Result<()> {
    let mut bytes = Vec::new();
    io::stdin()
        .read_to_end(&mut bytes)
        .context("failed to read screenshot bytes for the clipboard helper")?;
    if bytes.is_empty() {
        bail!("clipboard helper received an empty screenshot payload");
    }

    let mut options = Options::new();
    options.foreground(true);
    let prepared = options
        .prepare_copy(
            Source::Bytes(bytes.into_boxed_slice()),
            MimeType::Specific("image/png".into()),
        )
        .context("failed to claim the Wayland clipboard for the screenshot")?;

    let mut stdout = io::stdout().lock();
    stdout
        .write_all(format!("{READY_SIGNAL}\n").as_bytes())
        .context("failed to report clipboard helper readiness")?;
    stdout
        .flush()
        .context("failed to flush clipboard helper readiness")?;

    prepared
        .serve()
        .context("clipboard helper stopped serving the screenshot")
}

fn wait_for_helper_ready(child: &mut Child) -> Result<()> {
    let stdout = child
        .stdout
        .take()
        .context("clipboard helper did not expose a readiness pipe")?;
    let mut ready_line = String::new();
    let bytes_read = BufReader::new(stdout)
        .read_line(&mut ready_line)
        .context("failed while waiting for the clipboard helper to become ready")?;

    if bytes_read == 0 {
        if let Some(status) = child
            .try_wait()
            .context("failed to inspect clipboard helper status")?
        {
            bail!("clipboard helper exited before becoming ready with status {status}");
        }
        bail!("clipboard helper exited before reporting readiness");
    }

    if ready_line.trim_end() != READY_SIGNAL {
        if let Some(status) = child
            .try_wait()
            .context("failed to inspect clipboard helper status")?
        {
            bail!(
                "clipboard helper reported `{}` before exiting with status {status}",
                ready_line.trim_end()
            );
        }
        bail!(
            "clipboard helper reported unexpected readiness payload `{}`",
            ready_line.trim_end()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ready_signal() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("printf 'ready\n'")
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn helper");
        assert!(wait_for_helper_ready(&mut child).is_ok());
    }

    #[test]
    fn rejects_unexpected_ready_signal() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("printf 'nope\n'")
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn helper");
        let err = wait_for_helper_ready(&mut child).expect_err("unexpected ready payload");
        assert!(err.to_string().contains("unexpected readiness payload"));
    }
}
