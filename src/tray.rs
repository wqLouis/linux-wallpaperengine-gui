use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use crossbeam_channel::{self, Sender};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Child;

use crate::config::Config;
use crate::ipc::{IpcCommand, IpcRequest, IpcResponse, TrayStatus};

pub struct TrayState {
    pub wallpaper_process: Option<Child>,
    pub gui_process: Option<std::process::Child>,
    pub current_wallpaper_title: Option<String>,
    pub gui_running: bool,
}

impl TrayState {
    pub fn new() -> Self {
        Self { wallpaper_process: None, gui_process: None, current_wallpaper_title: None, gui_running: false }
    }

    pub fn status(&self) -> TrayStatus {
        TrayStatus {
            wallpaper_running: self.wallpaper_process.is_some(),
            current_wallpaper_title: self.current_wallpaper_title.clone(),
            gui_running: self.gui_running,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TrayCommand { ShowGui, StopWallpaper, Quit }

struct WpeTray { tx: Sender<TrayCommand> }

impl ksni::Tray for WpeTray {
    fn icon_name(&self) -> String { "preferences-desktop-wallpaper".into() }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let size = 24i32;
        let mut data = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let dx = x - size / 2; let dy = y - size / 2;
                let in_circle = dx * dx + dy * dy <= (size / 2 - 2).max(1).pow(2);
                if in_circle { data.extend_from_slice(&[255, 66, 133, 244]); }
                else { data.extend_from_slice(&[0, 0, 0, 0]); }
            }
        }
        vec![ksni::Icon { width: size, height: size, data }]
    }

    fn title(&self) -> String { "Wallpaper Engine Manager".into() }
    fn id(&self) -> String { "linux-wallpaperengine-gui".into() }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let tx_show = self.tx.clone(); let tx_stop = self.tx.clone(); let tx_quit = self.tx.clone();
        vec![
            MenuItem::Standard(StandardItem { label: "Show GUI".into(), enabled: true, activate: Box::new(move |_| { let _ = tx_show.send(TrayCommand::ShowGui); }), ..Default::default() }),
            MenuItem::Standard(StandardItem { label: "Stop Wallpaper".into(), enabled: true, activate: Box::new(move |_| { let _ = tx_stop.send(TrayCommand::StopWallpaper); }), ..Default::default() }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem { label: "Quit".into(), enabled: true, activate: Box::new(move |_| { let _ = tx_quit.send(TrayCommand::Quit); }), ..Default::default() }),
        ]
    }

    fn activate(&mut self, _x: i32, _y: i32) { let _ = self.tx.send(TrayCommand::ShowGui); }
}

pub fn run_tray() -> ! {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<TrayCommand>();
    let state = Arc::new(Mutex::new(TrayState::new()));

    spawn_gui_process(&state);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2).enable_all().build().expect("tokio runtime");

    rt.block_on(async move {
        use ksni::TrayMethods;
        let tray = WpeTray { tx: cmd_tx.clone() };
        let tray_handle = tray.spawn().await.expect("ksni tray service");

        let listener = start_ipc_server().await;

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, _)) => {
                            let state = Arc::clone(&state);
                            let tx = cmd_tx.clone();
                            tokio::spawn(async move { handle_ipc_client(stream, state, tx).await; });
                        }
                        Err(e) => log::error!("IPC accept error: {e}"),
                    }
                }
                Ok(cmd) = async { cmd_rx.recv() } => {
                    handle_tray_command(cmd, &state, &cmd_tx, &tray_handle).await;
                }
            }
        }
    });

    std::process::exit(0);
}

async fn start_ipc_server() -> UnixListener {
    let socket_path = Config::socket_path();
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).expect("bind IPC socket");
    let info = serde_json::json!({ "socket_path": socket_path.to_string_lossy() });
    let info_path = Config::socket_info_path();
    if let Ok(json) = serde_json::to_string(&info) { let _ = std::fs::write(&info_path, json); }
    log::info!("IPC listening on {}", socket_path.display());
    listener
}

fn spawn_gui_process(state: &Arc<Mutex<TrayState>>) {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("linux-wallpaperengine-gui"));
    match std::process::Command::new(&exe).arg("--gui")
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    {
        Ok(child) => {
            log::info!("GUI spawned pid={}", child.id());
            if let Ok(mut s) = state.lock() { s.gui_process = Some(child); s.gui_running = true; }
        }
        Err(e) => log::error!("Failed to spawn GUI: {e}"),
    }
}

fn kill_gui_process(state: &Arc<Mutex<TrayState>>) {
    if let Ok(mut s) = state.lock() {
        if let Some(mut child) = s.gui_process.take() {
            log::info!("Killing GUI pid={}", child.id());
            let _ = child.kill(); let _ = child.wait();
        }
        s.gui_running = false;
    }
}

async fn handle_ipc_client(stream: UnixStream, state: Arc<Mutex<TrayState>>, _tx: Sender<TrayCommand>) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    log::info!("GUI connected via IPC");

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                if let Ok(mut s) = state.lock() { s.gui_running = false; s.gui_process = None; }
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }
                let request: IpcRequest = match serde_json::from_str(trimmed) {
                    Ok(req) => req,
                    Err(e) => {
                        let resp = IpcResponse::err(0, format!("Parse: {e}"));
                        let json = serde_json::to_string(&resp).unwrap() + "\n";
                        let _ = writer.write_all(json.as_bytes()).await;
                        continue;
                    }
                };
                log::debug!("IPC req id={} cmd={:?}", request.id, request.cmd);
                let response = process_ipc_request(request, &state).await;
                let json = serde_json::to_string(&response).unwrap() + "\n";
                if writer.write_all(json.as_bytes()).await.is_err() { break; }
            }
            Err(e) => { log::error!("IPC read error: {e}"); break; }
        }
    }
}

