use std::path::PathBuf;

/// Which backend implementation to invoke for scene wallpapers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EngineVariant {
    /// `linux-wallpaper-engine` — the user's Rust port.
    #[serde(rename = "rust")]
    Rust,
    /// `linux-wallpaperengine` — Almamu's C++ original.
    #[serde(rename = "cpp")]
    Cpp,
}

impl Default for EngineVariant {
    fn default() -> Self {
        EngineVariant::Rust
    }
}

impl EngineVariant {
    /// Default binary name for this variant.
    #[allow(dead_code)]
    pub fn default_binary(self) -> &'static str {
        match self {
            EngineVariant::Rust => "linux-wallpaper-engine",
            EngineVariant::Cpp => "linux-wallpaperengine",
        }
    }

    /// Human-readable label including the implementation family.
    pub fn label(self) -> &'static str {
        match self {
            EngineVariant::Rust => "linux-wallpaper-engine (Rust)",
            EngineVariant::Cpp => "linux-wallpaperengine (C++)",
        }
    }
}

/// Tool availability detected at startup
#[derive(Debug, Clone)]
pub struct ToolsStatus {
    /// `linux-wallpaper-engine` (Rust) is available, regardless of selection.
    pub engine_rust_available: bool,
    /// `linux-wallpaperengine` (C++) is available, regardless of selection.
    pub engine_cpp_available: bool,
    pub mpvpaper_available: bool,
}

impl ToolsStatus {
    pub fn detect(
        rust_bin: &str,
        cpp_bin: &str,
        mpvpaper_bin: &str,
        _selected: EngineVariant,
    ) -> Self {
        Self {
            engine_rust_available: which_bin(rust_bin),
            engine_cpp_available: which_bin(cpp_bin),
            mpvpaper_available: which_bin(mpvpaper_bin),
        }
    }

    /// Convenience: was the binary for the currently selected variant found?
    pub fn selected_available(&self, selected: EngineVariant) -> bool {
        match selected {
            EngineVariant::Rust => self.engine_rust_available,
            EngineVariant::Cpp => self.engine_cpp_available,
        }
    }
}

/// Check if a binary exists in PATH or common locations.
/// If `name` contains a path separator, treat it as an explicit path
/// and only check that exact location.
pub fn which_bin(name: &str) -> bool {
    if name.contains('/') {
        return PathBuf::from(name).is_file();
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let candidate = PathBuf::from(dir).join(name);
            if candidate.is_file() {
                return true;
            }
        }
    }
    for prefix in ["/usr/bin", "/usr/local/bin", "/opt"] {
        let candidate = PathBuf::from(prefix).join(name);
        if candidate.is_file() {
            return true;
        }
    }
    false
}

/// Parameters for the scene-wallpaper engine CLI.
/// Fields are kept as a superset of both variants' CLIs; unused fields
/// are simply ignored when invoking the other engine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EngineParams {
    /// Which backend to invoke.
    #[serde(default)]
    pub variant: EngineVariant,

    // ── Common ───────────────────────────────────────────────────────────
    /// Log level: "error", "warning", "info", "debug", "trace"
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Target FPS. None = render as fast as possible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_fps: Option<u32>,

    /// Disable effects (Rust only — silently ignored by C++).
    #[serde(default)]
    pub no_effects: bool,

    // ── Rust-only (`linux-wallpaper-engine`) ─────────────────────────────
    /// Output mode (e.g. "wlr", "winit").
    #[serde(default = "default_mode")]
    pub mode: String,

    /// How the wallpaper fits the screen: "cover", "contain", "stretch".
    #[serde(default = "default_fit_mode")]
    pub fit_mode: String,

    /// Use stdin for JSON config instead of CLI args (Rust only).
    #[serde(default)]
    pub use_stdin: bool,

    // ── C++-only (`linux-wallpaperengine`) ──────────────────────────────
    /// Wallpaper scaling: "stretch", "fit", "fill", "default".
    #[serde(default = "default_scaling")]
    pub scaling: String,

    /// Output display (e.g. "*", "eDP-1", "HDMI-A-1"). `*` = all
    /// currently connected displays (auto-detected via wlr-randr /
    /// xrandr). Anything else is passed verbatim to `--screen-root`.
    #[serde(default = "default_screen_root")]
    pub screen_root: String,

    /// Mute background audio.
    #[serde(default)]
    pub silent: bool,

    /// Disable mouse interaction.
    #[serde(default)]
    pub disable_mouse: bool,

    /// Disable parallax effect.
    #[serde(default)]
    pub disable_parallax: bool,
}

