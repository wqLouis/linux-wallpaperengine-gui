use iced::widget::{button, column, container, image, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length, Subscription, Task};
use iced::event;
use iced::window;

use crate::config::{Config, ToolsStatus};
use crate::wallpaper::{discover_wallpapers, Wallpaper, WallpaperType};

/// Main application state
pub struct App {
    screen: Screen,
    config: Config,
    tools: ToolsStatus,
    wallpapers: Vec<Wallpaper>,
    /// Steam path input buffer (for settings)
    path_input: String,
    /// Status message shown to the user
    status_message: Option<String>,
    /// Current window width for responsive layout
    window_width: f32,
}

#[derive(Debug, Clone, PartialEq)]
enum Screen {
    Library,
    Settings,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Navigation
    GoToLibrary,
    GoToSettings,

    // Settings
    PathInputChanged(String),
    #[allow(dead_code)]
    BrowsePath,
    AutoDetectPath,
    SaveSettings,

    // Wallpaper actions
    ApplyWallpaper(usize),
    RefreshWallpapers,

    // Async results
    WallpapersLoaded(Vec<Wallpaper>),
    PathSelected(Option<String>),

    // Window events
    WindowResized(f32),
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let config = Config::load();
        let tools = ToolsStatus::detect();
        let path_input = config.steamapps_path.clone();

        let mut app = Self {
            screen: Screen::Library,
            config,
            tools,
            wallpapers: Vec::new(),
            path_input,
            status_message: None,
            window_width: 1200.0,
        };

        let task = if app.config.steamapps_path.is_empty() {
            // No path configured — try auto-detect
            if let Some(auto_path) = Config::auto_detect_steam_path() {
                app.config.steamapps_path = auto_path.clone();
                app.path_input = auto_path;
                app.config.save();
                app.load_wallpapers_task()
            } else {
                // Go to settings so user can configure
                app.screen = Screen::Settings;
                Task::none()
            }
        } else {
            app.load_wallpapers_task()
        };