async fn process_ipc_request(request: IpcRequest, state: &Arc<Mutex<TrayState>>) -> IpcResponse {
    let id = request.id;
    match request.cmd {
        IpcCommand::GetStatus => {
            let status = state.lock().unwrap().status();
            serde_json::to_value(status).map(|v| IpcResponse::ok(id, Some(v)))
                .unwrap_or_else(|e| IpcResponse::err(id, format!("serialize: {e}")))
        }
        IpcCommand::ApplyScene { file_path, title } => {
            let old = state.lock().unwrap().wallpaper_process.take();
            if let Some(mut c) = old { let _ = c.kill().await; let _ = c.wait().await; }
            let mut s = state.lock().unwrap();
            let config = Config::load();
            let engine = &config.engine;
            let mut cmd = tokio::process::Command::new("linux-wallpaper-engine");
            cmd.arg("-p").arg(&file_path).arg("-m").arg(&engine.mode)
                .arg("--fit-mode").arg(&engine.fit_mode).arg("-l").arg(&engine.log_level);
            if engine.no_effects { cmd.arg("--no-effects"); }
            if let Some(fps) = engine.target_fps { cmd.arg("--target-fps").arg(fps.to_string()); }
            if let Some(ref assets) = config.assets_path() { cmd.arg("--assets-path").arg(assets); }
            cmd.stdout(Stdio::null()).stderr(Stdio::null()).stdin(Stdio::null());
            match cmd.spawn() {
                Ok(child) => {
                    log::info!("Launched scene pid={} title={title}", child.id().unwrap_or(0));
                    s.wallpaper_process = Some(child); s.current_wallpaper_title = Some(title);
                    IpcResponse::ok(id, None)
                }
                Err(e) => IpcResponse::err(id, format!("launch: {e}")),
            }
        }
        IpcCommand::ApplyVideo { file_path, title } => {
            let old = state.lock().unwrap().wallpaper_process.take();
            if let Some(mut c) = old { let _ = c.kill().await; let _ = c.wait().await; }
            let mut s = state.lock().unwrap();
            let config = Config::load();
            let mpv = &config.mpvpaper;
            let mut cmd = tokio::process::Command::new("mpvpaper");
            cmd.arg(&mpv.output).arg(&file_path);
            for opt in &mpv.mpv_options { cmd.arg("-o").arg(opt); }
            cmd.stdout(Stdio::null()).stderr(Stdio::null()).stdin(Stdio::null());
            match cmd.spawn() {
                Ok(child) => {
                    log::info!("Launched video pid={} title={title}", child.id().unwrap_or(0));
                    s.wallpaper_process = Some(child); s.current_wallpaper_title = Some(title);
                    IpcResponse::ok(id, None)
                }
                Err(e) => IpcResponse::err(id, format!("launch mpvpaper: {e}")),
            }
        }
        IpcCommand::StopWallpaper => {
            let child = state.lock().unwrap().wallpaper_process.take();
            if let Some(mut c) = child { let _ = c.kill().await; let _ = c.wait().await; }
            state.lock().unwrap().current_wallpaper_title = None;
            log::info!("Wallpaper stopped");
            IpcResponse::ok(id, None)
        }
        IpcCommand::Quit => {
            let child = state.lock().unwrap().wallpaper_process.take();
            if let Some(mut c) = child { let _ = c.kill().await; let _ = c.wait().await; }
            IpcResponse::ok(id, None);
            log::info!("Shutdown by GUI");
            kill_gui_process(state);
            std::process::exit(0);
        }
    }
}

async fn handle_tray_command(cmd: TrayCommand, state: &Arc<Mutex<TrayState>>, _tx: &Sender<TrayCommand>, _h: &ksni::Handle<WpeTray>) {
    match cmd {
        TrayCommand::ShowGui => {
            let alive = state.lock().unwrap().gui_process.as_mut().map_or(false, |c| {
                matches!(c.try_wait(), Ok(None))
            });
            if !alive {
                if let Ok(mut s) = state.lock() { s.gui_running = false; s.gui_process = None; }
                spawn_gui_process(state);
            } else {
                log::debug!("ShowGui: GUI already running");
            }
        }
        TrayCommand::StopWallpaper => {
            let child = state.lock().unwrap().wallpaper_process.take();
            if let Some(mut c) = child { let _ = c.kill().await; let _ = c.wait().await; }
            state.lock().unwrap().current_wallpaper_title = None;
            log::info!("Wallpaper stopped (tray menu)");
        }
        TrayCommand::Quit => {
            log::info!("Quit from tray menu");
            let child = state.lock().unwrap().wallpaper_process.take();
            if let Some(mut c) = child { let _ = c.kill().await; let _ = c.wait().await; }
            kill_gui_process(state);
            let _ = std::fs::remove_file(Config::socket_path());
            let _ = std::fs::remove_file(Config::socket_info_path());
            std::process::exit(0);
        }
    }
}
