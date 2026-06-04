use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use iced::event;
use iced::widget::{
    pick_list, scrollable, text_input, button, column, container, image, row, text, toggler, Space,
};
use iced::window;
use iced::{Alignment, Element, Length, Subscription, Task};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::oneshot;

use std::pin::Pin;
use std::task::{Context, Poll};
use iced::futures::Stream;

// ── Global IPC state for the reader subscription ───────────────────────────
// Iced 0.14's Subscription::run_with requires D: Hash, so we store the
// shared state in a OnceLock and pass a simple key.

struct IpcGlobals {
    writer: Arc<tokio::sync::Mutex<Option<OwnedWriteHalf>>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<IpcResponse>>>>,
}

static IPC_GLOBALS: OnceLock<IpcGlobals> = OnceLock::new();

use crate::config::{Config, EngineParams, MpvpaperParams, ToolsStatus};
use crate::ipc::{IpcCommand, IpcEvent, IpcMessage, IpcRequest, IpcResponse, TrayStatus};
use crate::theme;
use crate::wallpaper::{Wallpaper, WallpaperType, discover_wallpapers};

// ── App state ──────────────────────────────────────────────────────────────

pub struct GuiApp {
    screen: Screen,
    config: Config,
    tools: ToolsStatus,
    wallpapers: Vec<Wallpaper>,
    tray_status: TrayStatus,
    path_input: String,
    status_message: Option<String>,
    window_width: f32,
    window_id: Option<window::Id>,
    ipc_connected: bool,
    next_request_id: u64,
    /// Persistent IPC writer (shared with the reader task for send).
    ipc_writer: Arc<tokio::sync::Mutex<Option<OwnedWriteHalf>>>,
    /// Pending request ID → oneshot sender for response routing.
    pending_requests: Arc<Mutex<HashMap<u64, oneshot::Sender<IpcResponse>>>>,
    engine_rust_binary: String,
    engine_cpp_binary: String,
    mpvpaper_binary: String,
    engine_variant: crate::config::EngineVariant,
    engine_mode: String,
    engine_fit_mode: String,
    engine_log_level: String,
    engine_target_fps_str: String,
    engine_no_effects: bool,
    engine_scaling: String,
    engine_screen_root: String,
    engine_silent: bool,
    engine_disable_mouse: bool,
    engine_disable_parallax: bool,
    mpv_output: String,
    mpv_options_str: String,
    auto_start_enabled: bool,
    auto_start_file_path: String,
    auto_start_title: String,
    auto_start_wallpaper_type: String,
    last_applied_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Screen {
    Library,
    Settings,
}

#[derive(Debug, Clone)]
pub enum Message {
    GoToLibrary,
    GoToSettings,
    PathInputChanged(String),
    AutoDetectPath,
    SaveSettings,
    EngineRustBinaryChanged(String),
    EngineCppBinaryChanged(String),
    MpvpaperBinaryChanged(String),
    EngineVariantChanged(crate::config::EngineVariant),
    EngineModeChanged(String),
    EngineFitModeChanged(String),
    EngineLogLevelChanged(String),
    EngineTargetFpsChanged(String),
    EngineNoEffectsToggled(bool),
    EngineScalingChanged(String),
    EngineScreenRootChanged(String),
    EngineSilentToggled(bool),
    EngineDisableMouseToggled(bool),
    EngineDisableParallaxToggled(bool),
    MpvOutputChanged(String),
    MpvOptionsChanged(String),
    ApplyWallpaper(usize),
    StopWallpaper,
    RefreshWallpapers,
    /// A response was matched to a pending request.
    IpcResponseReceived(IpcResponse),
    /// Push event received from the tray.
    IpcEvent(IpcEvent),
    /// IPC connection established.
    IpcConnected,
    /// IPC connection lost.
    IpcDisconnected,
    WindowResized(window::Id, f32),
    AutoStartToggled(bool),
    SetCurrentAsAutoStart,
    ClearAutoStart,

    /// User clicked window close — notify tray then exit.
    WindowCloseRequested,
    /// Perform actual window close.
    Exit,
}

// ── Constructor ────────────────────────────────────────────────────────────

impl GuiApp {
    pub fn new() -> (Self, Task<Message>) {
        let config = Config::load();
        let tools = ToolsStatus::detect(
            &config.engine_rust_binary,
            &config.engine_cpp_binary,
            &config.mpvpaper_binary,
            config.engine.variant,
        );
        let empty_status = TrayStatus {
            wallpaper_running: false,
            current_wallpaper_title: None,
            gui_running: false,
        };
        let wallpapers = {
            let w = config.workshop_path();
            let b = config.builtin_projects_path();
            discover_wallpapers(w, b)
        };

        // Write lock file so the tray can detect us
        Config::write_gui_lock();

        let app = Self {
            screen: Screen::Library,
            path_input: config.steamapps_path.clone(),
            engine_rust_binary: config.engine_rust_binary.clone(),
            engine_cpp_binary: config.engine_cpp_binary.clone(),
            mpvpaper_binary: config.mpvpaper_binary.clone(),
            engine_variant: config.engine.variant,
            engine_mode: config.engine.mode.clone(),
            engine_fit_mode: config.engine.fit_mode.clone(),
            engine_log_level: config.engine.log_level.clone(),
            engine_target_fps_str: config
                .engine
                .target_fps
                .map(|f| f.to_string())
                .unwrap_or_default(),
            engine_no_effects: config.engine.no_effects,
            engine_scaling: config.engine.scaling.clone(),
            engine_screen_root: config.engine.screen_root.clone(),
            engine_silent: config.engine.silent,
            engine_disable_mouse: config.engine.disable_mouse,
            engine_disable_parallax: config.engine.disable_parallax,
            mpv_output: config.mpvpaper.output.clone(),
            mpv_options_str: config.mpvpaper.mpv_options.join(", "),
            auto_start_enabled: config.auto_start.enabled,
            auto_start_file_path: config.auto_start.file_path.clone(),
            auto_start_title: config.auto_start.title.clone(),
            auto_start_wallpaper_type: config.auto_start.wallpaper_type.clone(),
            last_applied_type: None,
            config,
            tools,
            wallpapers,
            tray_status: empty_status,
            status_message: Some("Connecting to daemon…".into()),
            window_width: 1200.0,
            window_id: None,
            ipc_connected: false,
            next_request_id: 1,
            ipc_writer: Arc::new(tokio::sync::Mutex::new(None)),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
        };

        (app, Task::none())
    }

