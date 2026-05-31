use std::path::PathBuf;

/// Tool availability detected at startup
#[derive(Debug, Clone)]
pub struct ToolsStatus {
    pub engine_available: bool,
    pub mpvpaper_available: bool,
}

impl ToolsStatus {
    pub fn detect() -> Self {
        Self {
            engine_available: which_bin("linux-wallpaper-engine"),
            mpvpaper_available: which_bin("mpvpaper"),
        }
    }
}

/// Check if a binary exists in PATH or common locations
fn which_bin(name: &str) -> bool {
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

/// Parameters for linux-wallpaper-engine CLI
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EngineParams {
    /// Output mode (e.g. "wlr", "kms", "x11")
    #[serde(default = "default_mode")]
    pub mode: String,

    /// How the wallpaper fits the screen: "cover", "contain", "fill", "stretch"
    #[serde(default = "default_fit_mode")]
    pub fit_mode: String,

    /// Log level: "error", "warning", "info", "debug", "trace"
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Target FPS. None = render as fast as possible
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_fps: Option<u32>,

    /// Disable effects
    #[serde(default)]
    pub no_effects: bool,

    /// Use stdin for JSON config instead of CLI args
    #[serde(default)]
    pub use_stdin: bool,
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
impl Default for EngineParams {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            fit_mode: default_fit_mode(),
            log_level: default_log_level(),
            target_fps: None,
            no_effects: false,
            use_stdin: false,
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

/// Application configuration persisted to disk (TOML format)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// Path to the steamapps directory (contains common/ and workshop/)
    #[serde(default)]
    pub steamapps_path: String,

    /// linux-wallpaper-engine parameters
    #[serde(default)]
    pub engine: EngineParams,

    /// mpvpaper parameters
    #[serde(default)]
    pub mpvpaper: MpvpaperParams,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            steamapps_path: String::new(),
            engine: EngineParams::default(),
            mpvpaper: MpvpaperParams::default(),
        }
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
    pub fn assets_path(&self) -> Option<PathBuf> {
        if self.steamapps_path.is_empty() {
            return None;
        }
        let p = PathBuf::from(&self.steamapps_path)
            .join("common")
            .join("wallpaper_engine")
            .join("assets");
        if p.is_dir() {
            Some(p)
        } else {
            None
        }
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
}
