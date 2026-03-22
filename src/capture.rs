use anyhow::{Context, Result, bail};
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
        if let Some(name) = name {
            let output = self
                .conn
                .get_all_outputs()
                .iter()
                .find(|output| output.name == name)
                .with_context(|| format!("unknown output `{name}`"))?;

            return self
                .conn
                .screenshot_single_output(output, show_pointer)
                .with_context(|| format!("failed to capture output {}", output.name));
        }

        let snapshot = self.snapshot()?;
        let output = detect_output_under_pointer(&snapshot.outputs).with_context(|| {
            format!(
                "failed to determine the current monitor from the pointer; pass `--output` explicitly. Known outputs: {}",
                self.describe_outputs()
            )
        })?;

        Ok(screenshot_variant(output, show_pointer))
    }

    pub fn describe_outputs(&self) -> String {
        self.conn
            .get_all_outputs()
            .iter()
            .map(|output| {
                let position = output.logical_position();
                let size = output.logical_size();
                format!(
                    "{} @ ({}, {}) {}x{}",
                    output.name, position.x, position.y, size.width, size.height
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub fn encode_png(image: &DynamicImage) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
        .context("failed to encode screenshot as png")?;
    Ok(bytes)
}

fn detect_output_under_pointer(outputs: &[CaptureOutput]) -> Result<&CaptureOutput> {
    let changed_outputs = outputs
        .iter()
        .filter(|output| output_changed_when_pointer_toggled(output))
        .collect::<Vec<_>>();

    match changed_outputs.as_slice() {
        [output] => Ok(*output),
        [] => bail!(
            "could not determine which monitor contains the pointer because all output captures were identical; pass `--output` explicitly"
        ),
        outputs => {
            let names = outputs
                .iter()
                .map(|output| output.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "could not determine which monitor contains the pointer because multiple outputs changed when toggling the cursor: {names}; pass `--output` explicitly"
            )
        }
    }
}

fn output_changed_when_pointer_toggled(output: &CaptureOutput) -> bool {
    images_differ(
        &output.screenshot_with_pointer,
        &output.screenshot_without_pointer,
    )
}

fn images_differ(with_pointer: &DynamicImage, without_pointer: &DynamicImage) -> bool {
    let with_pointer = with_pointer.to_rgba8();
    let without_pointer = without_pointer.to_rgba8();

    with_pointer.dimensions() != without_pointer.dimensions()
        || with_pointer.as_raw() != without_pointer.as_raw()
}

fn screenshot_variant(output: &CaptureOutput, show_pointer: bool) -> DynamicImage {
    if show_pointer {
        output.screenshot_with_pointer.clone()
    } else {
        output.screenshot_without_pointer.clone()
    }
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
