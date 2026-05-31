use serde::{Deserialize, Serialize};

/// A request from GUI to tray daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcRequest {
    pub id: u64,
    pub cmd: IpcCommand,
}

/// Commands the GUI can send to the tray daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IpcCommand {
    /// Get tray status (wallpaper running, title, etc.)
    GetStatus,
    /// Apply a scene wallpaper
    ApplyScene {
        file_path: String,
        title: String,
    },
    /// Apply a video wallpaper
    ApplyVideo {
        file_path: String,
        title: String,
    },
    /// Stop the currently running wallpaper
    StopWallpaper,
    /// Shutdown the tray daemon (and stop wallpaper)
    Quit,
}

/// A response from tray daemon to GUI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Status information from the tray daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayStatus {
    pub wallpaper_running: bool,
    pub current_wallpaper_title: Option<String>,
    pub gui_running: bool,
}

impl IpcRequest {
    pub fn new(id: u64, cmd: IpcCommand) -> Self {
        Self { id, cmd }
    }
}

impl IpcResponse {
    pub fn ok(id: u64, data: Option<serde_json::Value>) -> Self {
        Self {
            id,
            ok: true,
            error: None,
            data,
        }
    }

    pub fn err(id: u64, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            error: Some(error.into()),
            data: None,
        }
    }
}
