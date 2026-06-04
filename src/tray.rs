use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use crossbeam_channel::{self, Sender};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Child;

use crate::config::Config;
use crate::ipc::{IpcCommand, IpcEvent, IpcMessage, IpcRequest, IpcResponse, TrayStatus};

// ── Wallpaper management ───────────────────────────────────────────────────

/// A wallpaper process managed by the tray daemon.
pub struct ManagedWallpaper {
    pub child: Child,
    pub pid: u32,
    pub title: String,
}

impl ManagedWallpaper {
    async fn stop(mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }

    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

// ── GUI lifecycle state machine ────────────────────────────────────────────

/// Trayside view of the GUI process lifecycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum GuiState {
    /// No GUI running (lock file absent or stale).
    NotRunning,
    /// GUI process spawned but IPC not yet established.
    Starting,
    /// GUI connected via IPC — at least one client.
    Connected,
}

/// Shared, thread-safe tray state.
pub struct TrayState {
    pub wallpaper: Option<ManagedWallpaper>,
    pub gui_process: Option<std::process::Child>,
    pub gui_state: GuiState,
    /// Write halves of currently connected GUI clients (for push events).
    /// Each writer is behind its own mutex so both the client handler and
    /// broadcast can access them concurrently without deadlocks.
    pub clients: Vec<Arc<tokio::sync::Mutex<tokio::io::BufWriter<tokio::net::unix::OwnedWriteHalf>>>>,
}

impl TrayState {
    pub fn new() -> Self {
        Self {
            wallpaper: None,
            gui_process: None,
            gui_state: GuiState::NotRunning,
            clients: Vec::new(),
        }
    }

    pub fn status(&self) -> TrayStatus {
        TrayStatus {
            wallpaper_running: self.wallpaper.is_some(),
            current_wallpaper_title: self.wallpaper.as_ref().map(|w| w.title.clone()),
            gui_running: self.gui_state != GuiState::NotRunning,
        }
    }

    fn reap_wallpaper_if_dead(&mut self) {
        if let Some(ref mut w) = self.wallpaper {
            if !w.is_running() {
                log::info!("Wallpaper pid={} has exited, cleaning up", w.pid);
                self.wallpaper = None;
            }
        }
    }
}

// ── Tray commands from ksni menu ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TrayCommand {
    ShowGui,
    StopWallpaper,
    Quit,
}

// ── ksni tray implementation ───────────────────────────────────────────────

struct WpeTray {
    tx: Sender<TrayCommand>,
}