fn default_mode() -> String {
    "wlr".to_string()
}
fn default_fit_mode() -> String {
    "cover".to_string()
}
fn default_log_level() -> String {
    "warning".to_string()
}
fn default_scaling() -> String {
    "default".to_string()
}
fn default_screen_root() -> String {
    "*".to_string()
}
impl Default for EngineParams {
    fn default() -> Self {
        Self {
            variant: EngineVariant::default(),
            log_level: default_log_level(),
            target_fps: None,
            no_effects: false,
            mode: default_mode(),
            fit_mode: default_fit_mode(),
            use_stdin: false,
            scaling: default_scaling(),
            screen_root: default_screen_root(),
            silent: false,
            disable_mouse: false,
            disable_parallax: false,
        }
    }
}

/// Parameters for mpvpaper
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MpvpaperParams {
    /// mpv options passed to the mpv instance
    #[serde(default)]
    pub mpv_options: Vec<String>,

    /// Output to apply wallpaper to (e.g. "HDMI-A-1", "*" for all)
    #[serde(default = "default_output")]
    pub output: String,
}

fn default_output() -> String {
    "*".to_string()
}

impl Default for MpvpaperParams {
    fn default() -> Self {
        Self {
            mpv_options: vec!["loop".to_string()],
            output: default_output(),
        }
    }
}

/// Auto-start wallpaper configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AutoStart {
    /// Whether to auto-apply a wallpaper on daemon startup
    #[serde(default)]
    pub enabled: bool,

    /// Full path to the wallpaper file (scene.pkg or video file)
    #[serde(default)]
    pub file_path: String,

    /// Display title for the wallpaper
    #[serde(default)]
    pub title: String,

    /// "scene" or "video"
    #[serde(default)]
    pub wallpaper_type: String,
}

impl Default for AutoStart {
    fn default() -> Self {
        Self {
            enabled: false,
            file_path: String::new(),
            title: String::new(),
            wallpaper_type: String::new(),
        }
    }
}

/// Application configuration persisted to disk (TOML format)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// Path to the steamapps directory (contains common/ and workshop/)
    #[serde(default)]
    pub steamapps_path: String,

    /// Path or name of the Rust engine binary (`linux-wallpaper-engine`).
    /// Falls back to the legacy `engine_binary` key for old configs.
    #[serde(
        default = "default_engine_rust_binary",
        alias = "engine_binary"
    )]
    pub engine_rust_binary: String,

    /// Path or name of the C++ engine binary (`linux-wallpaperengine`).
    #[serde(default = "default_engine_cpp_binary")]
    pub engine_cpp_binary: String,

    /// Path or name of the mpvpaper binary
    #[serde(default = "default_mpvpaper_binary")]
    pub mpvpaper_binary: String,

    /// Engine parameters (variant + per-variant CLI options)
    #[serde(default)]
    pub engine: EngineParams,

    /// mpvpaper parameters
    #[serde(default)]
    pub mpvpaper: MpvpaperParams,

    /// Auto-start wallpaper (applied when daemon starts)
    #[serde(default)]
    pub auto_start: AutoStart,
}

fn default_engine_rust_binary() -> String {
    "linux-wallpaper-engine".to_string()
}
fn default_engine_cpp_binary() -> String {
    "linux-wallpaperengine".to_string()
}
fn default_mpvpaper_binary() -> String {
    "mpvpaper".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            steamapps_path: String::new(),
            engine_rust_binary: default_engine_rust_binary(),
            engine_cpp_binary: default_engine_cpp_binary(),
            mpvpaper_binary: default_mpvpaper_binary(),
            engine: EngineParams::default(),
            mpvpaper: MpvpaperParams::default(),
            auto_start: AutoStart::default(),
        }
    }
}

impl Config {
    /// Return the binary path/name for the currently selected engine variant.
    pub fn engine_binary(&self) -> &str {
        match self.engine.variant {
            EngineVariant::Rust => &self.engine_rust_binary,
            EngineVariant::Cpp => &self.engine_cpp_binary,
        }
    }

    /// Whether the C++ engine binary (`linux-wallpaperengine`) is on PATH
    /// or at the configured explicit location. Used to decide whether
    /// video wallpapers can be routed through it before falling back to
    /// mpvpaper.
    pub fn cpp_engine_available(&self) -> bool {
        which_bin(&self.engine_cpp_binary)
    }
}

