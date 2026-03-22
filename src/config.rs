use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Local;
use directories::{ProjectDirs, UserDirs};
use serde::Deserialize;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};

use crate::theme::Theme;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub screenshot_path: Option<String>,
    pub keymap: Keymap,
    pub theme: Theme,
}

#[derive(Debug, Clone)]
pub struct Keymap {
    pub confirm: Vec<KeyBinding>,
    pub copy_only: Vec<KeyBinding>,
    pub cancel: Vec<KeyBinding>,
    pub toggle_pointer: Vec<KeyBinding>,
    pub move_left: Vec<KeyBinding>,
    pub move_right: Vec<KeyBinding>,
    pub move_up: Vec<KeyBinding>,
    pub move_down: Vec<KeyBinding>,
    pub resize_left: Vec<KeyBinding>,
    pub resize_right: Vec<KeyBinding>,
    pub resize_up: Vec<KeyBinding>,
    pub resize_down: Vec<KeyBinding>,
    pub next_output: Vec<KeyBinding>,
    pub previous_output: Vec<KeyBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
    pub key: Keysym,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    screenshot_path: Option<Option<String>>,
    keymap: Option<FileKeymap>,
    theme: Option<FileThemeConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct FileThemeConfig {
    name: Option<String>,
    flavor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileKeymap {
    confirm: Option<Vec<String>>,
    copy_only: Option<Vec<String>>,
    cancel: Option<Vec<String>>,
    toggle_pointer: Option<Vec<String>>,
    move_left: Option<Vec<String>>,
    move_right: Option<Vec<String>>,
    move_up: Option<Vec<String>>,
    move_down: Option<Vec<String>>,
    resize_left: Option<Vec<String>>,
    resize_right: Option<Vec<String>>,
    resize_up: Option<Vec<String>>,
    resize_down: Option<Vec<String>>,
    next_output: Option<Vec<String>>,
    previous_output: Option<Vec<String>>,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config at {}", path.display()))?;
        let file: FileConfig = toml::from_str(&raw)
            .with_context(|| format!("failed to parse config at {}", path.display()))?;
        Self::from_file(file)
    }

    fn from_file(file: FileConfig) -> Result<Self> {
        let defaults = Keymap::default();
        let file_keymap = file.keymap.unwrap_or_default();
        let file_theme = file.theme.unwrap_or_default();
        let theme = crate::theme::resolve_theme(
            file_theme.name.as_deref(),
            file_theme.flavor.as_deref(),
        )?;

        Ok(Self {
            screenshot_path: file
                .screenshot_path
                .unwrap_or_else(|| Some(default_screenshot_pattern())),
            theme,
            keymap: Keymap {
                confirm: parse_bindings(file_keymap.confirm, defaults.confirm)?,
                copy_only: parse_bindings(file_keymap.copy_only, defaults.copy_only)?,
                cancel: parse_bindings(file_keymap.cancel, defaults.cancel)?,
                toggle_pointer: parse_bindings(
                    file_keymap.toggle_pointer,
                    defaults.toggle_pointer,
                )?,
                move_left: parse_bindings(file_keymap.move_left, defaults.move_left)?,
                move_right: parse_bindings(file_keymap.move_right, defaults.move_right)?,
                move_up: parse_bindings(file_keymap.move_up, defaults.move_up)?,
                move_down: parse_bindings(file_keymap.move_down, defaults.move_down)?,
                resize_left: parse_bindings(file_keymap.resize_left, defaults.resize_left)?,
                resize_right: parse_bindings(file_keymap.resize_right, defaults.resize_right)?,
                resize_up: parse_bindings(file_keymap.resize_up, defaults.resize_up)?,
                resize_down: parse_bindings(file_keymap.resize_down, defaults.resize_down)?,
                next_output: parse_bindings(file_keymap.next_output, defaults.next_output)?,
                previous_output: parse_bindings(
                    file_keymap.previous_output,
                    defaults.previous_output,
                )?,
            },
        })
    }

    pub fn resolve_output_path(&self, override_path: Option<&Path>) -> Result<Option<PathBuf>> {
        if let Some(path) = override_path {
            return Ok(Some(path.to_path_buf()));
        }

        let Some(pattern) = &self.screenshot_path else {
            return Ok(None);
        };

        let expanded = expand_tilde(pattern);
        let formatted = Local::now().format(&expanded).to_string();
        Ok(Some(PathBuf::from(formatted)))
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            screenshot_path: Some(default_screenshot_pattern()),
            keymap: Keymap::default(),
            theme: Theme::default(),
        }
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            confirm: vec![binding("Return"), binding("space")],
            copy_only: vec![binding("Ctrl+C")],
            cancel: vec![binding("Escape")],
            toggle_pointer: vec![binding("p")],
            move_left: vec![binding("Left")],
            move_right: vec![binding("Right")],
            move_up: vec![binding("Up")],
            move_down: vec![binding("Down")],
            resize_left: vec![binding("Shift+Left")],
            resize_right: vec![binding("Shift+Right")],
            resize_up: vec![binding("Shift+Up")],
            resize_down: vec![binding("Shift+Down")],
            next_output: vec![binding("Tab")],
            previous_output: vec![binding("Shift+Tab")],
        }
    }
}

