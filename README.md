# linux-wallpaperengine-gui

A modern GUI manager for [linux-wallpaperengine](https://github.com/0xFA11/linux-wallpaperengine) — browse, apply, and manage animated wallpapers from Steam Workshop with a sleek dark-themed interface.

## Features

- **Library browser** — discover and preview all your Wallpaper Engine wallpapers (both Steam Workshop and built-in)
- **One-click apply** — launch scene wallpapers via `linux-wallpaper-engine` (Rust) or `linux-wallpaperengine` (C++), or video wallpapers via `mpvpaper`
- **Engine picker** — choose between the Rust and C++ wallpaper engines in Settings, with engine-specific options shown conditionally
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
engine_rust_binary = "linux-wallpaper-engine"
engine_cpp_binary = "linux-wallpaperengine"
mpvpaper_binary = "mpvpaper"

[engine]
variant = "rust"            # "rust" (linux-wallpaper-engine) or "cpp" (linux-wallpaperengine)
mode = "wlr"                # rust only
fit_mode = "cover"          # rust only
scaling = "default"         # cpp only
screen_root = "*"           # cpp only, e.g. "eDP-1"; "*" = all connected displays
silent = false              # cpp only
disable_mouse = false       # cpp only
disable_parallax = false    # cpp only
log_level = "warning"
target_fps = 60
no_effects = false

[mpvpaper]
output = "*"
mpv_options = ["loop"]
```

### Choosing an engine

The GUI's Settings tab has a segmented picker at the top of the engine
section to choose between the two backends:

- **linux-wallpaper-engine (Rust)** — the Rust port. Uses `-p`, `-m`,
  `--fit-mode`, `-l`, `--no-effects`, `--target-fps`, `--assets-path`.
- **linux-wallpaperengine (C++)** — Almamu's C++ original. Uses
  `--screen-root` (auto-detected from `wlr-randr` / `xrandr` when set
  to `*`), `--bg` to assign the wallpaper, plus `--scaling`, `--fps`,
  `--silent`, `--disable-mouse`, `--disable-parallax`, `--assets-dir`.
  The path is the *project directory* (containing `project.json` and
  `scene.pkg`), not the `.pkg` file itself.

Both binary paths are stored independently, so switching variants never
loses a custom path. Common options (target FPS, log level) apply to
both.

### Video wallpapers

Video wallpapers are routed through the C++ engine (`linux-wallpaperengine`)
whenever it is installed, regardless of which variant is selected for
scene wallpapers. The C++ engine has its own `VideoPlayback/MPV`
subsystem and integrates with the same display / `--bg` plumbing as
scene wallpapers, so all the same per-screen options (scaling, FPS,
silent, etc.) apply.

If the C++ engine is not installed, the tray falls back to `mpvpaper`
with the configured display output and `mpv_options` from the Settings
tab.

## License

TBD