impl Config {
    fn config_path() -> PathBuf {
        let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("linux-wallpaperengine-gui");
        std::fs::create_dir_all(&path).ok();
        path.push("config.toml");
        path
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Ok(toml_str) = toml::to_string_pretty(self) {
            std::fs::write(&path, toml_str).ok();
        }
    }

    /// Try to auto-detect the steamapps directory
    pub fn auto_detect_steam_path() -> Option<String> {
        let home = dirs::home_dir()?;
        let candidates = [
            home.join(".steam/steam/steamapps"),
            home.join(".local/share/Steam/steamapps"),
            home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps"),
        ];
        for candidate in &candidates {
            let we = candidate.join("common").join("wallpaper_engine");
            if we.is_dir() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
        None
    }

    /// steamapps/common/wallpaper_engine/projects/defaultprojects
    pub fn builtin_projects_path(&self) -> Option<PathBuf> {
        if self.steamapps_path.is_empty() {
            return None;
        }
        let p = PathBuf::from(&self.steamapps_path)
            .join("common")
            .join("wallpaper_engine")
            .join("projects")
            .join("defaultprojects");
        Some(p)
    }

    /// steamapps/workshop/content/431960
    pub fn workshop_path(&self) -> Option<PathBuf> {
        if self.steamapps_path.is_empty() {
            return None;
        }
        let p = PathBuf::from(&self.steamapps_path)
            .join("workshop")
            .join("content")
            .join("431960");
        Some(p)
    }

    /// steamapps/common/wallpaper_engine/assets (for texture fallback)
    /// Path to the Wallpaper Engine assets directory (shaders, fonts, etc.).
    /// Does NOT check if the directory exists — the wallpaper engine binary
    /// needs this path regardless.
    pub fn assets_path(&self) -> Option<PathBuf> {
        if self.steamapps_path.is_empty() {
            return None;
        }
        Some(
            PathBuf::from(&self.steamapps_path)
                .join("common")
                .join("wallpaper_engine")
                .join("assets"),
        )
    }

    /// Path to the IPC socket
    pub fn socket_path() -> PathBuf {
        let mut path = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        path.push("linux-wallpaperengine-gui");
        std::fs::create_dir_all(&path).ok();
        path.push("ipc.sock");
        path
    }

    /// Path that stores the socket path for GUI to find the tray
    pub fn socket_info_path() -> PathBuf {
        let mut path = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        path.push("linux-wallpaperengine-gui");
        std::fs::create_dir_all(&path).ok();
        path.push("ipc-info.json");
        path
    }

    /// Lock file written by the GUI on startup (contains PID).
    /// The tray reads this to detect an already-running GUI.
    pub fn gui_lock_path() -> PathBuf {
        let mut path = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        path.push("linux-wallpaperengine-gui");
        std::fs::create_dir_all(&path).ok();
        path.push("gui-lock.json");
        path
    }

    /// Check whether the PID stored in the GUI lock file is still alive.
    /// Returns Some(pid) if alive, None otherwise.
    pub fn check_gui_alive() -> Option<u32> {
        let path = Self::gui_lock_path();
        let content = std::fs::read_to_string(&path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&content).ok()?;
        let pid = v["pid"].as_u64()? as u32;
        // Check /proc/<pid> — if it exists the process is alive
        let proc_path = std::path::PathBuf::from(format!("/proc/{}", pid));
        if proc_path.exists() {
            Some(pid)
        } else {
            // Stale lock file — clean up
            let _ = std::fs::remove_file(&path);
            None
        }
    }

    /// Write the GUI lock file with current PID.
    pub fn write_gui_lock() {
        let path = Self::gui_lock_path();
        let pid = std::process::id();
        let json = serde_json::json!({"pid": pid});
        if let Ok(s) = serde_json::to_string(&json) {
            let _ = std::fs::write(&path, s);
        }
    }

    /// Remove the GUI lock file on clean shutdown.
    pub fn remove_gui_lock() {
        let _ = std::fs::remove_file(Self::gui_lock_path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_variant_default_is_rust() {
        assert_eq!(EngineVariant::default(), EngineVariant::Rust);
        assert_eq!(EngineVariant::Rust.default_binary(), "linux-wallpaper-engine");
        assert_eq!(EngineVariant::Cpp.default_binary(), "linux-wallpaperengine");
    }

    #[test]
    fn engine_params_default_includes_cpp_fields() {
        let p = EngineParams::default();
        assert_eq!(p.variant, EngineVariant::Rust);
        assert_eq!(p.scaling, "default");
        assert_eq!(p.screen_root, "*", "screen_root should default to '*' (all)");
        assert!(!p.silent);
        assert!(!p.disable_mouse);
        assert!(!p.disable_parallax);
    }

    #[test]
    fn config_round_trip_preserves_cpp_variant() {
        let cfg = Config {
            steamapps_path: "/tmp/steam".into(),
            engine_rust_binary: "/opt/rust/engine".into(),
            engine_cpp_binary: "/opt/cpp/engine".into(),
            mpvpaper_binary: "mpvpaper".into(),
            engine: EngineParams {
                variant: EngineVariant::Cpp,
                scaling: "fill".into(),
                screen_root: "eDP-1".into(),
                silent: true,
                target_fps: Some(30),
                disable_parallax: true,
                ..EngineParams::default()
            },
            mpvpaper: MpvpaperParams::default(),
            auto_start: AutoStart::default(),
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.engine_rust_binary, "/opt/rust/engine");
        assert_eq!(back.engine_cpp_binary, "/opt/cpp/engine");
        assert_eq!(back.engine.variant, EngineVariant::Cpp);
        assert_eq!(back.engine.scaling, "fill");
        assert_eq!(back.engine.screen_root, "eDP-1");
        assert!(back.engine.silent);
        assert!(back.engine.disable_parallax);
        assert_eq!(back.engine.target_fps, Some(30));
        assert_eq!(back.engine_binary(), "/opt/cpp/engine");
    }

    #[test]
    fn legacy_engine_binary_alias_loads_as_rust() {
        // Old configs written before the per-variant split used a single
        // `engine_binary` key. The serde alias should keep those working,
        // treating the old key as the rust binary path.
        let old = r#"
            steamapps_path = "/x"
            engine_binary = "/old/rust/path"
            mpvpaper_binary = "mpv"
        "#;
        let parsed: Config = toml::from_str(old).unwrap();
        assert_eq!(parsed.engine_rust_binary, "/old/rust/path");
        assert_eq!(parsed.engine_cpp_binary, "linux-wallpaperengine");
        assert_eq!(parsed.engine_binary(), "/old/rust/path");
    }

    #[test]
    fn cpp_engine_binary_helper_switches_with_variant() {
        let mut cfg = Config::default();
        assert_eq!(cfg.engine_binary(), "linux-wallpaper-engine");
        cfg.engine.variant = EngineVariant::Cpp;
        assert_eq!(cfg.engine_binary(), "linux-wallpaperengine");
        cfg.engine_cpp_binary = "/custom/cpp".into();
        assert_eq!(cfg.engine_binary(), "/custom/cpp");
    }
}
#[test]
fn on_disk_round_trip() {
    use std::fs;
    let dir = std::env::temp_dir().join("lwpe-gui-smoke");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    let original = r#"
steamapps_path = "/home/x/.steam/steam/steamapps"
engine_rust_binary = "/custom/rust/path"
engine_cpp_binary = "/custom/cpp/path"
mpvpaper_binary = "mpvpaper"

[engine]
variant = "cpp"
log_level = "debug"
target_fps = 60
no_effects = false
mode = "wlr"
fit_mode = "cover"
use_stdin = false
scaling = "fill"
screen_root = "eDP-1"
silent = true
disable_mouse = true
disable_parallax = false
"#;
    fs::write(&path, original).unwrap();
    let s = fs::read_to_string(&path).unwrap();
    let cfg: Config = toml::from_str(&s).unwrap();
    assert_eq!(cfg.engine.variant, EngineVariant::Cpp);
    assert_eq!(cfg.engine_rust_binary, "/custom/rust/path");
    assert_eq!(cfg.engine_cpp_binary, "/custom/cpp/path");
    assert_eq!(cfg.engine.scaling, "fill");
    assert_eq!(cfg.engine.screen_root, "eDP-1");
    assert!(cfg.engine.silent);
    assert!(cfg.engine.disable_mouse);
    assert!(!cfg.engine.disable_parallax);
    assert_eq!(cfg.engine.target_fps, Some(60));
    // Also verify that re-serializing gives a stable result
    let out = toml::to_string_pretty(&cfg).unwrap();
    let back: Config = toml::from_str(&out).unwrap();
    assert_eq!(back.engine.variant, EngineVariant::Cpp);
    assert_eq!(back.engine_cpp_binary, "/custom/cpp/path");
}
