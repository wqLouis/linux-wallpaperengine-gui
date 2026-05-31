use std::path::{Path, PathBuf};

/// Type of wallpaper
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WallpaperType {
    Scene,
    Video,
    /// Wallpaper exists but can't be used (e.g., built-in without .pkg)
    Unsupported,
}

/// Represents a discovered wallpaper
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Wallpaper {
    pub title: String,
    pub wallpaper_type: WallpaperType,
    /// Path to the wallpaper directory
    pub dir: PathBuf,
    /// Path to the main file (scene.pkg or video file)
    pub file_path: Option<PathBuf>,
    /// Path to preview image (jpg/png/gif)
    pub preview_path: Option<PathBuf>,
    /// Source: "builtin" or "workshop"
    pub source: String,
    /// Whether this wallpaper can be applied
    pub can_apply: bool,
}

/// Raw project.json structure
#[derive(Debug, serde::Deserialize)]
struct ProjectJson {
    title: Option<String>,
    #[serde(rename = "type")]
    project_type: Option<String>,
    file: Option<String>,
    #[allow(dead_code)]
    _preview: Option<String>,
}

/// Discover all wallpapers from the configured Steam paths
pub fn discover_wallpapers(workshop_path: Option<PathBuf>, builtin_path: Option<PathBuf>) -> Vec<Wallpaper> {
    let mut wallpapers = Vec::new();

    // Discover workshop wallpapers
    if let Some(ref wp) = workshop_path {
        if wp.is_dir() {
            if let Ok(entries) = std::fs::read_dir(wp) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        if let Some(wp) = parse_wallpaper_dir(&entry.path(), "workshop") {
                            wallpapers.push(wp);
                        }
                    }
                }
            }
        }
    }

    // Discover built-in wallpapers
    if let Some(ref bp) = builtin_path {
        if bp.is_dir() {
            if let Ok(entries) = std::fs::read_dir(bp) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        if let Some(wp) = parse_wallpaper_dir(&entry.path(), "builtin") {
                            if !wallpapers.iter().any(|w| w.dir == wp.dir) {
                                wallpapers.push(wp);
                            }
                        }
                    }
                }
            }
        }
    }

    wallpapers
}

/// Parse a single wallpaper directory, reading project.json
fn parse_wallpaper_dir(dir: &Path, source: &str) -> Option<Wallpaper> {
    let project_path = dir.join("project.json");
    if !project_path.is_file() {
        return None;
    }

    let content = std::fs::read_to_string(&project_path).ok()?;
    let project: ProjectJson = serde_json::from_str(&content).ok()?;

    let title = project.title.clone().unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    });

    let project_type = project.project_type.as_deref().unwrap_or("").to_lowercase();

    match project_type.as_str() {
        "scene" => parse_scene_wallpaper(dir, source, &title, &project),
        "video" => parse_video_wallpaper(dir, source, &title, &project),
        _ => {
            // Unknown type — try to detect
            if dir.join("scene.pkg").is_file() {
                parse_scene_wallpaper(dir, source, &title, &project)
            } else if let Some(vf) = find_video_file(dir) {
                Some(Wallpaper {
                    title,
                    wallpaper_type: WallpaperType::Video,
                    dir: dir.to_path_buf(),
                    file_path: Some(vf),
                    preview_path: find_preview(dir),
                    source: source.to_string(),
                    can_apply: true,
                })
            } else {
                None
            }
        }
    }
}

fn parse_scene_wallpaper(dir: &Path, source: &str, title: &str, _project: &ProjectJson) -> Option<Wallpaper> {
    let pkg_path = dir.join("scene.pkg");

    if pkg_path.is_file() {
        Some(Wallpaper {
            title: title.to_string(),
            wallpaper_type: WallpaperType::Scene,
            dir: dir.to_path_buf(),
            file_path: Some(pkg_path),
            preview_path: find_preview(dir),
            source: source.to_string(),
            can_apply: true,
        })
    } else {
        // Scene wallpaper without .pkg — unsupported (usually built-in)
        Some(Wallpaper {
            title: title.to_string(),
            wallpaper_type: WallpaperType::Unsupported,
            dir: dir.to_path_buf(),
            file_path: None,
            preview_path: find_preview(dir),
            source: source.to_string(),
            can_apply: false,
        })
    }
}

fn parse_video_wallpaper(dir: &Path, source: &str, title: &str, project: &ProjectJson) -> Option<Wallpaper> {
    let video_path = if let Some(ref file) = project.file {
        let p = dir.join(file);
        if p.is_file() {
            Some(p)
        } else {
            find_video_file(dir)
        }
    } else {
        find_video_file(dir)
    };

    let can_apply = video_path.is_some();

    Some(Wallpaper {
        title: title.to_string(),
        wallpaper_type: WallpaperType::Video,
        dir: dir.to_path_buf(),
        file_path: video_path,
        preview_path: find_preview(dir),
        source: source.to_string(),
        can_apply,
    })
}

fn find_video_file(dir: &Path) -> Option<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                let ext = ext.to_string_lossy().to_lowercase();
                if matches!(ext.as_ref(), "mp4" | "webm" | "avi" | "mkv") {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn find_preview(dir: &Path) -> Option<PathBuf> {
    for name in ["preview.jpg", "preview.gif", "preview.png"] {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}