    pub fn title(&self) -> String {
        "Wallpaper Engine Manager".into()
    }

    // ── Update ──────────────────────────────────────────────────────────

    pub fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::GoToLibrary => {
                self.screen = Screen::Library;
                Task::none()
            }
            Message::GoToSettings => {
                self.screen = Screen::Settings;
                self.sync_form();
                Task::none()
            }
            Message::PathInputChanged(v) => {
                self.path_input = v;
                Task::none()
            }
            Message::AutoDetectPath => {
                self.status_message = if let Some(p) = Config::auto_detect_steam_path() {
                    self.path_input = p;
                    Some("Path auto-detected".into())
                } else {
                    Some("Could not auto-detect".into())
                };
                Task::none()
            }
            Message::EngineRustBinaryChanged(v) => {
                self.engine_rust_binary = v;
                Task::none()
            }
            Message::EngineCppBinaryChanged(v) => {
                self.engine_cpp_binary = v;
                Task::none()
            }
            Message::MpvpaperBinaryChanged(v) => {
                self.mpvpaper_binary = v;
                Task::none()
            }
            Message::EngineVariantChanged(v) => {
                self.engine_variant = v;
                Task::none()
            }
            Message::SaveSettings => {
                let cfg = self.build_config();
                cfg.save();
                self.config = cfg;
                self.tools = ToolsStatus::detect(
                    &self.engine_rust_binary,
                    &self.engine_cpp_binary,
                    &self.mpvpaper_binary,
                    self.engine_variant,
                );
                self.status_message = Some("Saved. Refreshing…".into());
                let path = self.config.steamapps_path.clone();
                return Task::perform(
                    async {
                        tokio::task::spawn_blocking(move || {
                            let c = Config {
                                steamapps_path: path,
                                ..Default::default()
                            };
                            discover_wallpapers(c.workshop_path(), c.builtin_projects_path())
                        })
                        .await
                        .unwrap_or_default()
                    },
                    |wps| {
                        Message::IpcResponseReceived(IpcResponse::ok(
                            0,
                            serde_json::to_value(wps).ok(),
                        ))
                    },
                );
            }
            Message::EngineModeChanged(v) => {
                self.engine_mode = v;
                Task::none()
            }
            Message::EngineFitModeChanged(v) => {
                self.engine_fit_mode = v;
                Task::none()
            }
            Message::EngineLogLevelChanged(v) => {
                self.engine_log_level = v;
                Task::none()
            }
            Message::EngineTargetFpsChanged(v) => {
                self.engine_target_fps_str = v;
                Task::none()
            }
            Message::EngineNoEffectsToggled(v) => {
                self.engine_no_effects = v;
                Task::none()
            }
            Message::EngineScalingChanged(v) => {
                self.engine_scaling = v;
                Task::none()
            }
            Message::EngineScreenRootChanged(v) => {
                self.engine_screen_root = v;
                Task::none()
            }
            Message::EngineSilentToggled(v) => {
                self.engine_silent = v;
                Task::none()
            }
            Message::EngineDisableMouseToggled(v) => {
                self.engine_disable_mouse = v;
                Task::none()
            }
            Message::EngineDisableParallaxToggled(v) => {
                self.engine_disable_parallax = v;
                Task::none()
            }
            Message::MpvOutputChanged(v) => {
                self.mpv_output = v;
                Task::none()
            }
            Message::MpvOptionsChanged(v) => {
                self.mpv_options_str = v;
                Task::none()
            }