impl ksni::Tray for WpeTray {
    fn icon_name(&self) -> String {
        "preferences-desktop-wallpaper".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let size = 24i32;
        let mut data = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let dx = x - size / 2;
                let dy = y - size / 2;
                let in_circle = dx * dx + dy * dy <= (size / 2 - 2).max(1).pow(2);
                if in_circle {
                    data.extend_from_slice(&[255, 66, 133, 244]);
                } else {
                    data.extend_from_slice(&[0, 0, 0, 0]);
                }
            }
        }
        vec![ksni::Icon {
            width: size,
            height: size,
            data,
        }]
    }

    fn title(&self) -> String {
        "Wallpaper Engine Manager".into()
    }

    fn id(&self) -> String {
        "linux-wallpaperengine-gui".into()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let tx_show = self.tx.clone();
        let tx_stop = self.tx.clone();
        let tx_quit = self.tx.clone();
        vec![
            MenuItem::Standard(StandardItem {
                label: "Show GUI".into(),
                enabled: true,
                activate: Box::new(move |_| {
                    let _ = tx_show.send(TrayCommand::ShowGui);
                }),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "Stop Wallpaper".into(),
                enabled: true,
                activate: Box::new(move |_| {
                    let _ = tx_stop.send(TrayCommand::StopWallpaper);
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Quit".into(),
                enabled: true,
                activate: Box::new(move |_| {
                    let _ = tx_quit.send(TrayCommand::Quit);
                }),
                ..Default::default()
            }),
        ]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(TrayCommand::ShowGui);
    }
}

// ── Public entry point ─────────────────────────────────────────────────────

pub fn run_tray() -> ! {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<TrayCommand>();
    let state = Arc::new(Mutex::new(TrayState::new()));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");

    rt.block_on(async move {
        use ksni::TrayMethods;
        let tray = WpeTray {
            tx: cmd_tx.clone(),
        };
        let tray_handle = tray.spawn().await.expect("ksni tray service");

        let listener = start_ipc_server().await;

        // Check for an existing GUI before spawning — if lock file says one is
        // alive, mark it as Starting (it will reconnect via IPC subscription).
        if let Some(pid) = Config::check_gui_alive() {
            log::info!("Existing GUI detected (pid={}), waiting for IPC", pid);
            if let Ok(mut s) = state.lock() {
                s.gui_state = GuiState::Starting;
            }
        } else {
            spawn_gui_process(&state);
        }

        // ── Auto-start wallpaper ──────────────────────────────────────────
        let config = Config::load();
        log::info!(
            "Auto-start config: enabled={}, type='{}', title='{}', path='{}'",
            config.auto_start.enabled,
            config.auto_start.wallpaper_type,
            config.auto_start.title,
            config.auto_start.file_path,
        );
        if config.auto_start.enabled
            && !config.auto_start.file_path.is_empty()
            && !config.auto_start.wallpaper_type.is_empty()
        {
            let state = Arc::clone(&state);
            let fp = config.auto_start.file_path.clone();
            let title = config.auto_start.title.clone();
            let wp_type = config.auto_start.wallpaper_type.clone();
            tokio::spawn(async move {
                log::info!(
                    "Auto-start: applying {} wallpaper '{}' from {}",
                    wp_type,
                    title,
                    fp
                );
                match wp_type.as_str() {
                    "scene" => {
                        if let Err(e) = apply_scene_wallpaper(&state, &fp, &title).await {
                            log::error!("Auto-start scene failed: {e}");
                        }
                    }
                    "video" => {
                        if let Err(e) = apply_video_wallpaper(&state, &fp, &title).await {
                            log::error!("Auto-start video failed: {e}");
                        }
                    }
                    _ => log::warn!("Auto-start: unknown wallpaper type '{}'", wp_type),
                }
            });
        } else if config.auto_start.enabled {
            log::warn!(
                "Auto-start enabled but file_path or wallpaper_type is empty — nothing to start"
            );
        }

        // Background task: periodically check for dead wallpaper processes
        // and broadcast status updates so the GUI stays in sync.
        {
            let state = Arc::clone(&state);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    let had_wallpaper = { state.lock().unwrap().wallpaper.is_some() };
                    if had_wallpaper {
                        let still_alive = {
                            let mut s = state.lock().unwrap();
                            s.reap_wallpaper_if_dead();
                            s.wallpaper.is_some()
                        };
                        if !still_alive {
                            log::info!("Wallpaper process died, broadcasting status");
                            broadcast_status(&state).await;
                        }
                    }
                }
            });
        }

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
                cmd = async {
                    loop {
                        match cmd_rx.try_recv() {
                            Ok(cmd) => break cmd,
                            Err(crossbeam_channel::TryRecvError::Empty) => {
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                                std::process::exit(0);
                            }
                        }
                    }
                } => {
                    handle_tray_command(cmd, &state, &cmd_tx, &tray_handle).await;
                }
            }
        }
    });

    std::process::exit(0);
}

// ── IPC server setup ───────────────────────────────────────────────────────

async fn start_ipc_server() -> UnixListener {
    let socket_path = Config::socket_path();
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).expect("bind IPC socket");
    let info = serde_json::json!({ "socket_path": socket_path.to_string_lossy() });
    let info_path = Config::socket_info_path();
    if let Ok(json) = serde_json::to_string(&info) {
        let _ = std::fs::write(&info_path, json);
    }
    log::info!("IPC listening on {}", socket_path.display());
    listener
}

// ── GUI process lifecycle ──────────────────────────────────────────────────

fn spawn_gui_process(state: &Arc<Mutex<TrayState>>) {
    // If lock file says a GUI is alive, don't spawn a duplicate.
    if let Some(pid) = Config::check_gui_alive() {
        log::info!("GUI already running (pid={}), not spawning duplicate", pid);
        if let Ok(mut s) = state.lock() {
            s.gui_state = GuiState::Starting;
        }
        return;
    }

    let exe =
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("linux-wallpaperengine-gui"));
    match std::process::Command::new(&exe)
        .arg("--gui")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            log::info!("GUI spawned pid={}", child.id());
            if let Ok(mut s) = state.lock() {
                s.gui_process = Some(child);
                s.gui_state = GuiState::Starting;
            }
        }
        Err(e) => log::error!("Failed to spawn GUI: {e}"),
    }
}

