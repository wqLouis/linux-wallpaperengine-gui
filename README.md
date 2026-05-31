# linux-wallpaperengine-gui

A modern GUI manager for [linux-wallpaperengine](https://github.com/0xFA11/linux-wallpaperengine) — browse, apply, and manage animated wallpapers from Steam Workshop with a sleek dark-themed interface.

![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)

## Features

- **Library browser** — discover and preview all your Wallpaper Engine wallpapers (both Steam Workshop and built-in)
- **One-click apply** — launch scene wallpapers via `linux-wallpaper-engine` or video wallpapers via `mpvpaper`
- **System tray** — runs as a background daemon with a tray icon for quick access
- **Full settings** — configure all engine parameters (output mode, fit mode, log level, target FPS, effects, mpv options)
- **Dark theme** — custom-styled interface with accent colors, card shadows, and modern typography

## Architecture

```
┌──────────────────────────────┐     ┌──────────────────────────────┐
│        TRAY PROCESS          │     │        GUI PROCESS           │
│  (default mode)              │     │     (--gui mode)             │
│                              │     │                              │
│  • ksni tray icon (DBus SNI) │     │  • iced GUI                  │
│  • Wallpaper process manager │◄───►│  • Wallpaper discovery       │
│  • Config reader             │ IPC │  • Settings UI               │
│  • IPC server (Unix socket)  │     │  • Config writer             │
└──────────────────────────────┘     └──────────────────────────────┘
```

- **Tray daemon** owns the wallpaper lifecycle — only one wallpaper at a time, killed on switch or quit
- **GUI** reads/writes `config.toml` directly, discovers wallpapers locally, and sends commands to the tray via Unix socket
- Works on Wayland/Hyprland — uses DBus SNI for the tray icon (no GTK dependency)

## Requirements

- [linux-wallpaperengine](https://github.com/0xFA11/linux-wallpaperengine) — for scene wallpapers
- [mpvpaper](https://github.com/GhostNaN/mpvpaper) — for video wallpapers (optional)
- A Wayland compositor with `status-notifier` support (e.g. Hyprland, Sway) or X11 for the tray icon

## Installation

### From source

```bash
git clone https://github.com/wqLouis/linux-wallpaperengine-gui.git
cd linux-wallpaperengine-gui
cargo build --release
```

The binary will be at `target/release/linux-wallpaperengine-gui`.

### Dependencies (Linux)

```bash
# Arch / Manjaro
pacman -S gtk3 libappindicator-gtk3

# Debian / Ubuntu
apt install libgtk-3-dev libappindicator3-dev
```

## Usage

```bash
# Start the tray daemon (also spawns the GUI automatically)
./linux-wallpaperengine-gui

# Run just the GUI (if tray is already running)
./linux-wallpaperengine-gui --gui
```

Set the log level via environment:
```bash
RUST_LOG=debug ./linux-wallpaperengine-gui
```

## Configuration

Settings are stored at `~/.config/linux-wallpaperengine-gui/config.toml`:

```toml
steamapps_path = "/home/user/.steam/steam/steamapps"

[engine]
mode = "wlr"
fit_mode = "cover"
log_level = "warning"
target_fps = 60
no_effects = false

[mpvpaper]
output = "*"
mpv_options = ["loop"]
```

Configured via the Settings tab in the GUI.

## License

MIT OR Apache-2.0
