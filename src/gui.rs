use iced::event;
use iced::widget::{
    Space, button, column, container, image, pick_list, row, scrollable, text, text_input, toggler,
};
use iced::window;
use iced::{Alignment, Element, Length, Subscription, Task};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::config::{Config, EngineParams, MpvpaperParams, ToolsStatus};
use crate::ipc::{IpcCommand, IpcRequest, IpcResponse, TrayStatus};
use crate::theme;
use crate::wallpaper::{Wallpaper, WallpaperType, discover_wallpapers};

pub struct GuiApp {
    screen: Screen,
    config: Config,
    tools: ToolsStatus,
    wallpapers: Vec<Wallpaper>,
    tray_status: TrayStatus,
    path_input: String,
    status_message: Option<String>,
    window_width: f32,
    ipc_connected: bool,
    next_request_id: u64,
    pending_request: Option<(u64, PendingRequest)>,
    engine_mode: String,
    engine_fit_mode: String,
    engine_log_level: String,
    engine_target_fps_str: String,
    engine_no_effects: bool,
    mpv_output: String,
    mpv_options_str: String,
}

#[derive(Debug, Clone, PartialEq)]
enum PendingRequest {
    Status,
    Apply,
    Stop,
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
    EngineModeChanged(String),
    EngineFitModeChanged(String),
    EngineLogLevelChanged(String),
    EngineTargetFpsChanged(String),
    EngineNoEffectsToggled(bool),
    MpvOutputChanged(String),
    MpvOptionsChanged(String),
    ApplyWallpaper(usize),
    StopWallpaper,
    RefreshWallpapers,
    IpcConnected(Result<(), String>),
    IpcResponseReceived(IpcResponse),
    WindowResized(f32),
    Tick,
}

impl GuiApp {
    pub fn new() -> (Self, Task<Message>) {
        let config = Config::load();
        let tools = ToolsStatus::detect();
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

        let app = Self {
            screen: Screen::Library,
            path_input: config.steamapps_path.clone(),
            engine_mode: config.engine.mode.clone(),
            engine_fit_mode: config.engine.fit_mode.clone(),
            engine_log_level: config.engine.log_level.clone(),
            engine_target_fps_str: config
                .engine
                .target_fps
                .map(|f| f.to_string())
                .unwrap_or_default(),
            engine_no_effects: config.engine.no_effects,
            mpv_output: config.mpvpaper.output.clone(),
            mpv_options_str: config.mpvpaper.mpv_options.join(", "),
            config,
            tools,
            wallpapers,
            tray_status: empty_status,
            status_message: Some("Connecting to daemon…".into()),
            window_width: 1200.0,
            ipc_connected: false,
            next_request_id: 1,
            pending_request: None,
        };
        let task = Task::perform(async { connect_to_tray().await }, |r| match r {
            Ok(_) => Message::IpcConnected(Ok(())),
            Err(e) => Message::IpcConnected(Err(e)),
        });
        (app, task)
    }