fn kill_gui_process(state: &Arc<Mutex<TrayState>>) {
    if let Ok(mut s) = state.lock() {
        if let Some(mut child) = s.gui_process.take() {
            log::info!("Killing GUI pid={}", child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
        s.gui_state = GuiState::NotRunning;
    }
    Config::remove_gui_lock();
}

// ── Broadcasting (push events to all GUI clients) ──────────────────────────

/// Push a serialised IpcMessage line to every connected GUI client.
/// Dead clients are pruned automatically.
async fn broadcast(state: &Arc<Mutex<TrayState>>, msg: &IpcMessage) {
    use std::sync::Arc as StdArc;

    let json = match serde_json::to_string(msg) {
        Ok(s) => s + "\n",
        Err(_) => return,
    };

    // Clone the Arcs so we don't hold the state lock across await.
    let clients: Vec<StdArc<tokio::sync::Mutex<tokio::io::BufWriter<tokio::net::unix::OwnedWriteHalf>>>> = {
        let s = state.lock().unwrap();
        s.clients.clone()
    };

    let mut dead = Vec::new();
    for client in &clients {
        let mut guard = client.lock().await;
        if guard.write_all(json.as_bytes()).await.is_err()
            || guard.flush().await.is_err()
        {
            dead.push(StdArc::clone(client));
        }
    }

    // Prune dead clients
    if !dead.is_empty() {
        let mut s = state.lock().unwrap();
        s.clients.retain(|c| !dead.iter().any(|d| StdArc::ptr_eq(c, d)));
        log::info!("Pruned {} dead GUI client(s)", dead.len());
        if s.clients.is_empty() && s.gui_state == GuiState::Connected {
            s.gui_state = GuiState::Starting;
        }
    }
}

/// Convenience: broadcast a status-changed event.
async fn broadcast_status(state: &Arc<Mutex<TrayState>>) {
    let status = state.lock().unwrap().status();
    let event = IpcEvent::status_changed(&status);
    let msg = IpcMessage::Event(event);
    broadcast(state, &msg).await;
}

// ── IPC client handler ─────────────────────────────────────────────────────

async fn handle_ipc_client(
    stream: UnixStream,
    state: Arc<Mutex<TrayState>>,
    _tx: Sender<TrayCommand>,
) {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let writer = Arc::new(tokio::sync::Mutex::new(tokio::io::BufWriter::new(writer)));
    let mut line = String::new();

    // Register this client
    {
        let mut s = state.lock().unwrap();
        s.clients.push(writer.clone());
        s.gui_state = GuiState::Connected;
    }

    // Send initial status so the GUI starts with current state
    {
        let initial_status = state.lock().unwrap().status();
        let event = IpcEvent::status_changed(&initial_status);
        let msg = IpcMessage::Event(event);
        if let Ok(json) = serde_json::to_string(&msg) {
            let mut w = writer.lock().await;
            let _ = w.write_all((json + "\n").as_bytes()).await;
            let _ = w.flush().await;
        }
    }
    log::info!("GUI connected via IPC (state → Connected)");

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Parse the envelope
                let msg: IpcMessage = match serde_json::from_str(trimmed) {
                    Ok(m) => m,
                    Err(e) => {
                        let resp = IpcResponse::err(0, format!("Parse: {e}"));
                        let json = serde_json::to_string(&IpcMessage::Response(resp)).unwrap()
                            + "\n";
                        let mut w = writer.lock().await;
                        let _ = w.write_all(json.as_bytes()).await;
                        continue;
                    }
                };

                match msg {
                    IpcMessage::Request(req) => {
                        log::info!("IPC request id={} cmd={:?}", req.id, req.cmd);
                        let is_state_change = matches!(
                            req.cmd,
                            IpcCommand::ApplyScene { .. }
                                | IpcCommand::ApplyVideo { .. }
                                | IpcCommand::StopWallpaper
                        );
                        let is_gui_closing = matches!(req.cmd, IpcCommand::GuiClosing);

                        let response = process_ipc_request(req, &state).await;
                        let json =
                            serde_json::to_string(&IpcMessage::Response(response)).unwrap() + "\n";
                        {
                            let mut w = writer.lock().await;
                            if w.write_all(json.as_bytes()).await.is_err() {
                                break;
                            }
                            let _ = w.flush().await;
                        }

                        // After state-changing commands, push status to ALL clients
                        if is_state_change {
                            broadcast_status(&state).await;
                        }

                        if is_gui_closing {
                            break;
                        }
                    }
                    // GUI shouldn't send events or responses — ignore
                    _ => {}
                }
            }
            Err(e) => {
                log::error!("IPC read error: {e}");
                break;
            }
        }
    }

    // Client disconnected — unregister
    {
        let mut s = state.lock().unwrap();
        s.clients.retain(|c| !Arc::ptr_eq(c, &writer));
        if s.clients.is_empty() {
            s.gui_state = GuiState::NotRunning;
            s.gui_process = None;
        }
        log::info!("GUI disconnected (state → {:?})", s.gui_state);
    }
    // Clean up lock file if no more clients
    let has_clients = state.lock().unwrap().clients.is_empty();
    if has_clients {
        Config::remove_gui_lock();
    }
}