impl KeyBinding {
    pub fn matches(&self, keysym: Keysym, modifiers: Modifiers) -> bool {
        self.key == keysym
            && self.ctrl == modifiers.ctrl
            && self.alt == modifiers.alt
            && self.shift == modifiers.shift
            && self.logo == modifiers.logo
    }
}

pub fn config_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("io", "github", "snappers")
        .context("could not resolve XDG project directories")?;
    Ok(dirs.config_dir().join("config.toml"))
}

pub fn default_screenshot_pattern() -> String {
    let base = UserDirs::new()
        .and_then(|dirs| dirs.picture_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("~/Pictures"));
    base.join("Screenshots")
        .join("Screenshot from %Y-%m-%d %H-%M-%S.png")
        .to_string_lossy()
        .into_owned()
}

fn parse_bindings(raw: Option<Vec<String>>, default: Vec<KeyBinding>) -> Result<Vec<KeyBinding>> {
    let Some(raw) = raw else {
        return Ok(default);
    };

    raw.into_iter()
        .map(|binding| parse_binding(&binding))
        .collect()
}

fn binding(raw: &str) -> KeyBinding {
    parse_binding(raw).expect("default binding must parse")
}

fn parse_binding(raw: &str) -> Result<KeyBinding> {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut logo = false;
    let mut key = None;

    for part in raw.split('+') {
        let token = part.trim();
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            "super" | "logo" | "meta" => logo = true,
            _ => key = Some(parse_keysym(token)?),
        }
    }

    Ok(KeyBinding {
        ctrl,
        alt,
        shift,
        logo,
        key: key.context("binding is missing a key")?,
    })
}

fn parse_keysym(raw: &str) -> Result<Keysym> {
    let lower = raw.trim().to_ascii_lowercase();
    let key = match lower.as_str() {
        "escape" | "esc" => Keysym::Escape,
        "return" | "enter" => Keysym::Return,
        "space" => Keysym::space,
        "tab" => Keysym::Tab,
        "left" => Keysym::Left,
        "right" => Keysym::Right,
        "up" => Keysym::Up,
        "down" => Keysym::Down,
        "c" => Keysym::c,
        "p" => Keysym::p,
        _ => anyhow::bail!("unsupported keysym `{raw}`"),
    };
    Ok(key)
}

fn expand_tilde(path: &str) -> String {
    if !path.starts_with("~/") {
        return path.to_owned();
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_owned());
    format!("{home}/{}", &path[2..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_binding() {
        let binding = parse_binding("Ctrl+Shift+Left").expect("binding");
        assert!(binding.ctrl);
        assert!(binding.shift);
        assert_eq!(binding.key, Keysym::Left);
    }

    #[test]
    fn expands_tilde_paths() {
        let expanded = expand_tilde("~/Pictures/example.png");
        assert!(expanded.contains("Pictures/example.png"));
    }

    #[test]
    fn default_path_uses_screenshots_directory() {
        let path = default_screenshot_pattern();
        assert!(path.contains("Screenshots"));
        assert!(path.contains("%Y-%m-%d"));
    }
}