    pub fn title(&self) -> String {
        "Wallpaper Engine Manager".into()
    }

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
            Message::SaveSettings => {
                let cfg = self.build_config();
                cfg.save();
                self.config = cfg;
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
                let id = self.next_id();
                self.pending_request = Some((id, PendingRequest::Apply));
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
                return Task::perform(send_ipc(id, cmd), Message::IpcResponseReceived);
            }
            Message::StopWallpaper => {
                if !self.ipc_connected {
                    return Task::none();
                }
                let id = self.next_id();
                self.pending_request = Some((id, PendingRequest::Stop));
                return Task::perform(
                    send_ipc(id, IpcCommand::StopWallpaper),
                    Message::IpcResponseReceived,
                );
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
            Message::IpcConnected(r) => match r {
                Ok(()) => {
                    self.ipc_connected = true;
                    self.status_message = Some("Connected".into());
                    let id = self.next_id();
                    self.pending_request = Some((id, PendingRequest::Status));
                    return Task::perform(
                        send_ipc(id, IpcCommand::GetStatus),
                        Message::IpcResponseReceived,
                    );
                }
                Err(e) => {
                    self.ipc_connected = false;
                    self.status_message = Some(format!("Daemon offline: {}", e));
                    Task::none()
                }
            },
            Message::IpcResponseReceived(resp) => {
                if resp.id == 0 {
                    if let Some(d) = resp.data {
                        if let Ok(wps) = serde_json::from_value::<Vec<Wallpaper>>(d) {
                            self.wallpapers = wps;
                            self.status_message =
                                Some(format!("{} wallpapers", self.wallpapers.len()));
                        }
                    }
                    return Task::none();
                }
                let (expected_id, pending) = match self.pending_request.take() {
                    Some(p) => p,
                    None => {
                        if resp.ok {
                            if let Some(d) = resp.data {
                                if let Ok(s) = serde_json::from_value::<TrayStatus>(d) {
                                    self.tray_status = s;
                                }
                            }
                        }
                        return Task::none();
                    }
                };
                if resp.id != expected_id {
                    return Task::none();
                }
                if !resp.ok {
                    self.status_message =
                        Some(format!("Error: {}", resp.error.unwrap_or_default()));
                    return Task::none();
                }
                match pending {
                    PendingRequest::Status => {
                        if let Some(d) = resp.data {
                            if let Ok(s) = serde_json::from_value::<TrayStatus>(d) {
                                self.tray_status = s;
                            }
                        }
                    }
                    PendingRequest::Apply => {
                        self.status_message = Some("Applied".into());
                        let id = self.next_id();
                        self.pending_request = Some((id, PendingRequest::Status));
                        return Task::perform(
                            send_ipc(id, IpcCommand::GetStatus),
                            Message::IpcResponseReceived,
                        );
                    }
                    PendingRequest::Stop => {
                        self.status_message = Some("Stopped".into());
                        self.tray_status.wallpaper_running = false;
                        self.tray_status.current_wallpaper_title = None;
                    }
                }
                Task::none()
            }
            Message::WindowResized(w) => {
                self.window_width = w;
                Task::none()
            }
            Message::Tick => {
                if self.ipc_connected && self.pending_request.is_none() {
                    let id = self.next_id();
                    self.pending_request = Some((id, PendingRequest::Status));
                    return Task::perform(
                        send_ipc(id, IpcCommand::GetStatus),
                        Message::IpcResponseReceived,
                    );
                }
                Task::none()
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            event::listen_with(|ev, _, _| {
                if let iced::Event::Window(window::Event::Resized(s)) = ev {
                    Some(Message::WindowResized(s.width))
                } else {
                    None
                }
            }),
            iced::time::every(std::time::Duration::from_secs(3)).map(|_| Message::Tick),
        ])
    }

    pub fn view(&self) -> Element<'_, Message> {
        column![
            self.top_bar(),
            match self.screen {
                Screen::Library => self.library_view(),
                Screen::Settings => self.settings_view(),
            }
        ]
        .into()
    }

    fn sync_form(&mut self) {
        self.path_input = self.config.steamapps_path.clone();
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
        self.mpv_output = self.config.mpvpaper.output.clone();
        self.mpv_options_str = self.config.mpvpaper.mpv_options.join(", ");
    }

    fn build_config(&self) -> Config {
        Config {
            steamapps_path: self.path_input.clone(),
            engine: EngineParams {
                mode: self.engine_mode.clone(),
                fit_mode: self.engine_fit_mode.clone(),
                log_level: self.engine_log_level.clone(),
                target_fps: self.engine_target_fps_str.trim().parse().ok(),
                no_effects: self.engine_no_effects,
                use_stdin: false,
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
        }
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
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
                    .content_fit(iced::ContentFit::Cover),
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

        let eng_ok = if self.tools.engine_available {
            "✅ linux-wallpaper-engine"
        } else {
            "❌ linux-wallpaper-engine not found"
        };
        let mpv_ok = if self.tools.mpvpaper_available {
            "✅ mpvpaper"
        } else {
            "❌ mpvpaper not found"
        };
        // Effects toggle button
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

        scrollable(
            column![
                // ── Steam path ──
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
                // ── Engine ──
                self.section(
                    "linux-wallpaper-engine",
                    column![
                        row![
                            container(self.dropdown("Output Mode", MODES, &self.engine_mode, Message::EngineModeChanged)).width(hf),
                            Space::new().width(16),
                            container(self.dropdown("Fit Mode", FIT_MODES, &self.engine_fit_mode, Message::EngineFitModeChanged)).width(hf),
                        ].spacing(8),

                        Space::new().height(14),

                        row![
                            container(self.dropdown("Log Level", LOG_LEVELS, &self.engine_log_level, Message::EngineLogLevelChanged)).width(hf),
                            Space::new().width(16),
                            column![].width(hf),
                        ]
                        .spacing(8),
                        Space::new().height(14),
                        row![
                            self.input_field(
                                "Target FPS",
                                &self.engine_target_fps_str,
                                Message::EngineTargetFpsChanged,
                                "unlimited (leave empty)",
                                hf
                            ),
                            Space::new().width(16),
                            column![effects_toggle].width(hf).align_x(Alignment::Center),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    ]
                    .spacing(4)
                    .width(w.clone())
                ),
                Space::new().height(20),
                // ── mpvpaper ──
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
                // ── Tools ──
                self.section(
                    "Tools",
                    column![
                        text(eng_ok).size(13),
                        text(mpv_ok).size(13),
                        Space::new().height(4),
                        text(
                            "linux-wallpaper-engine: AUR / github.com/0xFA11/linux-wallpaperengine"
                        )
                        .size(10),
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
        .height(Length::Fill)
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
}

// ── IPC ──────────────────────────────────────────────────────────────────

async fn connect_to_tray() -> Result<UnixStream, String> {
    let info_path = Config::socket_info_path();
    let info_str = tokio::fs::read_to_string(&info_path)
        .await
        .map_err(|e| format!("Cannot read socket info: {}", e))?;
    let info: serde_json::Value =
        serde_json::from_str(&info_str).map_err(|e| format!("Invalid socket info: {}", e))?;
    let socket_path = info["socket_path"].as_str().ok_or("No socket_path")?;
    UnixStream::connect(socket_path)
        .await
        .map_err(|e| format!("Cannot connect: {}", e))
}

async fn send_ipc(id: u64, cmd: IpcCommand) -> IpcResponse {
    let stream = match connect_to_tray().await {
        Ok(s) => s,
        Err(e) => return IpcResponse::err(id, e),
    };
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let req = IpcRequest::new(id, cmd);
    let json = serde_json::to_string(&req).unwrap() + "\n";
    if writer.write_all(json.as_bytes()).await.is_err() {
        return IpcResponse::err(id, "Send failed");
    }
    line.clear();
    match reader.read_line(&mut line).await {
        Ok(0) => IpcResponse::err(id, "Connection closed"),
        Ok(_) => serde_json::from_str(line.trim())
            .unwrap_or_else(|e| IpcResponse::err(id, format!("Parse: {}", e))),
        Err(e) => IpcResponse::err(id, format!("Read: {}", e)),
    }
}