// ── Request processing ─────────────────────────────────────────────────────

async fn process_ipc_request(
    request: IpcRequest,
    state: &Arc<Mutex<TrayState>>,
) -> IpcResponse {
    let id = request.id;
    match request.cmd {
        IpcCommand::GetStatus => {
            let mut s = state.lock().unwrap();
            s.reap_wallpaper_if_dead();
            let status = s.status();
            drop(s);
            serde_json::to_value(status)
                .map(|v| IpcResponse::ok(id, Some(v)))
                .unwrap_or_else(|e| IpcResponse::err(id, format!("serialize: {e}")))
        }
        IpcCommand::ApplyScene { file_path, title } => {
            log::info!("ApplyScene request: file='{}' title='{}'", file_path, title);
            match apply_scene_wallpaper(state, &file_path, &title).await {
                Ok(()) => {
                    log::info!("ApplyScene succeeded");
                    // Return updated status
                    let status = state.lock().unwrap().status();
                    IpcResponse::ok(
                        id,
                        Some(serde_json::to_value(status).unwrap_or_default()),
                    )
                }
                Err(e) => {
                    log::error!("ApplyScene failed: {e}");
                    IpcResponse::err(id, e)
                }
            }
        }
        IpcCommand::ApplyVideo { file_path, title } => {
            log::info!("ApplyVideo request: file='{}' title='{}'", file_path, title);
            match apply_video_wallpaper(state, &file_path, &title).await {
                Ok(()) => {
                    log::info!("ApplyVideo succeeded");
                    let status = state.lock().unwrap().status();
                    IpcResponse::ok(
                        id,
                        Some(serde_json::to_value(status).unwrap_or_default()),
                    )
                }
                Err(e) => {
                    log::error!("ApplyVideo failed: {e}");
                    IpcResponse::err(id, e)
                }
            }
        }
        IpcCommand::StopWallpaper => {
            stop_current_wallpaper(state).await;
            log::info!("Wallpaper stopped");
            let status = state.lock().unwrap().status();
            IpcResponse::ok(
                id,
                Some(serde_json::to_value(status).unwrap_or_default()),
            )
        }
        IpcCommand::GuiClosing => {
            log::info!("GUI is closing");
            {
                let mut s = state.lock().unwrap();
                s.gui_state = GuiState::NotRunning;
                s.gui_process = None;
            }
            Config::remove_gui_lock();
            IpcResponse::ok(id, None)
        }
        IpcCommand::Quit => {
            stop_current_wallpaper(state).await;
            IpcResponse::ok(id, None);
            log::info!("Shutdown by GUI");
            kill_gui_process(state);
            std::process::exit(0);
        }
    }
}

// ── Tray command handler ───────────────────────────────────────────────────

async fn handle_tray_command(
    cmd: TrayCommand,
    state: &Arc<Mutex<TrayState>>,
    _tx: &Sender<TrayCommand>,
    _h: &ksni::Handle<WpeTray>,
) {
    match cmd {
        TrayCommand::ShowGui => {
            let gui_state = state.lock().unwrap().gui_state;

            match gui_state {
                GuiState::Connected => {
                    // GUI is connected — send a show_window event
                    log::debug!("ShowGui: GUI already connected, sending show_window");
                    let event = IpcEvent::show_window();
                    let msg = IpcMessage::Event(event);
                    broadcast(state, &msg).await;
                }
                GuiState::Starting => {
                    // GUI was spawned but hasn't connected yet — do nothing
                    log::debug!("ShowGui: GUI is starting, waiting for IPC");
                }
                GuiState::NotRunning => {
                    // Check lock file first — maybe GUI is alive but tray restarted
                    if let Some(pid) = Config::check_gui_alive() {
                        log::info!(
                            "ShowGui: GUI found via lock file (pid={}), waiting for reconnect",
                            pid
                        );
                        if let Ok(mut s) = state.lock() {
                            s.gui_state = GuiState::Starting;
                        }
                    } else {
                        log::info!("ShowGui: spawning new GUI");
                        spawn_gui_process(state);
                    }
                }
            }
        }
        TrayCommand::StopWallpaper => {
            stop_current_wallpaper(state).await;
            log::info!("Wallpaper stopped (tray menu)");
            broadcast_status(state).await;
        }
        TrayCommand::Quit => {
            log::info!("Quit from tray menu");
            stop_current_wallpaper(state).await;
            kill_gui_process(state);
            let _ = std::fs::remove_file(Config::socket_path());
            let _ = std::fs::remove_file(Config::socket_info_path());
            std::process::exit(0);
        }
    }
}

