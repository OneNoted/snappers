/// Configurable color themes for the overlay UI.

/// An RGBA color with components in the 0.0–1.0 range.
#[derive(Debug, Clone, Copy)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Rgba {
    /// Set this color as the current Cairo source.
    pub fn set_source(&self, cr: &cairo::Context) {
        cr.set_source_rgba(self.r, self.g, self.b, self.a);
    }

    /// Convert to `[f32; 4]` for wgpu solid instances.
    pub fn as_f32_array(&self) -> [f32; 4] {
        [self.r as f32, self.g as f32, self.b as f32, self.a as f32]
    }

    /// Format as `#RRGGBB` hex for use in Pango markup attributes.
    pub fn to_hex_rgb(&self) -> String {
        format!(
            "#{:02X}{:02X}{:02X}",
            (self.r * 255.0).round() as u8,
            (self.g * 255.0).round() as u8,
            (self.b * 255.0).round() as u8,
        )
    }
}

/// Create an opaque color from 0–255 RGB components.
const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    Rgba {
        r: r as f64 / 255.0,
        g: g as f64 / 255.0,
        b: b as f64 / 255.0,
        a: 1.0,
    }
}

/// Create a color from 0–255 RGB components with explicit alpha.
const fn rgba(r: u8, g: u8, b: u8, a: f64) -> Rgba {
    Rgba {
        r: r as f64 / 255.0,
        g: g as f64 / 255.0,
        b: b as f64 / 255.0,
        a,
    }
}

/// All colors consumed by the overlay UI.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Panel background.
    pub panel_bg: Rgba,
    /// Panel border (1px rounded-rect stroke).
    pub panel_border: Rgba,
    /// Primary body text.
    pub text: Rgba,
    /// Secondary / dimmed text.
    pub text_dim: Rgba,
    /// Keyboard-shortcut badge background.
    pub badge_bg: Rgba,
    /// Keyboard-shortcut badge text.
    pub badge_text: Rgba,
    /// Accent: selection border, capture button, highlights.
    pub accent: Rgba,
    /// Dim mask applied to non-selected screen areas.
    pub dim_mask: Rgba,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            panel_bg: rgb(0x22, 0x22, 0x33),
            panel_border: rgb(0x44, 0x44, 0x55),
            text: rgb(0xE0, 0xE4, 0xF0),
            text_dim: rgb(0x99, 0x9D, 0xAA),
            badge_bg: rgb(0x33, 0x33, 0x44),
            badge_text: rgb(0x7A, 0xA2, 0xF7),
            accent: rgb(0x7A, 0xA2, 0xF7),
            dim_mask: rgba(0, 0, 0, 0.5),
        }
    }
}

impl Theme {
    pub fn catppuccin_mocha() -> Self {
        Self {
            panel_bg: rgb(0x18, 0x18, 0x25),      // Mantle
            panel_border: rgb(0x45, 0x47, 0x5A),   // Surface1
            text: rgb(0xBA, 0xC2, 0xDE),           // Subtext1
            text_dim: rgb(0xA6, 0xAD, 0xC8),       // Subtext0
            badge_bg: rgb(0x31, 0x32, 0x44),        // Surface0
            badge_text: rgb(0xB4, 0xBE, 0xFE),     // Lavender
            accent: rgb(0xB4, 0xBE, 0xFE),         // Lavender
            dim_mask: rgba(0x11, 0x11, 0x1B, 0.5), // Crust
        }
    }

    pub fn catppuccin_macchiato() -> Self {
        Self {
            panel_bg: rgb(0x1E, 0x20, 0x30),      // Mantle
            panel_border: rgb(0x49, 0x4D, 0x64),   // Surface1
            text: rgb(0xB8, 0xC0, 0xE0),           // Subtext1
            text_dim: rgb(0xA5, 0xAD, 0xCB),       // Subtext0
            badge_bg: rgb(0x36, 0x3A, 0x4F),        // Surface0
            badge_text: rgb(0xB7, 0xBD, 0xF8),     // Lavender
            accent: rgb(0xB7, 0xBD, 0xF8),         // Lavender
            dim_mask: rgba(0x18, 0x19, 0x26, 0.5), // Crust
        }
    }

    pub fn catppuccin_frappe() -> Self {
        Self {
            panel_bg: rgb(0x29, 0x2C, 0x3C),      // Mantle
            panel_border: rgb(0x51, 0x57, 0x6D),   // Surface1
            text: rgb(0xB5, 0xBF, 0xE2),           // Subtext1
            text_dim: rgb(0xA5, 0xAD, 0xCE),       // Subtext0
            badge_bg: rgb(0x41, 0x45, 0x59),        // Surface0
            badge_text: rgb(0xBA, 0xBB, 0xF1),     // Lavender
            accent: rgb(0xBA, 0xBB, 0xF1),         // Lavender
            dim_mask: rgba(0x23, 0x26, 0x34, 0.5), // Crust
        }
    }

    pub fn catppuccin_latte() -> Self {
        Self {
            panel_bg: rgb(0xE6, 0xE9, 0xEF),      // Mantle
            panel_border: rgb(0xBC, 0xC0, 0xCC),   // Surface1
            text: rgb(0x5C, 0x5F, 0x77),           // Subtext1
            text_dim: rgb(0x6C, 0x6F, 0x85),       // Subtext0
            badge_bg: rgb(0xCC, 0xD0, 0xDA),        // Surface0
            badge_text: rgb(0x72, 0x87, 0xFD),     // Lavender
            accent: rgb(0x72, 0x87, 0xFD),         // Lavender
            dim_mask: rgba(0xFF, 0xFF, 0xFF, 0.4), // Light wash
        }
    }
}

pub fn resolve_theme(name: Option<&str>, flavor: Option<&str>) -> anyhow::Result<Theme> {
    match name.unwrap_or("default") {
        "default" => Ok(Theme::default()),
        "catppuccin" => match flavor.unwrap_or("mocha") {
            "mocha" => Ok(Theme::catppuccin_mocha()),
            "macchiato" => Ok(Theme::catppuccin_macchiato()),
            "frappe" => Ok(Theme::catppuccin_frappe()),
            "latte" => Ok(Theme::catppuccin_latte()),
            other => anyhow::bail!("unknown catppuccin flavor `{other}`"),
        },
        other => anyhow::bail!("unknown theme `{other}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_has_opaque_accent() {
        let theme = Theme::default();
        assert_eq!(theme.accent.a, 1.0);
    }

    #[test]
    fn hex_rgb_round_trips() {
        let color = rgb(0xB4, 0xBE, 0xFE);
        assert_eq!(color.to_hex_rgb(), "#B4BEFE");
    }

    #[test]
    fn resolve_default_theme() {
        let theme = resolve_theme(None, None).unwrap();
        assert_eq!(theme.dim_mask.a, 0.5);
    }

    #[test]
    fn resolve_catppuccin_flavors() {
        for flavor in ["mocha", "macchiato", "frappe", "latte"] {
            resolve_theme(Some("catppuccin"), Some(flavor))
                .unwrap_or_else(|_| panic!("flavor {flavor} should resolve"));
        }
    }

    #[test]
    fn resolve_unknown_theme_fails() {
        assert!(resolve_theme(Some("nope"), None).is_err());
    }

    #[test]
    fn resolve_unknown_flavor_fails() {
        assert!(resolve_theme(Some("catppuccin"), Some("nope")).is_err());
    }
}