            Message::ApplyWallpaper(idx) => {
                if !self.ipc_connected {
                    self.status_message = Some("Not connected".into());
                    return Task::none();
                }
                let Some(wp) = self
                    .wallpapers
                    .get(idx)
                    .filter(|w| w.can_apply && w.file_path.is_some())
                    .cloned()
                else {
                    return Task::none();
                };
                let fp = wp.file_path.as_ref().unwrap().to_string_lossy().to_string();
                let title = wp.title.clone();

                self.last_applied_type = Some(match wp.wallpaper_type {
                    WallpaperType::Scene => "scene",
                    WallpaperType::Video => "video",
                    WallpaperType::Unsupported => "",
                }
                .to_string());

                self.status_message = Some(format!("Applying {}…", title));
                let cmd = match wp.wallpaper_type {
                    WallpaperType::Scene => IpcCommand::ApplyScene {
                        file_path: fp,
                        title,
                    },
                    WallpaperType::Video => IpcCommand::ApplyVideo {
                        file_path: fp,
                        title,
                    },
                    WallpaperType::Unsupported => return Task::none(),
                };
                return self.ipc_request(cmd);
            }
            Message::StopWallpaper => {
                if !self.ipc_connected {
                    return Task::none();
                }
                return self.ipc_request(IpcCommand::StopWallpaper);
            }
            Message::RefreshWallpapers => {
                self.status_message = Some("Discovering…".into());
                let path = self.config.steamapps_path.clone();
                return Task::perform(
                    async {
                        tokio::task::spawn_blocking(move || {
                            let c = Config {
                                steamapps_path: path,
                                ..Default::default()
                            };
                            discover_wallpapers(c.workshop_path(), c.builtin_projects_path())
                        })
                        .await
                        .unwrap_or_default()
                    },
                    |wps| {
                        Message::IpcResponseReceived(IpcResponse::ok(
                            0,
                            serde_json::to_value(wps).ok(),
                        ))
                    },
                );
            }
            Message::IpcConnected => {
                self.ipc_connected = true;
                self.status_message = Some("Connected".into());
                // Request initial status now that we're connected
                return self.ipc_request(IpcCommand::GetStatus);
            }
            Message::IpcDisconnected => {
                self.ipc_connected = false;
                self.status_message = Some("Disconnected — retrying…".into());
                Task::none()
            }
            Message::IpcEvent(evt) => {
                match evt.event.as_str() {
                    "status_changed" => {
                        if let Some(data) = evt.data {
                            if let Ok(s) = serde_json::from_value::<TrayStatus>(data) {
                                self.tray_status = s;
                            }
                        }
                    }
                    "show_window" => {
                        // Tray wants us to come to the foreground
                        if let Some(id) = self.window_id {
                            return window::gain_focus(id);
                        }
                    }
                    _ => {}
                }
                Task::none()
            }
            Message::IpcResponseReceived(resp) => {
                if resp.id == 0 {
                    // Wallpaper list update from SaveSettings / RefreshWallpapers
                    if let Some(d) = resp.data {
                        if let Ok(wps) = serde_json::from_value::<Vec<Wallpaper>>(d) {
                            self.wallpapers = wps;
                            self.status_message =
                                Some(format!("{} wallpapers", self.wallpapers.len()));
                        }
                    }
                    return Task::none();
                }

                if !resp.ok {
                    // Show application-level errors without dropping the connection.
                    // Actual disconnections are handled by the IpcDisconnected reader stream.
                    let err_msg = resp.error.clone().unwrap_or_default();
                    self.status_message =
                        Some(format!("Error: {}", err_msg));
                    return Task::none();
                }

                // Response data may carry updated tray status
                if let Some(d) = resp.data {
                    if let Ok(s) = serde_json::from_value::<TrayStatus>(d) {
                        self.tray_status = s;
                    }
                }

                Task::none()
            }
            Message::WindowResized(id, w) => {
                self.window_id = Some(id);
                self.window_width = w;
                Task::none()
            }
            Message::AutoStartToggled(v) => {
                self.auto_start_enabled = v;
                Task::none()
            }
            Message::SetCurrentAsAutoStart => {
                let matched =
                    self.tray_status
                        .current_wallpaper_title
                        .as_ref()
                        .and_then(|title| {
                            self.wallpapers.iter().find(|w| w.title == *title).map(|wp| {
                                let fp = wp
                                    .file_path
                                    .as_ref()
                                    .map(|p| p.to_string_lossy().to_string());
                                let wt = match wp.wallpaper_type {
                                    WallpaperType::Scene => "scene",
                                    WallpaperType::Video => "video",
                                    WallpaperType::Unsupported => "",
                                };
                                (fp, wt, wp.title.clone())
                            })
                        });

                match matched {
                    Some((Some(fp), wt, title)) if !wt.is_empty() => {
                        self.auto_start_enabled = true;
                        self.auto_start_file_path = fp;
                        self.auto_start_title = title;
                        self.auto_start_wallpaper_type = wt.to_string();

                        let mut cfg = self.build_config();
                        cfg.auto_start.enabled = true;
                        cfg.auto_start.file_path = self.auto_start_file_path.clone();
                        cfg.auto_start.title = self.auto_start_title.clone();
                        cfg.auto_start.wallpaper_type = self.auto_start_wallpaper_type.clone();
                        cfg.save();
                        self.config = cfg;
                        self.status_message = Some("Auto-start saved".into());

                    }
                    _ => {
                        self.status_message =
                            Some("No wallpaper currently running or not found in library".into());

                    }
                }
                Task::none()
            }
            Message::ClearAutoStart => {
                self.auto_start_enabled = false;
                self.auto_start_file_path.clear();
                self.auto_start_title.clear();
                self.auto_start_wallpaper_type.clear();

                let mut cfg = self.build_config();
                cfg.auto_start = crate::config::AutoStart::default();
                cfg.save();
                self.config = cfg;
                self.status_message = Some("Auto-start cleared".into());
                Task::none()
            }
            Message::WindowCloseRequested => {
                // Notify the tray we're closing, then exit
                let writer = self.ipc_writer.clone();
                return Task::perform(
                    async move {
                        let mut guard = writer.lock().await;
                        if let Some(ref mut w) = *guard {
                            let req = IpcRequest::new(0, IpcCommand::GuiClosing);
                            let msg = IpcMessage::Request(req);
                            if let Ok(json) = serde_json::to_string(&msg) {
                                let _ = w.write_all((json + "\n").as_bytes()).await;
                            }
                        }
                        Config::remove_gui_lock();
                    },
                    |_| Message::Exit,
                );
            }
            Message::Exit => {
                if let Some(id) = self.window_id {
                    return window::close(id);
                }
                // Fallback: try with a unique ID (may not work, but best effort)
                return window::close(window::Id::unique());
            }

        }
    }

    // ── Subscriptions ───────────────────────────────────────────────────

    pub fn subscription(&self) -> Subscription<Message> {
        // Store globals for the IPC reader stream to access
        let _ = IPC_GLOBALS.set(IpcGlobals {
            writer: self.ipc_writer.clone(),
            pending: self.pending_requests.clone(),
        });

        Subscription::batch([
            event::listen_with(|ev, _, id| {
                if let iced::Event::Window(window::Event::Resized(s)) = ev {
                    Some(Message::WindowResized(id, s.width))
                } else if let iced::Event::Window(window::Event::CloseRequested) = ev {
                    Some(Message::WindowCloseRequested)
                } else {
                    None
                }
            }),
            // Persistent IPC reader — connects, reads events/responses, reconnects on failure
            iced::Subscription::run_with(
                0u64,
                ipc_reader_stream,
            ),

        ])
    }

    // ── View ────────────────────────────────────────────────────────────

    pub fn view(&self) -> Element<'_, Message> {
        container(
            column![
                self.top_bar(),
                match self.screen {
                    Screen::Library => self.library_view(),
                    Screen::Settings => self.settings_view(),
                }
            ],
        )
        .style(theme::app_background)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    // ── IPC helper ──────────────────────────────────────────────────────

    /// Queue an IPC request over the persistent connection.
    /// Returns a Task that resolves to IpcResponseReceived.
    fn ipc_request(&mut self, cmd: IpcCommand) -> Task<Message> {
        let id = self.next_request_id;
        self.next_request_id += 1;
        let writer = self.ipc_writer.clone();
        let pending = self.pending_requests.clone();

        Task::perform(
            async move {
                let (tx, rx) = oneshot::channel();
                pending.lock().unwrap().insert(id, tx);

                let req = IpcRequest::new(id, cmd);
                let msg = IpcMessage::Request(req);
                let Ok(json) = serde_json::to_string(&msg) else {
                    pending.lock().unwrap().remove(&id);
                    return IpcResponse::err(id, "serialize failed");
                };

                let mut guard = writer.lock().await;
                match &mut *guard {
                    Some(w) => {
                        if w.write_all((json + "\n").as_bytes()).await.is_err() {
                            pending.lock().unwrap().remove(&id);
                            return IpcResponse::err(id, "write failed");
                        }
                    }
                    None => {
                        pending.lock().unwrap().remove(&id);
                        return IpcResponse::err(id, "not connected");
                    }
                }
                drop(guard);

                match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
                    Ok(Ok(resp)) => resp,
                    Ok(Err(_)) => IpcResponse::err(id, "channel closed"),
                    Err(_) => {
                        pending.lock().unwrap().remove(&id);
                        IpcResponse::err(id, "timeout")
                    }
                }
            },
            Message::IpcResponseReceived,
        )
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    fn sync_form(&mut self) {
        self.path_input = self.config.steamapps_path.clone();
        self.engine_rust_binary = self.config.engine_rust_binary.clone();
        self.engine_cpp_binary = self.config.engine_cpp_binary.clone();
        self.mpvpaper_binary = self.config.mpvpaper_binary.clone();
        self.engine_variant = self.config.engine.variant;
        self.engine_mode = self.config.engine.mode.clone();
        self.engine_fit_mode = self.config.engine.fit_mode.clone();
        self.engine_log_level = self.config.engine.log_level.clone();
        self.engine_target_fps_str = self
            .config
            .engine
            .target_fps
            .map(|f| f.to_string())
            .unwrap_or_default();
        self.engine_no_effects = self.config.engine.no_effects;
        self.engine_scaling = self.config.engine.scaling.clone();
        self.engine_screen_root = self.config.engine.screen_root.clone();
        self.engine_silent = self.config.engine.silent;
        self.engine_disable_mouse = self.config.engine.disable_mouse;
        self.engine_disable_parallax = self.config.engine.disable_parallax;
        self.mpv_output = self.config.mpvpaper.output.clone();
        self.mpv_options_str = self.config.mpvpaper.mpv_options.join(", ");
        self.auto_start_enabled = self.config.auto_start.enabled;
        self.auto_start_file_path = self.config.auto_start.file_path.clone();
        self.auto_start_title = self.config.auto_start.title.clone();
        self.auto_start_wallpaper_type = self.config.auto_start.wallpaper_type.clone();
    }



    fn build_config(&self) -> Config {
        Config {
            steamapps_path: self.path_input.clone(),
            engine_rust_binary: self.engine_rust_binary.clone(),
            engine_cpp_binary: self.engine_cpp_binary.clone(),
            mpvpaper_binary: self.mpvpaper_binary.clone(),
            engine: EngineParams {
                variant: self.engine_variant,
                mode: self.engine_mode.clone(),
                fit_mode: self.engine_fit_mode.clone(),
                log_level: self.engine_log_level.clone(),
                target_fps: self.engine_target_fps_str.trim().parse().ok(),
                no_effects: self.engine_no_effects,
                use_stdin: false,
                scaling: self.engine_scaling.clone(),
                screen_root: self.engine_screen_root.clone(),
                silent: self.engine_silent,
                disable_mouse: self.engine_disable_mouse,
                disable_parallax: self.engine_disable_parallax,
            },
            mpvpaper: MpvpaperParams {
                output: self.mpv_output.clone(),
                mpv_options: self
                    .mpv_options_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            },
            auto_start: crate::config::AutoStart {
                enabled: self.auto_start_enabled,
                file_path: self.auto_start_file_path.clone(),
                title: self.auto_start_title.clone(),
                wallpaper_type: self.auto_start_wallpaper_type.clone(),
            },
        }
    }

    // ── Top bar ─────────────────────────────────────────────────────────

    fn top_bar(&self) -> Element<'_, Message> {
        let lib_btn = button(text("Library").size(14))
            .style(if self.screen == Screen::Library {
                theme::btn_nav_active as fn(&_, _) -> _
            } else {
                theme::btn_nav
            })
            .on_press(Message::GoToLibrary);
        let set_btn = button(text("Settings").size(14))
            .style(if self.screen == Screen::Settings {
                theme::btn_nav_active as fn(&_, _) -> _
            } else {
                theme::btn_nav
            })
            .on_press(Message::GoToSettings);
        let dot = if self.ipc_connected {
            theme::SUCCESS
        } else {
            theme::ERROR
        };
        let conn = row![
            container(Space::new().width(8).height(8)).style(theme::status_dot(dot)),
            text(if self.ipc_connected {
                "Connected"
            } else {
                "Offline"
            })
            .size(12)
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let refresh = button(text("↻ Refresh").size(13))
            .style(theme::btn_secondary)
            .on_press(Message::RefreshWallpapers);
        container(
            row![
                lib_btn,
                set_btn,
                Space::new().width(Length::Fill),
                conn,
                Space::new().width(12),
                refresh
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding(12)
        .style(theme::top_bar_style)
        .into()
    }

    // ── Library ─────────────────────────────────────────────────────────

    fn library_view(&self) -> Element<'_, Message> {
        if self.wallpapers.is_empty() {
            return container(
                column![
                    text("No wallpapers found").size(18),
                    text("Set your Steam path in Settings, then refresh").size(13)
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
        }

        let running: Element<_> = if self.tray_status.wallpaper_running {
            if let Some(ref title) = self.tray_status.current_wallpaper_title {
                container(
                    row![
                        container(Space::new().width(8).height(8))
                            .style(theme::status_dot(theme::SUCCESS)),
                        text(format!("Running: {}", title)).size(12),
                        Space::new().width(Length::Fill),
                        button("Stop")
                            .style(theme::btn_secondary)
                            .on_press(Message::StopWallpaper)
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding(10)
                .style(theme::surface_style)
                .into()
            } else {
                Space::new().height(0).into()
            }
        } else {
            Space::new().height(0).into()
        };

        // Warning banner when the selected engine isn't installed —
        // helps users debug "Apply does nothing" without having to dig
        // into the settings page.
        let engine_warning: Element<_> =
            if !self.tools.selected_available(self.engine_variant) {
                container(
                    row![
                        container(Space::new().width(8).height(8))
                            .style(theme::status_dot(theme::ERROR)),
                        text(format!(
                            "{} is not installed \u{2014} apply will fail",
                            self.engine_variant.label()
                        ))
                        .size(12),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding(10)
                .style(theme::surface_style)
                .into()
            } else {
                Space::new().height(0).into()
            };

        let pad: f32 = 32.0;
        let gap: f32 = 16.0;
        let min: f32 = 240.0;
        let usable = (self.window_width - pad).max(min);
        let per_row = (usable / min).floor() as usize;
        let per_row = per_row.max(1);
        let cw = ((usable - gap * (per_row as f32 - 1.0)) / per_row as f32).max(120.0);

        let mut rows: Vec<Element<Message>> = Vec::new();
        let mut cur: Vec<Element<Message>> = Vec::new();
        for (i, wp) in self.wallpapers.iter().enumerate() {
            cur.push(self.card(i, wp, cw));
            if cur.len() >= per_row {
                rows.push(row(cur.drain(..)).spacing(16).into());
            }
        }
        if !cur.is_empty() {
            while cur.len() < per_row {
                cur.push(Space::new().width(Length::Fill).into());
            }
            rows.push(row(cur).spacing(16).into());
        }

        let status = if let Some(ref m) = self.status_message {
            container(text(m).size(12)).padding(8)
        } else {
            container(text(""))
        };

        column![
            engine_warning,
            running,
            scrollable(column(rows).spacing(16).padding(16))
                .style(theme::scrollable_style)
                .height(Length::Fill),
            status
        ]
        .into()
    }

    fn card<'a>(&self, idx: usize, wp: &'a Wallpaper, w: f32) -> Element<'a, Message> {
        let is_cur = self.tray_status.current_wallpaper_title.as_deref() == Some(&wp.title);

        let preview: Element<_> = if let Some(ref p) = wp.preview_path {
            container(
                image(p.clone())
                    .width(Length::Fixed(w))
                    .height(Length::Fixed(w))
                    .content_fit(iced::ContentFit::Cover)
                    .border_radius(theme::R_MD),
            )
            .width(Length::Fixed(w))
            .height(Length::Fixed(w))
            .style(theme::preview_container)
            .into()
        } else {
            container(
                column![
                    text(match wp.wallpaper_type {
                        WallpaperType::Scene => "🎬",
                        WallpaperType::Video => "🎥",
                        WallpaperType::Unsupported => "🚫",
                    })
                    .size(28),
                    text("No preview").size(11)
                ]
                .align_x(Alignment::Center)
                .spacing(4),
            )
            .width(Length::Fixed(w))
            .height(Length::Fixed(w))
            .style(theme::preview_container)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
        };

        let type_badge: Element<_> = match wp.wallpaper_type {
            WallpaperType::Scene => container(text("scene").size(10))
                .padding(2)
                .style(|_t| container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgba(
                        0.255, 0.510, 0.875, 0.15,
                    ))),
                    border: iced::Border {
                        radius: theme::R_FULL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into(),
            WallpaperType::Video => container(text("video").size(10))
                .padding(2)
                .style(|_t| container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgba(
                        0.133, 0.773, 0.369, 0.15,
                    ))),
                    border: iced::Border {
                        radius: theme::R_FULL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into(),
            WallpaperType::Unsupported => container(text("n/a").size(10))
                .padding(2)
                .style(|_t| container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgba(
                        0.937, 0.267, 0.267, 0.15,
                    ))),
                    border: iced::Border {
                        radius: theme::R_FULL.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into(),
        };

        let active_dot: Element<_> = if is_cur {
            container(Space::new().width(6).height(6))
                .style(theme::status_dot(theme::SUCCESS))
                .into()
        } else {
            Space::new().width(0).height(0).into()
        };

        let btn = if wp.can_apply {
            if is_cur {
                button("Stop")
                    .style(theme::btn_secondary)
                    .on_press(Message::StopWallpaper)
            } else {
                button("Apply")
                    .style(theme::btn_primary)
                    .on_press(Message::ApplyWallpaper(idx))
            }
        } else {
            button("Unavailable").style(theme::btn_secondary)
        };

        container(
            column![
                preview,
                text(&wp.title).size(13).width(Length::Fill),
                row![
                    active_dot,
                    text(format!("[{}]", wp.source)).size(10),
                    Space::new().width(Length::Fill),
                    type_badge
                ]
                .spacing(4)
                .align_y(Alignment::Center),
                btn
            ]
            .spacing(8)
            .width(Length::Fill),
        )
        .padding(12)
        .style(theme::card_style)
        .into()
    }

    // ── Settings ────────────────────────────────────────────────────────

    fn settings_view(&self) -> Element<'_, Message> {
        let w = Length::Fixed(540.0);
        let hf = Length::FillPortion(1);

        const MODES: &[&str] = &["wlr", "winit"];
        const FIT_MODES: &[&str] = &["cover", "contain", "stretch"];
        const LOG_LEVELS: &[&str] = &["verbose", "debug", "warning", "errors"];
        const SCALINGS: &[&str] = &["default", "stretch", "fit", "fill"];

        // Two independent status lines for the two engines.
        let rust_status = if self.tools.engine_rust_available {
            "✅ linux-wallpaper-engine (Rust) found"
        } else {
            "❌ linux-wallpaper-engine (Rust) not found in PATH"
        };
        let cpp_status = if self.tools.engine_cpp_available {
            "✅ linux-wallpaperengine (C++) found"
        } else {
            "❌ linux-wallpaperengine (C++) not found in PATH"
        };
        let mpv_ok = if self.tools.mpvpaper_available {
            "✅ mpvpaper"
        } else {
            "❌ mpvpaper not found"
        };
        let effects_label = if self.engine_no_effects {
            "Effects: Off"
        } else {
            "Effects: On"
        };
        let effects_toggle = row![
            toggler(!self.engine_no_effects).on_toggle(|b| Message::EngineNoEffectsToggled(!b)),
            text(effects_label).size(13),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        // Segmented picker for the engine variant.
        let variant_picker: Element<_> = {
            let is_rust = self.engine_variant == crate::config::EngineVariant::Rust;
            let is_cpp = self.engine_variant == crate::config::EngineVariant::Cpp;
            let rust_btn = button(text("linux-wallpaper-engine (Rust)").size(13))
                .style(if is_rust { theme::btn_nav_active } else { theme::btn_nav })
                .on_press(Message::EngineVariantChanged(
                    crate::config::EngineVariant::Rust,
                ))
                .width(Length::Fill);
            let cpp_btn = button(text("linux-wallpaperengine (C++)").size(13))
                .style(if is_cpp { theme::btn_nav_active } else { theme::btn_nav })
                .on_press(Message::EngineVariantChanged(
                    crate::config::EngineVariant::Cpp,
                ))
                .width(Length::Fill);
            row![rust_btn, Space::new().width(8), cpp_btn]
                .align_y(Alignment::Center)
                .into()
        };

        // Variant-specific options column.
        let variant_options: Element<_> = match self.engine_variant {
            crate::config::EngineVariant::Rust => column![
                row![
                    container(self.dropdown("Output Mode", MODES, &self.engine_mode, Message::EngineModeChanged)).width(hf),
                    Space::new().width(16),
                    container(self.dropdown("Fit Mode", FIT_MODES, &self.engine_fit_mode, Message::EngineFitModeChanged)).width(hf),
                ].spacing(8),
                Space::new().height(14),
                row![
                    container(self.dropdown("Log Level", LOG_LEVELS, &self.engine_log_level, Message::EngineLogLevelChanged)).width(hf),
                    Space::new().width(16),
                    column![effects_toggle].width(hf).align_x(Alignment::Center),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(4)
            .into(),
            crate::config::EngineVariant::Cpp => column![
                row![
                    container(self.dropdown("Scaling", SCALINGS, &self.engine_scaling, Message::EngineScalingChanged)).width(hf),
                    Space::new().width(16),
                    self.input_field(
                        "Screen Root",
                        &self.engine_screen_root,
                        Message::EngineScreenRootChanged,
                        "* (all displays) or specific name",
                        hf
                    ),
                ]
                .spacing(8),
                // Clickable chips for detected displays — saves the user
                // from having to look up output names from `xrandr`.
                self.display_chips_row(),
                Space::new().height(14),
                row![
                    row![
                        toggler(self.engine_silent).on_toggle(Message::EngineSilentToggled),
                        text("Silent (mute audio)").size(13),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .width(hf),
                    Space::new().width(16),
                    row![
                        toggler(self.engine_disable_mouse)
                            .on_toggle(Message::EngineDisableMouseToggled),
                        text("Disable mouse").size(13),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .width(hf),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                Space::new().height(10),
                row![
                    row![
                        toggler(self.engine_disable_parallax)
                            .on_toggle(Message::EngineDisableParallaxToggled),
                        text("Disable parallax").size(13),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .width(hf),
                    Space::new().width(16),
                    column![].width(hf),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(4)
            .into(),
        };

        let engine_section_title = self.engine_variant.label();

        column![
            scrollable(
                column![
                    self.section(
                        "Steam Library",
                        column![
                            text("Steam apps Path").size(13),
                            row![
                                text_input("/home/user/.steam/steam/steamapps", &self.path_input)
                                    .on_input(Message::PathInputChanged)
                                    .style(theme::text_input_style)
                                    .width(Length::Fill),
                                button("Auto-detect")
                                    .style(theme::btn_secondary)
                                    .on_press(Message::AutoDetectPath),
                            ]
                            .spacing(8),
                            text("Directory containing common/ and workshop/ folders").size(10),
                        ]
                        .spacing(4)
                        .width(w.clone())
                    ),
                    Space::new().height(20),
                    self.section(
                        engine_section_title,
                        column![
                            variant_picker,
                            Space::new().height(14),
                            variant_options,
                            Space::new().height(14),
                            self.input_field(
                                "Target FPS",
                                &self.engine_target_fps_str,
                                Message::EngineTargetFpsChanged,
                                "unlimited (leave empty)",
                                Length::Fill
                            ),
                        ]
                        .spacing(4)
                        .width(w.clone())
                    ),
                    Space::new().height(20),
                    self.section(
                        "mpvpaper",
                        column![
                            row![
                                self.input_field(
                                    "Display Output",
                                    &self.mpv_output,
                                    Message::MpvOutputChanged,
                                    "* (all displays)",
                                    hf
                                ),
                                Space::new().width(16),
                                self.input_field(
                                    "mpv Options",
                                    &self.mpv_options_str,
                                    Message::MpvOptionsChanged,
                                    "loop, no-audio",
                                    hf
                                ),
                            ]
                            .spacing(8),
                        ]
                        .spacing(4)
                        .width(w.clone())
                    ),
                    Space::new().height(20),
                    self.section(
                        "Auto-Start",
                        column![
                            row![
                                toggler(self.auto_start_enabled)
                                    .on_toggle(Message::AutoStartToggled),
                                text("Apply wallpaper on daemon startup").size(13),
                            ]
                            .spacing(10)
                            .align_y(Alignment::Center),
                            Space::new().height(8),
                            if self.auto_start_enabled && !self.auto_start_title.is_empty() {
                                column![
                                    text(format!("Wallpaper: {}", self.auto_start_title)).size(12),
                                    text(format!("Path: {}", self.auto_start_file_path))
                                        .size(10)
                                        .style(|_| iced::widget::text::Style {
                                            color: Some(theme::TEXT_MUTED),
                                        }),
                                    Space::new().height(8),
                                    row![
                                        button("Set to Current Wallpaper")
                                            .style(theme::btn_primary)
                                            .on_press(Message::SetCurrentAsAutoStart),
                                        Space::new().width(8),
                                        button("Clear")
                                            .style(theme::btn_secondary)
                                            .on_press(Message::ClearAutoStart),
                                    ]
                                    .spacing(8),
                                ]
                                .spacing(4)
                            } else {
                                column![
                                    text("No auto-start wallpaper configured.").size(12),
                                    Space::new().height(8),
                                    button("Set to Current Wallpaper")
                                        .style(theme::btn_primary)
                                        .on_press(Message::SetCurrentAsAutoStart),
                                ]
                                .spacing(4)
                            }
                        ]
                        .spacing(4)
                        .width(w.clone())
                    ),
                    Space::new().height(20),
                    self.section(
                        "Tools",
                        column![
                            row![
                                self.input_field(
                                    "Engine Binary (Rust)",
                                    &self.engine_rust_binary,
                                    Message::EngineRustBinaryChanged,
                                    "linux-wallpaper-engine",
                                    hf
                                ),
                                Space::new().width(16),
                                self.input_field(
                                    "Engine Binary (C++)",
                                    &self.engine_cpp_binary,
                                    Message::EngineCppBinaryChanged,
                                    "linux-wallpaperengine",
                                    hf
                                ),
                            ]
                            .spacing(8),
                            Space::new().height(10),
                            row![
                                self.input_field(
                                    "mpvpaper Binary",
                                    &self.mpvpaper_binary,
                                    Message::MpvpaperBinaryChanged,
                                    "mpvpaper",
                                    hf
                                ),
                                Space::new().width(16),
                                column![].width(hf),
                            ]
                            .spacing(8),
                            Space::new().height(10),
                            text(rust_status).size(13),
                            text(cpp_status).size(13),
                            text(mpv_ok).size(13),
                            Space::new().height(4),
                            text("linux-wallpaper-engine (Rust): AUR / github.com/0xFA11/linux-wallpaperengine").size(10),
                            text("linux-wallpaperengine (C++):   AUR / github.com/Almamu/linux-wallpaperengine").size(10),
                            text("mpvpaper: package manager / github.com/GhostNaN/mpvpaper").size(10),
                        ]
                        .spacing(2)
                        .width(w.clone())
                    ),
                    Space::new().height(20),
                    container(row![
                        Space::new().width(Length::Fill),
                        button(text("Save & Refresh").size(14))
                            .style(theme::btn_primary)
                            .on_press(Message::SaveSettings)
                    ])
                    .width(Length::Fill),
                    Space::new().height(20),
                ]
                .align_x(Alignment::Center)
                .padding(24),
            )
            .style(theme::scrollable_style)
            .height(Length::Fill),
        ]
        .into()
    }

    fn section<'a>(
        &self,
        _title: &'a str,
        body: iced::widget::Column<'a, Message>,
    ) -> Element<'a, Message> {
        let b: Element<_> = body.into();
        container(column![text(_title).size(15), Space::new().height(12), b].spacing(4))
            .padding(20)
            .style(theme::surface_style)
            .width(Length::Fill)
            .into()
    }

    fn input_field<'a>(
        &self,
        _label: &'a str,
        value: &str,
        on_change: fn(String) -> Message,
        placeholder: &str,
        width: impl Into<Length>,
    ) -> iced::widget::Column<'a, Message> {
        column![
            text(_label).size(13),
            text_input(placeholder, value)
                .on_input(on_change)
                .style(theme::text_input_style)
                .width(width.into())
        ]
        .spacing(4)
    }

    fn dropdown<'a>(
        &self,
        label: &'a str,
        opts: &'a [&'a str],
        selected: &'a str,
        msg: fn(String) -> Message,
    ) -> Element<'a, Message> {
        column![
            text(label).size(13),
            pick_list(opts, Some(selected), move |s: &str| msg(s.to_string())),
        ]
        .spacing(4)
        .into()
    }

    /// A row of clickable chips for the currently connected displays,
    /// used by the C++ engine's `Screen Root` field. Clicking a chip
    /// sets the screen root to that display's name. The `*` chip sets
    /// it back to "all displays".
    fn display_chips_row(&self) -> Element<'_, Message> {
        let displays = crate::displays::detect_connected_displays();
        let mut chips: Vec<Element<Message>> = Vec::new();
        // "All" chip
        let all_label = if self.engine_screen_root == "*" || self.engine_screen_root.is_empty() {
            "★ * (all)"
        } else {
            "* (all)"
        };
        chips.push(
            button(text(all_label).size(11))
                .style(theme::btn_secondary)
                .on_press(Message::EngineScreenRootChanged("*".into()))
                .into(),
        );
        match displays {
            Some(names) if !names.is_empty() => {
                for name in names {
                    let label = if self.engine_screen_root == name {
                        format!("★ {}", name)
                    } else {
                        name.clone()
                    };
                    chips.push(
                        button(text(label).size(11))
                            .style(theme::btn_secondary)
                            .on_press(Message::EngineScreenRootChanged(name))
                            .into(),
                    );
                }
            }
            _ => {
                // No detection available — leave only the * chip.
            }
        }
        let label = if chips.len() <= 1 {
            text("Detected displays: (none — install wlr-randr or xrandr)").size(10)
        } else {
            text("Detected displays (click to set):").size(10)
        };
        column![label, row(chips).spacing(6)]
            .spacing(4)
            .into()
    }


}

// ── Persistent IPC reader stream ───────────────────────────────────────────

/// Wraps a `tokio::sync::mpsc::Receiver` as an Iced-compatible `Stream`.
struct TokioMpscStream<T> {
    rx: tokio::sync::mpsc::Receiver<T>,
}

impl<T> Stream for TokioMpscStream<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.rx.poll_recv(cx)
    }
}

/// Creates a subscription stream that connects to the tray IPC socket,
/// reads incoming messages (responses routed via oneshot channels,
/// events forwarded as `Message::IpcEvent`), and reconnects on failure.
fn ipc_reader_stream(_key: &u64) -> TokioMpscStream<Message> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Message>(100);

    let g = IPC_GLOBALS.get().expect("IPC_GLOBALS not set");
    let writer = g.writer.clone();
    let pending = g.pending.clone();

    // Spawn the persistent reader task on the Tokio runtime.
    // It connects, reads events/responses, and reconnects on failure.
    tokio::spawn(async move {
        loop {
            // Connect to tray
            match connect_to_tray().await {
                Ok((reader, write_half)) => {
                    // Store the write half so update() can send requests
                    *writer.lock().await = Some(write_half);
                    let _ = tx.send(Message::IpcConnected).await;

                    let mut reader = BufReader::new(reader);
                    let mut line = String::new();

                    loop {
                        line.clear();
                        match reader.read_line(&mut line).await {
                            Ok(0) => break, // EOF — connection closed
                            Ok(_) => {
                                let trimmed = line.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }
                                let Ok(msg) =
                                    serde_json::from_str::<IpcMessage>(trimmed)
                                else {
                                    continue;
                                };
                                match msg {
                                    IpcMessage::Response(resp) => {
                                        // Route to the waiting oneshot
                                        if let Some(tx_oneshot) =
                                            pending.lock().unwrap().remove(&resp.id)
                                        {
                                            let _ = tx_oneshot.send(resp);
                                        }
                                    }
                                    IpcMessage::Event(evt) => {
                                        if tx.send(Message::IpcEvent(evt)).await.is_err() {
                                            return; // App shutting down
                                        }
                                    }
                                    IpcMessage::Request(_) => {
                                        // GUI doesn't handle requests
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("IPC read error: {e}");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    log::warn!("IPC connect failed: {e}");
                }
            }

            // Connection lost — reset and retry
            *writer.lock().await = None;
            if tx.send(Message::IpcDisconnected).await.is_err() {
                return; // App shutting down
            }

            // Wait before retrying
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });

    TokioMpscStream { rx }
}

/// Synchronously connect to the tray's Unix socket.
/// Blocks for at most 3 seconds.
async fn connect_to_tray() -> Result<(OwnedReadHalf, OwnedWriteHalf), String> {
    // 1. Read socket info file
    let info_path = Config::socket_info_path();
    let info_str = tokio::fs::read_to_string(&info_path)
        .await
        .map_err(|e| format!("Cannot read socket info: {e}"))?;
    let info: serde_json::Value =
        serde_json::from_str(&info_str).map_err(|e| format!("Invalid socket info: {e}"))?;
    let socket_path = info["socket_path"]
        .as_str()
        .ok_or("No socket_path in info")?
        .to_string();

    // 2. Connect with timeout
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::UnixStream::connect(&socket_path),
    )
    .await
    .map_err(|_| "Connect timed out".to_string())?
    .map_err(|e| format!("Cannot connect: {e}"))?;

    Ok(stream.into_split())
}