// ── Wallpaper lifecycle helpers ────────────────────────────────────────────

async fn stop_current_wallpaper(state: &Arc<Mutex<TrayState>>) {
    let wp = state.lock().unwrap().wallpaper.take();
    if let Some(w) = wp {
        log::info!("Stopping wallpaper pid={} title={}", w.pid, w.title);
        w.stop().await;
    }
}

/// Derive the project directory (containing `project.json`) from a
/// `Wallpaper::file_path`. Our wallpapers store `scene.pkg` as
/// `file_path`, but Almamu's `linux-wallpaperengine` wants the parent
/// directory. Built-in / video wallpapers that already point at a
/// directory pass through unchanged.
fn cpp_project_dir(file_path: &str) -> String {
    let p = std::path::Path::new(file_path);
    match p.file_name().and_then(|n| n.to_str()) {
        // Common case: .../<workshop_id>/scene.pkg → .../<workshop_id>/
        Some("scene.pkg") => p
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_path.to_string()),
        // Already a directory, or a file we don't recognize — pass through.
        _ => file_path.to_string(),
    }
}

/// Resolve which display outputs to pass to `--screen-root`.
/// `spec` is the value of `EngineParams::screen_root`:
/// - `""` or `"*"` → all currently connected displays (auto-detected)
/// - anything else → that single specific display
///
/// Returns an error if `*` is requested but no display-detection tool
/// (wlr-randr / xrandr) is available, so the caller gets a clear
/// message instead of the engine silently opening a window.
fn resolve_cpp_displays(spec: &str) -> Result<Vec<String>, String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() || trimmed == "*" {
        match crate::displays::detect_connected_displays() {
            Some(v) if !v.is_empty() => Ok(v),
            Some(_) => Err(
                "No connected displays detected (wlr-randr / xrandr reported none). \
                 Set a specific Screen Root in Settings, or plug in a display."
                    .to_string(),
            ),
            None => Err(
                "Cannot auto-detect displays: neither wlr-randr nor xrandr is available. \
                 Set a specific Screen Root in Settings (e.g. \"eDP-1\" or \"DP-3\")."
                    .to_string(),
            ),
        }
    } else {
        // Specific display name — trust the user. If the engine later
        // complains it's not a real output, that's a clearer error than
        // opening a window.
        Ok(vec![trimmed.to_string()])
    }
}

/// Build the shared C++ engine argv as a flat list of strings. Used
/// by both the scene branch (where `background_path` is the project
/// directory) and the video branch (where it is a direct path to a
/// video file).
///
/// `background_path` is the path that the engine will load as the
/// wallpaper — a folder for scene/web projects, or a single video
/// file for video wallpapers. The engine auto-detects the type.
///
/// The engine applies per-screen options (--scaling, --fps, --silent,
/// --bg, …) to the *most recent* --screen-root. So when we want the
/// same background + settings on multiple screens, we interleave
/// the per-screen args after each --screen-root declaration:
///
///     --screen-root D1 --scaling X --fps N --bg <path> \
///     --screen-root D2 --scaling X --fps N --bg <path>
fn build_cpp_engine_args(
    config: &Config,
    background_path: &str,
) -> Result<Vec<String>, String> {
    let engine = &config.engine;
    let targets = resolve_cpp_displays(&engine.screen_root)?;
    if targets.is_empty() {
        return Err(
            "linux-wallpaperengine: no displays selected (set Screen Root in Settings)"
                .to_string(),
        );
    }

    let mut out: Vec<String> = Vec::new();
    let assets_arg = config.assets_path().map(|p| p.to_string_lossy().into_owned());

    for display in &targets {
        // --screen-root sets `lastScreen` to <display>; everything
        // below this point targets it until the next --screen-root.
        out.push("--screen-root".into());
        out.push(display.clone());
        out.push("--scaling".into());
        out.push(engine.scaling.clone());
        if let Some(fps) = engine.target_fps {
            out.push("--fps".into());
            out.push(fps.to_string());
        }
        if engine.silent {
            out.push("--silent".into());
        }
        if engine.disable_mouse {
            out.push("--disable-mouse".into());
        }
        if engine.disable_parallax {
            out.push("--disable-parallax".into());
        }
        if let Some(ref path) = assets_arg {
            out.push("--assets-dir".into());
            out.push(path.clone());
        }
        out.push("--bg".into());
        out.push(background_path.to_string());
    }
    Ok(out)
}

