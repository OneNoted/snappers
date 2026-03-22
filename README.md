# snappers

`snappers` is a standalone Wayland screenshot tool that aims to reproduce niri's built-in screenshot UI as a normal client application.

It currently provides:

- `snappers area` for the interactive region picker
- `snappers screen` for whole-output capture
- clipboard copy plus optional file saving
- output-aware `screen` capture that defaults to the monitor under the pointer

## Requirements

`snappers` targets compositors that expose the wlroots-style screenshot stack:

- `wlr-layer-shell`
- screencopy support exposed through `libwayshot`
- a working Wayland clipboard path

## Build

```bash
cargo build --release
```

## Commands

```bash
snappers area
snappers screen
snappers screen --output DP-1
snappers config-path
```

Notes:

- `area` opens the niri-style selection overlay.
- `screen` captures the output under the pointer by default.
- `screen --output <name>` bypasses auto-selection and captures the named output.
- captures are copied to the clipboard; saving to disk is enabled by default and can be disabled with `--write-to-disk=false`.

## Configuration

The default config path is:

```text
~/.config/snappers/config.toml
```

The default screenshot path pattern is:

```text
~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png
```

Example config:

```toml
screenshot_path = "~/Pictures/Screenshots/Snappers %Y-%m-%d %H-%M-%S.png"

[keymap]
confirm = ["Return", "space"]
copy_only = ["Ctrl+C"]
cancel = ["Escape"]
toggle_pointer = ["p"]
```

If `screen` cannot determine the output under the pointer, it fails clearly and asks for `--output`.

## Development

Useful checks:

```bash
cargo test --quiet
cargo check --quiet
```
