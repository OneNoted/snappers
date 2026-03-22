use std::f64::consts::{PI, TAU};

use anyhow::Result;
use cairo::{Context, Format, ImageSurface};
use pango::{Alignment, FontDescription};

use crate::geometry::{Point, Rect, Size};
use crate::theme::Theme;

pub const SELECTION_BORDER: i32 = 3;
pub const HANDLE_SIZE: i32 = 6;
const PADDING: i32 = 8;
const RADIUS: i32 = 16;
const BORDER: f64 = 1.0;
const CORNER_RADIUS: f64 = 8.0;
const FONT: &str = "sans 14px";

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

pub fn build_panel_assets(theme: &Theme) -> Result<PanelAssets> {
    Ok(PanelAssets {
        show_pointer: render_panel(&panel_markup("show", theme), theme)?,
        hide_pointer: render_panel(&panel_markup("hide", theme), theme)?,
    })
}

fn panel_markup(pointer_verb: &str, theme: &Theme) -> String {
    let bg = theme.badge_bg.to_hex_rgb();
    let fg = theme.badge_text.to_hex_rgb();
    format!(
        "Press <span face='mono' bgcolor='{bg}' fgcolor='{fg}'> Space </span> to save the screenshot.\n\
         Press <span face='mono' bgcolor='{bg}' fgcolor='{fg}'> P </span> to {pointer_verb} the pointer."
    )
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
    theme: &Theme,
) -> Result<()> {
    theme.dim_mask.set_source(cr);
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

        theme.accent.set_source(cr);
        cr.set_line_width(SELECTION_BORDER as f64);
        cr.rectangle(
            rect.x as f64 - SELECTION_BORDER as f64 / 2.0,
            rect.y as f64 - SELECTION_BORDER as f64 / 2.0,
            rect.width as f64 + SELECTION_BORDER as f64,
            rect.height as f64 + SELECTION_BORDER as f64,
        );
        cr.stroke()?;

        let hs = HANDLE_SIZE as f64;
        let half = hs / 2.0;
        for (hx, hy) in [
            (rect.x as f64, rect.y as f64),
            ((rect.x + rect.width) as f64, rect.y as f64),
            (rect.x as f64, (rect.y + rect.height) as f64),
            ((rect.x + rect.width) as f64, (rect.y + rect.height) as f64),
        ] {
            cr.rectangle(hx - half, hy - half, hs, hs);
            cr.fill()?;
        }
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

fn render_panel(text: &str, theme: &Theme) -> Result<PixelSurface> {
    let mut font = FontDescription::from_string(FONT);
    font.set_absolute_size((14 * pango::SCALE) as f64);

    let (width, height) = {
        let surface = ImageSurface::create(Format::ARgb32, 0, 0)?;
        let cr = Context::new(&surface)?;
        let layout = pangocairo::functions::create_layout(&cr);
        layout.context().set_round_glyph_positions(false);
        layout.set_font_description(Some(&font));
        layout.set_alignment(Alignment::Left);
        layout.set_markup(text);
        layout.set_spacing(2 * 1024);

        let (mut width, mut height) = layout.pixel_size();
        width += PADDING + RADIUS * 2 + PADDING + PADDING;
        height = height.max(RADIUS * 2);
        height += PADDING * 2;
        (width, height)
    };

    let surface = ImageSurface::create(Format::ARgb32, width, height)?;
    {
        let cr = Context::new(&surface)?;

        // Rounded-rect background
        let inset = BORDER / 2.0;
        rounded_rect(&cr, inset, inset, width as f64 - BORDER, height as f64 - BORDER, CORNER_RADIUS);
        cr.save()?;
        cr.clip_preserve();
        theme.panel_bg.set_source(&cr);
        cr.paint()?;
        cr.restore()?;

        // Border stroke along the same path
        theme.panel_border.set_source(&cr);
        cr.set_line_width(BORDER);
        cr.stroke()?;

        // Capture button — single accent-filled circle
        let yc = f64::from(height / 2);
        let r = f64::from(RADIUS);
        cr.new_sub_path();
        cr.arc(f64::from(PADDING) + r, yc, r - 3.0, 0.0, TAU);
        theme.accent.set_source(&cr);
        cr.fill()?;

        // Panel text
        cr.move_to(
            f64::from(PADDING + RADIUS * 2 + PADDING),
            f64::from(PADDING),
        );
        let layout = pangocairo::functions::create_layout(&cr);
        layout.context().set_round_glyph_positions(false);
        layout.set_font_description(Some(&font));
        layout.set_alignment(Alignment::Left);
        layout.set_markup(text);
        layout.set_spacing(2 * 1024);

        theme.text.set_source(&cr);
        pangocairo::functions::show_layout(&cr, &layout);
    }

    let data = surface.take_data()?;
    Ok(PixelSurface {
        width,
        height,
        data: data.to_vec(),
    })
}

fn rounded_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.arc(x + w - r, y + r, r, 1.5 * PI, 2.0 * PI);
    cr.arc(x + w - r, y + h - r, r, 0.0, 0.5 * PI);
    cr.arc(x + r, y + h - r, r, 0.5 * PI, PI);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_panel_assets_renders_pixel_buffers() {
        let theme = Theme::default();
        let assets = build_panel_assets(&theme).expect("panel assets should render");
        assert!(assets.show_pointer.width > 0);
        assert!(assets.show_pointer.height > 0);
        assert!(!assets.show_pointer.data.is_empty());
        assert!(assets.hide_pointer.width > 0);
        assert!(assets.hide_pointer.height > 0);
        assert!(!assets.hide_pointer.data.is_empty());
    }

    #[test]
    fn final_overlay_surface_can_be_mapped_after_painting() {
        let theme = Theme::default();
        let output_size = Size::new(800, 600);
        let mut background = PixelSurface {
            width: 2,
            height: 2,
            data: vec![255; 2 * 2 * 4],
        };
        let assets = build_panel_assets(&theme).expect("panel assets should render");
        let mut panel = assets.show_pointer.clone();
        let mut surface =
            ImageSurface::create(Format::ARgb32, output_size.width, output_size.height)
                .expect("surface should allocate");

        {
            let cr = Context::new(&surface).expect("context should create");
            paint_background(&cr, &mut background, output_size).expect("background should paint");
            paint_masks_and_border(
                &cr,
                output_size,
                Some(Rect::new(100, 120, 200, 160)),
                &theme,
            )
            .expect("mask should paint");
            paint_panel(&cr, &mut panel, output_size, false).expect("panel should paint");
        }

        let data = surface
            .data()
            .expect("surface data should be accessible after painting");
        assert!(!data.is_empty());
    }
}