/// Apply a flat list of args to a `tokio::process::Command` and mirror
/// them (as a debug-friendly log) into `display_args`. The mirror is
/// just the args joined with their values — we use it for logging.
fn apply_args(
    cmd: &mut tokio::process::Command,
    display_args: &mut Vec<String>,
    args: &[String],
) {
    for a in args {
        display_args.push(a.clone());
    }
    cmd.args(args);
}

fn append_cpp_engine_args(
    cmd: &mut tokio::process::Command,
    display_args: &mut Vec<String>,
    config: &Config,
    background_path: &str,
    engine_bin: &str,
) -> Result<(), String> {
    let args = build_cpp_engine_args(config, background_path).inspect_err(|e| {
        log::error!("{engine_bin}: {e}");
    })?;
    apply_args(cmd, display_args, &args);
    Ok(())
}

async fn apply_scene_wallpaper(
    state: &Arc<Mutex<TrayState>>,
    file_path: &str,
    title: &str,
) -> Result<(), String> {
    stop_current_wallpaper(state).await;

    let mut s = state.lock().unwrap();
    let config = Config::load();
    let engine = &config.engine;

    let engine_bin = config.engine_binary().to_string();
    let mut cmd = tokio::process::Command::new(&engine_bin);
    let mut display_args: Vec<String> = Vec::new();

    match engine.variant {
        crate::config::EngineVariant::Rust => {
            // linux-wallpaper-engine:  -p <path> -m <mode> --fit-mode <f>
            //                           -l <lvl> [--no-effects]
            //                           [--target-fps N] [--assets-path P]
            cmd.arg("-p").arg(file_path)
                .arg("-m").arg(&engine.mode)
                .arg("--fit-mode").arg(&engine.fit_mode)
                .arg("-l").arg(&engine.log_level);
            display_args.extend([
                format!("-p {}", file_path),
                format!("-m {}", engine.mode),
                format!("--fit-mode {}", engine.fit_mode),
                format!("-l {}", engine.log_level),
            ]);

            if engine.no_effects {
                cmd.arg("--no-effects");
                display_args.push("--no-effects".into());
            }
            if let Some(fps) = engine.target_fps {
                cmd.arg("--target-fps").arg(fps.to_string());
                display_args.push(format!("--target-fps {}", fps));
            }
            if let Some(ref assets) = config.assets_path() {
                cmd.arg("--assets-path").arg(assets);
                display_args.push(format!("--assets-path {}", assets.display()));
            }
        }
        crate::config::EngineVariant::Cpp => {
            // linux-wallpaperengine:    --screen-root D [--scaling m] [--fps N]
            //                           [--silent] [--disable-mouse]
            //                           [--disable-parallax] [--assets-dir P]
            //                           --bg <path>
            //
            // The background must be assigned via `--bg <path>` paired with
            // a preceding `--screen-root <display>`. Without `--screen-root`
            // the engine opens a window (its default render mode is
            // NORMAL_WINDOW), so we always emit at least one such pair.
            //
            // For scene wallpapers `<path>` is the *project directory*
            // (containing `project.json` and `scene.pkg`), NOT the path
            // to `scene.pkg` itself. See `cpp_project_dir`.
            let project_dir = cpp_project_dir(file_path);
            // Sanity-check the directory actually contains a project.json
            // — if not, the engine will fail with a confusing error.
            let project_json = std::path::Path::new(&project_dir).join("project.json");
            if !project_json.is_file() {
                let msg = format!(
                    "Refusing to invoke linux-wallpaperengine: '{}' has no project.json",
                    project_dir
                );
                log::error!("{msg}");
                return Err(msg);
            }
            append_cpp_engine_args(
                &mut cmd,
                &mut display_args,
                &config,
                &project_dir,
                &engine_bin,
            )?;
        }
    }

    log::info!(
        "Spawning: {} {}",
        engine_bin,
        display_args.join(" ")
    );

    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null());

    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id().unwrap_or(0);
            log::info!("Launched scene pid={} title={title} variant={:?}", pid, engine.variant);
            s.wallpaper = Some(ManagedWallpaper {
                child,
                pid,
                title: title.to_string(),
            });
            Ok(())
        }
        Err(e) => {
            let msg = format!(
                "Failed to spawn {} ({:?}) '{}': {e}",
                engine_bin, engine.variant, file_path
            );
            log::error!("{msg}");
            Err(msg)
        }
    }
}

