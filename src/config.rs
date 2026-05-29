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

/// Application configuration persisted to disk (TOML format)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// Path to the steamapps directory (contains common/ and workshop/)
    pub steamapps_path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            steamapps_path: String::new(),
        }
    }
}

impl Config {
    fn config_path() -> PathBuf {
        let mut path = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."));
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
}
