mod config;
mod displays;
mod gui;
mod ipc;
mod theme;
mod tray;
mod wallpaper;

use gui::GuiApp;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 && args[1] == "--gui" {
        log::info!("Starting GUI mode");
        run_gui();
    } else {
        log::info!("Starting tray mode");
        tray::run_tray();
    }
}

fn dark_theme(_: &GuiApp) -> iced::Theme {
    iced::Theme::custom(
        "Wallpaper Engine",
        iced::theme::Palette {
            background: crate::theme::BG_DEEP,
            text: crate::theme::TEXT_PRIMARY,
            primary: crate::theme::ACCENT,
            success: crate::theme::SUCCESS,
            warning: crate::theme::WARNING,
            danger: crate::theme::ERROR,
        },
    )
}

fn run_gui() {
    iced::application(GuiApp::new, GuiApp::update, GuiApp::view)
        .title(GuiApp::title)
        .subscription(GuiApp::subscription)
        .theme(dark_theme)
        .run()
        .expect("Failed to run GUI");
}
