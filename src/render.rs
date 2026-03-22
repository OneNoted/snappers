use std::f64::consts::TAU;

use anyhow::Result;
use cairo::{Context, Format, ImageSurface};
use pango::{Alignment, FontDescription};

use crate::geometry::{Point, Rect, Size};

pub const SELECTION_BORDER: i32 = 2;
const PADDING: i32 = 8;
const RADIUS: i32 = 16;
const BORDER: i32 = 4;
const FONT: &str = "sans 14px";

const TEXT_HIDE_P: &str = "Press <span face='mono' bgcolor='#2C2C2C'> Space </span> to save the screenshot.\n\
     Press <span face='mono' bgcolor='#2C2C2C'> P </span> to hide the pointer.";
const TEXT_SHOW_P: &str = "Press <span face='mono' bgcolor='#2C2C2C'> Space </span> to save the screenshot.\n\
     Press <span face='mono' bgcolor='#2C2C2C'> P </span> to show the pointer.";

#[derive(Debug, Clone)]
pub struct PixelSurface {
    pub width: i32,
    pub height: i32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PanelAssets {
    pub show_pointer: PixelSurface,
    pub hide_pointer: PixelSurface,
}

impl PixelSurface {
    pub fn from_rgba_image(image: &image::DynamicImage) -> Self {
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for pixel in rgba.pixels() {
            let [r, g, b, a] = pixel.0;
            let alpha = a as u32;
            let premul = |channel: u8| ((channel as u32 * alpha + 127) / 255) as u8;
            data.push(premul(b));
            data.push(premul(g));
            data.push(premul(r));
            data.push(a);
        }

        Self {
            width: width as i32,
            height: height as i32,
            data,
        }
    }
}

pub fn build_panel_assets() -> Result<PanelAssets> {
    Ok(PanelAssets {
        show_pointer: render_panel(TEXT_SHOW_P)?,
        hide_pointer: render_panel(TEXT_HIDE_P)?,
    })
}

pub fn paint_background(cr: &Context, surface: &mut PixelSurface, target_size: Size) -> Result<()> {
    let mut image = ImageSurface::create(Format::ARgb32, surface.width, surface.height)?;
    {
        let mut data = image.data()?;
        data.copy_from_slice(&surface.data);
    }
    cr.save()?;
    cr.scale(
        target_size.width as f64 / surface.width as f64,
        target_size.height as f64 / surface.height as f64,
    );
    cr.set_source_surface(&image, 0.0, 0.0)?;
    cr.paint()?;
    cr.restore()?;
    Ok(())
}

pub fn paint_masks_and_border(
    cr: &Context,
    output_size: Size,
    selection: Option<Rect>,
) -> Result<()> {
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.5);
    if let Some(rect) = selection {
        cr.rectangle(0.0, 0.0, output_size.width as f64, rect.y as f64);
        cr.fill()?;
        cr.rectangle(
            0.0,
            (rect.y + rect.height) as f64,
            output_size.width as f64,
            (output_size.height - rect.y - rect.height) as f64,
        );
        cr.fill()?;
        cr.rectangle(0.0, rect.y as f64, rect.x as f64, rect.height as f64);
        cr.fill()?;
        cr.rectangle(
            (rect.x + rect.width) as f64,
            rect.y as f64,
            (output_size.width - rect.x - rect.width) as f64,
            rect.height as f64,
        );
        cr.fill()?;

        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_line_width(SELECTION_BORDER as f64);
        cr.rectangle(
            rect.x as f64 - SELECTION_BORDER as f64 / 2.0,
            rect.y as f64 - SELECTION_BORDER as f64 / 2.0,
            rect.width as f64 + SELECTION_BORDER as f64,
            rect.height as f64 + SELECTION_BORDER as f64,
        );
        cr.stroke()?;
    } else {
        cr.rectangle(
            0.0,
            0.0,
            output_size.width as f64,
            output_size.height as f64,
        );
        cr.fill()?;
    }
    Ok(())
}

pub fn paint_panel(
    cr: &Context,
    panel: &mut PixelSurface,
    output_size: Size,
    dragging_selection: bool,
) -> Result<Rect> {
    let location = panel_location(output_size, Size::new(panel.width, panel.height));
    let alpha = if dragging_selection { 0.3 } else { 0.9 };

    let mut surface = ImageSurface::create(Format::ARgb32, panel.width, panel.height)?;
    {
        let mut data = surface.data()?;
        data.copy_from_slice(&panel.data);
    }
    cr.save()?;
    cr.set_source_surface(&surface, location.x as f64, location.y as f64)?;
    cr.paint_with_alpha(alpha)?;
    cr.restore()?;

    Ok(Rect::new(location.x, location.y, panel.width, panel.height))
}

pub fn panel_location(output_size: Size, panel_size: Size) -> Point {
    let x = ((output_size.width - panel_size.width) / 2).max(0);
    let y = (output_size.height - panel_size.height - PADDING * 2).max(0);
    Point::new(x, y)
}

pub fn capture_button_hit(panel_rect: Rect, point: Point) -> bool {
    let radius = RADIUS - 2;
    let xc = panel_rect.x + PADDING + radius;
    let yc = panel_rect.y + panel_rect.height / 2;
    let dx = point.x - xc;
    let dy = point.y - yc;
    dx * dx + dy * dy <= radius * radius
}

fn render_panel(text: &str) -> Result<PixelSurface> {
    let surface = ImageSurface::create(Format::ARgb32, 0, 0)?;
    let cr = Context::new(&surface)?;

    let mut font = FontDescription::from_string(FONT);
    font.set_absolute_size((14 * pango::SCALE) as f64);

    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    layout.set_alignment(Alignment::Left);
    layout.set_markup(text);
    layout.set_spacing(2 * 1024);

    let (mut width, mut height) = layout.pixel_size();
    width += PADDING + RADIUS * 2 + PADDING - BORDER / 2 + PADDING;
    height = height.max(RADIUS * 2);
    height += PADDING * 2;

    let surface = ImageSurface::create(Format::ARgb32, width, height)?;
    let cr = Context::new(&surface)?;
    cr.set_source_rgb(0.1, 0.1, 0.1);
    cr.paint()?;

    let yc = f64::from(height / 2);
    let r = f64::from(RADIUS);

    cr.new_sub_path();
    cr.arc(f64::from(PADDING) + r, yc, r, 0.0, TAU);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.fill()?;

    cr.new_sub_path();
    cr.arc(f64::from(PADDING) + r, yc, r - 2.0, 0.0, TAU);
    cr.set_source_rgb(0.1, 0.1, 0.1);
    cr.fill()?;

    cr.new_sub_path();
    cr.arc(f64::from(PADDING) + r, yc, r - 4.0, 0.0, TAU);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.fill()?;

    cr.move_to(
        f64::from(PADDING + RADIUS * 2 + PADDING - BORDER / 2),
        f64::from(PADDING),
    );
    let layout = pangocairo::functions::create_layout(&cr);
    layout.context().set_round_glyph_positions(false);
    layout.set_font_description(Some(&font));
    layout.set_alignment(Alignment::Left);
    layout.set_markup(text);
    layout.set_spacing(2 * 1024);

    cr.set_source_rgb(1.0, 1.0, 1.0);
    pangocairo::functions::show_layout(&cr, &layout);

    cr.rectangle(0.0, 0.0, width as f64, height as f64);
    cr.set_source_rgb(0.3, 0.3, 0.3);
    cr.set_line_width(BORDER as f64);
    cr.stroke()?;

    let data = surface.take_data()?;
    Ok(PixelSurface {
        width,
        height,
        data: data.to_vec(),
    })
}