        (app, task)
    }

    pub fn title(&self) -> String {
        "Wallpaper Engine Manager".to_string()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::GoToLibrary => {
                self.screen = Screen::Library;
                Task::none()
            }
            Message::GoToSettings => {
                self.screen = Screen::Settings;
                self.path_input = self.config.steamapps_path.clone();
                Task::none()
            }

            Message::PathInputChanged(value) => {
                self.path_input = value;
                Task::none()
            }
            Message::BrowsePath => {
                Task::done(Message::PathSelected(None)) // placeholder
            }
            Message::AutoDetectPath => {
                if let Some(path) = Config::auto_detect_steam_path() {
                    self.path_input = path;
                    self.status_message = Some("Steam path auto-detected!".to_string());
                } else {
                    self.status_message = Some("Could not auto-detect Steam path.".to_string());
                }
                Task::none()
            }
            Message::SaveSettings => {
                self.config.steamapps_path = self.path_input.clone();
                self.config.save();
                self.status_message = Some("Settings saved.".to_string());
                // Reload wallpapers with new path
                self.load_wallpapers_task()
            }
            Message::PathSelected(Some(path)) => {
                self.path_input = path;
                Task::none()
            }
            Message::PathSelected(None) => Task::none(),

            Message::ApplyWallpaper(index) => {
                if let Some(wp) = self.wallpapers.get(index) {
                    if !wp.can_apply || wp.file_path.is_none() {
                        self.status_message = Some(format!("Cannot apply '{}' — unsupported format", wp.title));
                        return Task::none();
                    }
                    let file_path = wp.file_path.as_ref().unwrap().to_string_lossy().to_string();
                    let wp_type = wp.wallpaper_type.clone();
                    let title = wp.title.clone();
                    let mpvpaper_available = self.tools.mpvpaper_available;
                    let engine_available = self.tools.engine_available;
                    let assets_path = self.assets_path();

                    return Task::perform(
                        async move {
                            launch_wallpaper(file_path, wp_type, title, mpvpaper_available, engine_available, assets_path).await
                        },
                        |msg| msg,
                    );
                }
                Task::none()
            }

            Message::RefreshWallpapers => {
                self.load_wallpapers_task()
            }

            Message::WallpapersLoaded(wallpapers) => {
                self.wallpapers = wallpapers;
                let count = self.wallpapers.len();
                self.status_message = Some(format!("Loaded {} wallpapers", count));
                Task::none()
            }

            Message::WindowResized(width) => {
                self.window_width = width;
                Task::none()
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        event::listen_with(|event, _status, _id| {
            if let iced::Event::Window(window::Event::Resized(size)) = event {
                Some(Message::WindowResized(size.width))
            } else {
                None
            }
        })
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

    fn top_bar(&self) -> Element<'_, Message> {
        let library_btn = button(text("Library").size(16))
            .style(if self.screen == Screen::Library {
                button::primary
            } else {
                button::secondary
            })
            .on_press(Message::GoToLibrary);

        let settings_btn = button(text("Settings").size(16))
            .style(if self.screen == Screen::Settings {
                button::primary
            } else {
                button::secondary
            })
            .on_press(Message::GoToSettings);

        let refresh_btn = button("↻ Refresh")
            .style(button::secondary)
            .on_press(Message::RefreshWallpapers);

        container(
            row![
                library_btn,
                settings_btn,
                Space::new().width(Length::Fill),
                refresh_btn,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding(12)
        .into()
    }

    fn library_view(&self) -> Element<'_, Message> {
        if self.wallpapers.is_empty() {
            return container(
                column![
                    text("No wallpapers found.").size(18),
                    text("Configure your Steam steamapps path in Settings, then refresh.").size(14),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
        }

        // Responsive: calculate cards per row based on window width
        let card_min_width: f32 = 240.0;
        let padding: f32 = 32.0;
        let spacing: f32 = 16.0;
        let usable = (self.window_width - padding).max(card_min_width);
        let cards_per_row = (usable / card_min_width).floor() as usize;
        let cards_per_row = cards_per_row.max(1);

        // Per-card width so all preview images are uniformly square
        let total_spacing = spacing * (cards_per_row as f32 - 1.0);
        let card_width = ((usable - total_spacing) / cards_per_row as f32).max(120.0);

        let mut rows: Vec<Element<Message>> = Vec::new();
        let mut current_row: Vec<Element<Message>> = Vec::new();

        for (i, wp) in self.wallpapers.iter().enumerate() {
            let card = self.wallpaper_card(i, wp, card_width);
            current_row.push(card);

            if current_row.len() >= cards_per_row {
                let drained: Vec<_> = current_row.drain(..).collect();
                rows.push(row(drained).spacing(16).into());
            }
        }

        if !current_row.is_empty() {
            // Last row: pad with empty space to maintain alignment
            while current_row.len() < cards_per_row {
                current_row.push(Space::new().width(Length::Fill).into());
            }
            rows.push(row(current_row).spacing(16).into());
        }

        let status: Element<Message> = if let Some(ref msg) = self.status_message {
            container(text(msg).size(13))
                .padding(8)
                .into()
        } else {
            container(text("").size(13)).padding(8).into()
        };

        column![
            scrollable(
                column(rows)
                    .spacing(16)
                    .padding(16)
            )
            .height(Length::Fill),
            status,
        ]
        .into()
    }

    fn wallpaper_card<'a>(&self, index: usize, wp: &'a Wallpaper, card_width: f32) -> Element<'a, Message> {
        // Preview image in a square container — all cards get uniform preview size
        let preview: Element<Message> = if let Some(ref preview_path) = wp.preview_path {
            container(
                image(preview_path.clone())
                    .width(Length::Fixed(card_width))
                    .height(Length::Fixed(card_width))
                    .content_fit(iced::ContentFit::Cover),
            )
            .width(Length::Fixed(card_width))
            .height(Length::Fixed(card_width))
            .style(container::dark)
            .into()
        } else {
            container(
                column![
                    text(match wp.wallpaper_type {
                        WallpaperType::Scene => "🎬 Scene",
                        WallpaperType::Video => "🎥 Video",
                        WallpaperType::Unsupported => "🚫 Unsupported",
                    })
                    .size(14),
                    text("No preview").size(12),
                ]
                .align_x(Alignment::Center)
                .spacing(4),
            )
            .width(Length::Fixed(card_width))
            .height(Length::Fixed(card_width))
            .style(container::dark)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
        };

        let title = text(&wp.title).size(14).width(Length::Fill);
        let source_label = text(format!("[{}]", wp.source)).size(11);

        let type_badge = text(match wp.wallpaper_type {
            WallpaperType::Scene => "scene",
            WallpaperType::Video => "video",
            WallpaperType::Unsupported => "unsupported",
        })
        .size(10);

        let apply_btn = if wp.can_apply {
            button("Apply")
                .style(button::primary)
                .on_press(Message::ApplyWallpaper(index))
        } else {
            button("Unavailable")
                .style(button::secondary)
        };

        let card = column![
            preview,
            title,
            row![source_label, Space::new().width(Length::Fill), type_badge]
                .align_y(Alignment::Center),
            apply_btn,
        ]
        .spacing(6)
        .width(Length::Fill);

        container(card)
            .padding(10)
            .style(container::bordered_box)
            .into()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        // Tool status
        let engine_status = if self.tools.engine_available {
            "✅ linux-wallpaper-engine found"
        } else {
            "❌ linux-wallpaper-engine NOT found"
        };
        let mpvpaper_status = if self.tools.mpvpaper_available {
            "✅ mpvpaper found"
        } else {
            "❌ mpvpaper NOT found"
        };

        let auto_detect_btn = button("Auto-detect")
            .on_press(Message::AutoDetectPath);

        let save_btn = button("Save & Refresh")
            .style(button::primary)
            .on_press(Message::SaveSettings);

        let status_text = if let Some(ref msg) = self.status_message {
            text(msg).size(13)
        } else {
            text("").size(13)
        };

        container(
            column![
                text("Settings").size(20),
                Space::new().height(16),

                text("Steam steamapps Path").size(14),
                text("e.g. /home/user/.steam/steam/steamapps").size(11),
                row![
                    text_input("Enter steamapps path...", &self.path_input)
                        .on_input(Message::PathInputChanged)
                        .width(Length::Fill),
                    auto_detect_btn,
                ]
                .spacing(8),
                Space::new().height(8),
                row![
                    Space::new().width(Length::Fill),
                    save_btn,
                ],

                Space::new().height(24),
                text("Required Tools").size(16),
                text(engine_status).size(14),
                text(mpvpaper_status).size(14),
                text("Install linux-wallpaper-engine from AUR or build from source").size(11),
                text("Install mpvpaper from your package manager or https://github.com/GhostNaN/mpvpaper").size(11),

                Space::new().height(16),
                status_text,
            ]
            .spacing(4),
        )
        .padding(24)
        .into()
    }

    fn load_wallpapers_task(&self) -> Task<Message> {
        let workshop = self.config.workshop_path();
        let builtin = self.config.builtin_projects_path();

        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    discover_wallpapers(workshop, builtin)
                })
                .await
                .unwrap_or_default()
            },
            Message::WallpapersLoaded,
        )
    }

    /// Path to wallpaper_engine/assets for texture fallback
    fn assets_path(&self) -> Option<String> {
        self.config.assets_path()
            .map(|p| p.to_string_lossy().to_string())
    }
}

