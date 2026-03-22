use anyhow::{Context, Result};
use image::{DynamicImage, ImageFormat};
use libwayshot::{
    WayshotConnection,
    region::{LogicalRegion, Position, Region, Size as ShotSize},
};

use crate::geometry::Rect;

#[derive(Debug, Clone)]
pub struct CaptureOutput {
    pub name: String,
    pub logical_rect: Rect,
    pub screenshot_with_pointer: DynamicImage,
    pub screenshot_without_pointer: DynamicImage,
}

#[derive(Debug, Clone)]
pub struct CaptureSnapshot {
    pub outputs: Vec<CaptureOutput>,
}

pub struct CaptureBackend {
    conn: WayshotConnection,
}

impl CaptureBackend {
    pub fn new() -> Result<Self> {
        let conn = WayshotConnection::new().context(
            "failed to connect to the capture backend; this compositor likely does not expose wlroots screenshot protocols",
        )?;
        Ok(Self { conn })
    }

    pub fn snapshot(&self) -> Result<CaptureSnapshot> {
        let mut outputs = Vec::new();
        for output in self.conn.get_all_outputs() {
            let logical_position = output.logical_position();
            let logical_size = output.logical_size();
            let with_pointer = self
                .conn
                .screenshot_single_output(output, true)
                .with_context(|| format!("failed to capture output {}", output.name))?;
            let without_pointer = self
                .conn
                .screenshot_single_output(output, false)
                .with_context(|| format!("failed to capture output {}", output.name))?;

            outputs.push(CaptureOutput {
                name: output.name.clone(),
                logical_rect: Rect::new(
                    logical_position.x,
                    logical_position.y,
                    logical_size.width as i32,
                    logical_size.height as i32,
                ),
                screenshot_with_pointer: with_pointer,
                screenshot_without_pointer: without_pointer,
            });
        }

        if outputs.is_empty() {
            anyhow::bail!("no outputs are available for capture");
        }

        Ok(CaptureSnapshot { outputs })
    }

    pub fn screenshot_region(&self, region: Rect, show_pointer: bool) -> Result<DynamicImage> {
        self.conn
            .screenshot(region_to_logical(region), show_pointer)
            .context("failed to capture screenshot region")
    }

    pub fn screenshot_output(
        &self,
        name: Option<&str>,
        show_pointer: bool,
    ) -> Result<DynamicImage> {
        let output = if let Some(name) = name {
            self.conn
                .get_all_outputs()
                .iter()
                .find(|output| output.name == name)
                .with_context(|| format!("unknown output `{name}`"))?
        } else {
            self.conn
                .get_all_outputs()
                .first()
                .context("no outputs are available for capture")?
        };

        self.conn
            .screenshot_single_output(output, show_pointer)
            .with_context(|| format!("failed to capture output {}", output.name))
    }
}

pub fn encode_png(image: &DynamicImage) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
        .context("failed to encode screenshot as png")?;
    Ok(bytes)
}

fn region_to_logical(rect: Rect) -> LogicalRegion {
    LogicalRegion {
        inner: Region {
            position: Position {
                x: rect.x,
                y: rect.y,
            },
            size: ShotSize {
                width: rect.width as u32,
                height: rect.height as u32,
            },
        },
    }
}