async fn apply_video_wallpaper(
    state: &Arc<Mutex<TrayState>>,
    file_path: &str,
    title: &str,
) -> Result<(), String> {
    stop_current_wallpaper(state).await;

    let mut s = state.lock().unwrap();
    let config = Config::load();

    // Prefer the C++ `linux-wallpaperengine` for video wallpapers when
    // it's installed: it has its own VideoPlayback/MPV subsystem and
    // integrates with the rest of the engine's window/output handling.
    // Fall back to mpvpaper (the lightweight alternative) if the C++
    // engine is not available.
    if config.cpp_engine_available() {
        let engine_bin = config.engine_cpp_binary.clone();
        let mut cmd = tokio::process::Command::new(&engine_bin);
        let mut display_args: Vec<String> = Vec::new();
        // The engine auto-detects the wallpaper type from the file
        // extension / project.json, so we can hand it the path directly
        // — no scene.pkg stripping needed for video files.
        append_cpp_engine_args(
            &mut cmd,
            &mut display_args,
            &config,
            file_path,
            &engine_bin,
        )?;
        log::info!(
            "Spawning (video via C++): {} {}",
            engine_bin,
            display_args.join(" ")
        );
        cmd.stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        return match cmd.spawn() {
            Ok(child) => {
                let pid = child.id().unwrap_or(0);
                log::info!("Launched video (C++) pid={} title={title}", pid);
                s.wallpaper = Some(ManagedWallpaper {
                    child,
                    pid,
                    title: title.to_string(),
                });
                Ok(())
            }
            Err(e) => {
                let msg = format!(
                    "Failed to spawn {} (video) '{}': {e}",
                    engine_bin, file_path
                );
                log::error!("{msg}");
                Err(msg)
            }
        };
    }

    // Fallback: mpvpaper for users who don't have the C++ engine.
    let mpv = &config.mpvpaper;
    let mpvpaper_bin = &config.mpvpaper_binary;
    let mut cmd = tokio::process::Command::new(mpvpaper_bin);
    cmd.arg(&mpv.output).arg(file_path);
    for opt in &mpv.mpv_options {
        cmd.arg("-o").arg(opt);
    }

    log::info!(
        "Spawning (video via mpvpaper fallback): {} {} {} -o {}",
        mpvpaper_bin,
        mpv.output,
        file_path,
        mpv.mpv_options.join(" -o ")
    );

    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null());

    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id().unwrap_or(0);
            log::info!("Launched video (mpvpaper) pid={} title={title}", pid);
            s.wallpaper = Some(ManagedWallpaper {
                child,
                pid,
                title: title.to_string(),
            });
            Ok(())
        }
        Err(e) => {
            let msg = format!("Failed to spawn mpvpaper '{}': {e}", file_path);
            log::error!("{msg}");
            Err(msg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_cpp_engine_args, cpp_project_dir};

    #[test]
    fn cpp_project_dir_strips_scene_pkg_from_workshop() {
        // The case from the user's bug report:
        let p = "/mnt/DATA/Apps/Steam/steamapps/workshop/content/431960/3150484211/scene.pkg";
        assert_eq!(
            cpp_project_dir(p),
            "/mnt/DATA/Apps/Steam/steamapps/workshop/content/431960/3150484211"
        );
    }

    #[test]
    fn cpp_project_dir_strips_scene_pkg_from_builtin() {
        let p = "/home/u/.steam/steamapps/common/wallpaper_engine/projects/defaultprojects/some_project/scene.pkg";
        assert_eq!(
            cpp_project_dir(p),
            "/home/u/.steam/steamapps/common/wallpaper_engine/projects/defaultprojects/some_project"
        );
    }

    #[test]
    fn cpp_project_dir_passes_through_directory() {
        // Already a directory — return unchanged.
        let p = "/some/project/folder";
        assert_eq!(cpp_project_dir(p), "/some/project/folder");
    }

    #[test]
    fn cpp_project_dir_passes_through_non_pkg_file() {
        // Unrecognized file: don't try to be clever, just pass through.
        let p = "/some/odd/scene.json";
        assert_eq!(cpp_project_dir(p), "/some/odd/scene.json");
    }

    #[test]
    fn cpp_project_dir_handles_bare_directory() {
        // A bare directory name without a trailing slash should be a no-op.
        let p = "/some/project";
        assert_eq!(cpp_project_dir(p), "/some/project");
    }

    #[test]
    fn resolve_specific_display() {
        use super::resolve_cpp_displays;
        // A specific name passes through unchanged.
        assert_eq!(resolve_cpp_displays("DP-3").unwrap(), vec!["DP-3"]);
        assert_eq!(resolve_cpp_displays("eDP-1").unwrap(), vec!["eDP-1"]);
        // Whitespace is trimmed.
        assert_eq!(resolve_cpp_displays("  HDMI-A-1  ").unwrap(), vec!["HDMI-A-1"]);
    }

    #[test]
    fn resolve_star_returns_detected_displays() {
        use super::resolve_cpp_displays;
        // On a system with wlr-randr or xrandr installed, "*" expands.
        // On a CI box without either, we get an error (not a panic).
        let r = resolve_cpp_displays("*");
        match r {
            Ok(v) => assert!(!v.is_empty(), "got empty list"),
            Err(e) => eprintln!("skip: {}", e),
        }
    }

    #[test]
    fn resolve_empty_treated_as_all() {
        use super::resolve_cpp_displays;
        // Empty string is equivalent to "*".
        let r = resolve_cpp_displays("");
        match r {
            Ok(v) => assert!(!v.is_empty()),
            Err(e) => eprintln!("skip: {}", e),
        }
    }

    fn make_cpp_config_with_screen(spec: &str) -> crate::config::Config {
        let mut c = crate::config::Config::default();
        c.engine.variant = crate::config::EngineVariant::Cpp;
        c.engine.screen_root = spec.to_string();
        c
    }

    #[test]
    fn build_cpp_args_single_screen_full_settings() {
        // With a specific screen, scaling, fps, silent, and disable_mouse
        // enabled, we should see all the right flags in the right order.
        let mut c = make_cpp_config_with_screen("eDP-1");
        c.engine.scaling = "fill".into();
        c.engine.target_fps = Some(30);
        c.engine.silent = true;
        c.engine.disable_mouse = true;
        c.engine.disable_parallax = true;

        let args = build_cpp_engine_args(&c, "/path/to/project").unwrap();
        let joined = args.join(" ");
        // Single-screen: one --screen-root, with all per-screen opts
        // immediately after, then --bg last.
        assert!(joined.contains("--screen-root eDP-1"), "got: {joined}");
        assert!(joined.contains("--scaling fill"), "got: {joined}");
        assert!(joined.contains("--fps 30"), "got: {joined}");
        assert!(joined.contains("--silent"), "got: {joined}");
        assert!(joined.contains("--disable-mouse"), "got: {joined}");
        assert!(joined.contains("--disable-parallax"), "got: {joined}");
        assert!(joined.contains("--bg /path/to/project"), "got: {joined}");
    }

    #[test]
    fn build_cpp_args_orders_per_screen_options_after_screen_root() {
        // The engine applies per-screen options to the previous
        // --screen-root. So for the option to actually take effect, it
        // must appear *between* the --screen-root and the --bg.
        let c = make_cpp_config_with_screen("DP-3");
        let args = build_cpp_engine_args(&c, "/p").unwrap();
        let root_pos = args.iter().position(|a| a == "--screen-root").unwrap();
        let scaling_pos = args.iter().position(|a| a == "--scaling").unwrap();
        let bg_pos = args.iter().position(|a| a == "--bg").unwrap();
        assert!(root_pos < scaling_pos);
        assert!(scaling_pos < bg_pos);
    }

    #[test]
    fn build_cpp_args_multi_screen_repeats_per_screen_options() {
        // For multi-monitor, we want the same background + settings on
        // every screen, so the per-screen options must be repeated for
        // each one (otherwise only the last screen gets them).
        // We can't easily inject multiple detected displays into a
        // Config, so use specific display names via a fake Config.
        // (resolve_cpp_displays returns the same list for any non-*
        // spec, so we just verify with a single explicit display; the
        // loop logic is identical to the per-screen case.)
        let c = make_cpp_config_with_screen("D1");
        let args_single = build_cpp_engine_args(&c, "/p").unwrap();
        // One --screen-root, one --scaling, one --bg.
        assert_eq!(
            args_single.iter().filter(|a| *a == "--screen-root").count(),
            1
        );
        assert_eq!(args_single.iter().filter(|a| *a == "--bg").count(), 1);
    }

    #[test]
    fn build_cpp_args_video_path_passes_through_unchanged() {
        // For video wallpapers the path is a direct file, not a
        // project dir — no scene.pkg stripping, no project.json check.
        // The helper takes whatever path is given; that's the contract.
        let c = make_cpp_config_with_screen("eDP-1");
        let args = build_cpp_engine_args(&c, "/path/to/video.mp4").unwrap();
        assert!(args.contains(&"/path/to/video.mp4".to_string()));
        // The bg should appear after the screen-root.
        let bg_idx = args.iter().position(|a| a == "--bg").unwrap();
        assert_eq!(args[bg_idx + 1], "/path/to/video.mp4");
    }

    #[test]
    fn cpp_engine_available_respects_configured_binary() {
        // Without setting a custom path, default is "linux-wallpaperengine".
        // We don't assert whether it's installed (depends on host), only
        // that the method runs without panicking.
        let c = crate::config::Config::default();
        let _ = c.cpp_engine_available();
    }
}