/// Launch a wallpaper using the appropriate program
async fn launch_wallpaper(
    file_path: String,
    wp_type: WallpaperType,
    title: String,
    mpvpaper_available: bool,
    engine_available: bool,
    assets_path: Option<String>,
) -> Message {
    match wp_type {
        WallpaperType::Scene => {
            if !engine_available {
                eprintln!("linux-wallpaper-engine not available");
                return Message::RefreshWallpapers;
            }
            let mut cmd = tokio::process::Command::new("linux-wallpaper-engine");
            cmd.arg("-p").arg(&file_path);
            if let Some(ref assets) = assets_path {
                cmd.arg("--assets-path").arg(assets);
            }
            match cmd.spawn() {
                Ok(_) => eprintln!("Launched scene wallpaper: {}", title),
                Err(e) => eprintln!("Failed to launch: {}", e),
            }
        }
        WallpaperType::Video => {
            if !mpvpaper_available {
                eprintln!("mpvpaper not available");
                return Message::RefreshWallpapers;
            }
            // mpvpaper: mpvpaper [output] [video]
            // Use '*' for all outputs
            match tokio::process::Command::new("mpvpaper")
                .arg("*")
                .arg(&file_path)
                .spawn()
            {
                Ok(_) => eprintln!("Launched video wallpaper: {}", title),
                Err(e) => eprintln!("Failed to launch mpvpaper: {}", e),
            }
        }
        WallpaperType::Unsupported => {
            eprintln!("Cannot launch unsupported wallpaper: {}", title);
        }
    }

    Message::RefreshWallpapers
}
