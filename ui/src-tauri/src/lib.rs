//! The search overlay's Rust half.
//!
//! It is deliberately thin. Everything it exposes to the frontend is a method the daemon
//! already serves, forwarded through [`bridge`] — the same discipline the MCP server follows
//! and for the same reason (D36): a GUI that computed anything would be a second implementation
//! of it, and D4's second condition says the frontend does no data processing at all.
//!
//! What genuinely belongs here is the part a browser cannot do: the socket, the window, the
//! global hotkey, and opening a file in whatever the desktop thinks should open it.

pub mod bridge;
pub mod clipboard;
pub mod preview;
pub mod startup;

use bridge::{Bridge, BridgeError};
use cortex_core::paths::Paths;
use serde_json::{json, Value};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

/// Everything the commands share.
pub struct AppState {
    pub bridge: Bridge,
    pub startup: startup::Startup,
    pub thumbnailer: preview::Thumbnailer,
}

// ---------------------------------------------------------------------------
// Commands. Each is one line of forwarding plus the argument the frontend finds natural.
// ---------------------------------------------------------------------------

#[tauri::command]
async fn search(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(
            cortex_rpc::protocol::method::SEARCH,
            json!({ "query": query, "limit": limit }),
        )
        .await
}

#[tauri::command]
async fn status(state: tauri::State<'_, AppState>) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(cortex_rpc::protocol::method::STATUS, json!({}))
        .await
}

#[tauri::command]
async fn extract_status(state: tauri::State<'_, AppState>) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(cortex_rpc::protocol::method::EXTRACT_STATUS, json!({}))
        .await
}

/// What the brain knows about one node: its edges, and the notes on it.
#[tauri::command]
async fn neighbors(
    state: tauri::State<'_, AppState>,
    target: Value,
    depth: Option<u32>,
    edge_types: Option<Vec<String>>,
) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(
            cortex_rpc::protocol::method::BRAIN_NEIGHBORS,
            json!({
                "target": target,
                "depth": depth.unwrap_or(1),
                "edge_types": edge_types.unwrap_or_default(),
            }),
        )
        .await
}

/// The episodes attached to one path — the "what happened to this" half of the sidebar.
#[tauri::command]
async fn timeline(
    state: tauri::State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(
            cortex_rpc::protocol::method::BRAIN_TIMELINE,
            json!({ "limit": limit.unwrap_or(20) }),
        )
        .await
}

#[tauri::command]
async fn node_get(state: tauri::State<'_, AppState>, path: String) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(
            cortex_rpc::protocol::method::NODE_GET,
            json!({ "path": path }),
        )
        .await
}

/// A directory, ordered by the daemon.
///
/// The ordering argument goes through rather than being applied here: a file manager that
/// sorted its own listing would be the frontend data processing D4 forbids, and two clients
/// would eventually disagree about what "by name" means.
#[tauri::command]
async fn list(
    state: tauri::State<'_, AppState>,
    path: String,
    sort: Option<String>,
    reverse: Option<bool>,
    depth: Option<u32>,
    limit: Option<usize>,
) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(
            cortex_rpc::protocol::method::FS_LIST,
            json!({
                "path": path,
                "sort": sort.unwrap_or_else(|| "name".into()),
                "reverse": reverse.unwrap_or(false),
                "depth": depth.unwrap_or(1),
                "limit": limit,
            }),
        )
        .await
}

/// Start a copy or a move. Answers with a handle; the work happens in the daemon and reports
/// through `transfer:progress` and `transfer:done`.
#[tauri::command]
async fn transfer(
    state: tauri::State<'_, AppState>,
    sources: Vec<String>,
    destination: String,
    mode: String,
    conflict: Option<String>,
    dry_run: Option<bool>,
) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(
            cortex_rpc::protocol::method::FS_TRANSFER,
            json!({
                "sources": sources,
                "destination": destination,
                "mode": mode,
                "conflict": conflict.unwrap_or_else(|| "fail".into()),
                "dry_run": dry_run.unwrap_or(false),
            }),
        )
        .await
}

#[tauri::command]
async fn cancel_operation(
    state: tauri::State<'_, AppState>,
    operation: u64,
) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(
            cortex_rpc::protocol::method::OPERATION_CANCEL,
            json!({ "operation": operation }),
        )
        .await
}

#[tauri::command]
async fn mkdir(state: tauri::State<'_, AppState>, path: String) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(cortex_rpc::protocol::method::FS_MKDIR, json!({ "path": path }))
        .await
}

/// Named `rename_entry` rather than `rename` because `rename` is taken on the JavaScript side
/// by nothing at all — but a command called `rename` in a file manager reads like a window
/// operation, and this one is about a file.
#[tauri::command]
async fn rename_entry(
    state: tauri::State<'_, AppState>,
    path: String,
    name: String,
) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(
            cortex_rpc::protocol::method::FS_RENAME,
            json!({ "path": path, "name": name }),
        )
        .await
}

/// Where a person would like to start. The daemon decides what is on it (D52).
#[tauri::command]
async fn places(state: tauri::State<'_, AppState>) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(cortex_rpc::protocol::method::FS_PLACES, json!({}))
        .await
}

#[tauri::command]
async fn trash(state: tauri::State<'_, AppState>, path: String) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(cortex_rpc::protocol::method::FS_TRASH, json!({ "path": path }))
        .await
}

#[tauri::command]
async fn undo_last(state: tauri::State<'_, AppState>, last: Option<usize>) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(
            cortex_rpc::protocol::method::UNDO,
            json!({ "last": last.unwrap_or(1) }),
        )
        .await
}

/// A capped slice of a file, for the preview pane.
///
/// It goes through the daemon's `fs.read` rather than reading the file here, which looks like a
/// detour and is not: that path applies the deny-list (R1) and writes an audit entry. A GUI
/// that opened files directly would be a way to read what the rest of the system refuses to.
#[tauri::command]
async fn read_text(
    state: tauri::State<'_, AppState>,
    path: String,
    bytes: Option<u64>,
) -> Result<Value, BridgeError> {
    let cap = bytes.unwrap_or(64 * 1024);
    state
        .bridge
        .call(
            cortex_rpc::protocol::method::FS_READ,
            json!({ "path": path, "byte_range": [0, cap] }),
        )
        .await
}

/// A thumbnail for the preview pane.
///
/// Rendered by a separate process (D14), so a malformed image kills a helper rather than this
/// window. The smaller cached thumbnail, when there is one, arrives first as a `preview:partial`
/// event — the caller shows it immediately and replaces it when this returns.
///
/// Blocking rather than async on purpose: it runs on Tauri's blocking pool, and the alternative
/// would be an async wrapper around a synchronous pipe, which is a thread either way.
#[tauri::command(async)]
fn preview(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    path: String,
    pixels: Option<u32>,
) -> Result<preview::Preview, preview::PreviewError> {
    let started = std::time::Instant::now();
    let emitter = window.clone();
    let answer = state.thumbnailer.preview(
        std::path::Path::new(&path),
        pixels.unwrap_or(512),
        move |params| {
            let _ = emitter.emit("preview:partial", params);
        },
    );

    let elapsed = started.elapsed().as_millis();
    if elapsed >= 50 {
        tracing::info!(path, ms = elapsed as u64, "slow preview");
    }
    answer
}

#[tauri::command]
async fn explain(
    state: tauri::State<'_, AppState>,
    query: String,
    path: String,
) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(
            cortex_rpc::protocol::method::SEARCH_EXPLAIN,
            json!({ "query": query, "path": path }),
        )
        .await
}

#[tauri::command]
async fn zones(state: tauri::State<'_, AppState>) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(cortex_rpc::protocol::method::ZONES_LIST, json!({}))
        .await
}

/// Tell the daemon a human is present. Expensive background stages run on a duty cycle keyed
/// to this (D22), and a person typing in the overlay is the clearest evidence there is.
#[tauri::command]
async fn activity_ping(state: tauri::State<'_, AppState>) -> Result<Value, BridgeError> {
    state
        .bridge
        .call(cortex_rpc::protocol::method::ACTIVITY_PING, json!({}))
        .await
}

/// Hand a file to the desktop and get out of the way.
///
/// This is a launcher: opening the result is the whole point of it, and the window closes
/// afterwards because leaving it up would make it a window manager's problem.
#[tauri::command]
fn open_path(window: tauri::WebviewWindow, path: String) -> Result<(), String> {
    let target = std::path::Path::new(&path);
    if !target.exists() {
        return Err(format!("{path} is not there any more"));
    }
    std::process::Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map_err(|e| format!("could not open {path}: {e}"))?;
    conceal(&window);
    Ok(())
}

#[tauri::command]
fn dismiss(window: tauri::WebviewWindow) {
    conceal(&window);
}

// ---------------------------------------------------------------------------
// The system clipboard. Not forwarded to the daemon, and that is the point: the daemon is
// headless and a clipboard belongs to a session (D49). These three are the only commands here
// that do something rather than relay it.
// ---------------------------------------------------------------------------

/// Put files on the system clipboard so that another application can paste them.
///
/// Blocking rather than async: it hops to the GTK thread and waits, which is a thread either
/// way, and `#[tauri::command(async)]` puts it on the blocking pool where waiting is allowed.
#[cfg(target_os = "linux")]
#[tauri::command(async)]
fn clipboard_write(
    app: tauri::AppHandle,
    paths: Vec<std::path::PathBuf>,
    mode: clipboard::Mode,
) -> Result<(), clipboard::ClipboardError> {
    let selection = clipboard::Selection { mode, paths };
    let taken = clipboard::on_main_thread(&app, move || clipboard::session::set(&selection))?;
    // A refusal is reported rather than swallowed. On Wayland the selection can only be taken
    // off the back of a recent input event, so a copy from an unfocused window does not happen
    // — and a person who is not told discovers it later by pasting the wrong thing.
    if taken {
        Ok(())
    } else {
        Err(clipboard::ClipboardError::Refused)
    }
}

/// What another application put on the clipboard, if it was files.
///
/// `None` for anything else — text, an image, a `trash://` URI — because "there is nothing here
/// to paste" and "the clipboard is empty" are the same answer as far as a file manager is
/// concerned.
#[cfg(target_os = "linux")]
#[tauri::command(async)]
fn clipboard_read(
    app: tauri::AppHandle,
) -> Result<Option<clipboard::Selection>, clipboard::ClipboardError> {
    clipboard::on_main_thread(&app, clipboard::session::get)
}

/// Give up the selection, after a cut has been carried out.
///
/// The paths on it do not exist any more, so leaving them there means the next paste anywhere
/// on the desktop fails with a list of files that have moved.
#[cfg(target_os = "linux")]
#[tauri::command(async)]
fn clipboard_clear(app: tauri::AppHandle) -> Result<(), clipboard::ClipboardError> {
    clipboard::on_main_thread(&app, clipboard::session::clear)
}

/// The frontend calls this the moment it has painted something a person can act on.
///
/// M4 §1 asks for cold start to interactive under 300 ms "measured with a real timer in the
/// real app", and this is that timer's other end. It is not a benchmark harness: it is the
/// number the app reports about itself, every time it starts.
#[tauri::command]
/// `page_ms` is milliseconds since the webview began navigating, as the page measured it, and
/// `dom_ready_ms` is when the document finished parsing. The gap between `page_ms` and the
/// process-side total is the webview's own startup — the part no amount of bundling work can
/// shorten, and therefore the first thing worth knowing when the budget is missed.
fn mark_interactive(
    state: tauri::State<'_, AppState>,
    page_ms: Option<u64>,
    dom_ready_ms: Option<u64>,
) -> u64 {
    let elapsed = state.startup.mark_interactive();
    let report = state.startup.report();
    // Both halves, every run. "601 ms" says the budget was missed; "connected at 3 ms,
    // interactive at 601" says by whom, and that is the difference between a number and a
    // diagnosis.
    tracing::info!(
        interactive_ms = elapsed,
        setup_ms = report.setup_ms.unwrap_or(0),
        connected_ms = report.connected_ms.unwrap_or(0),
        page_ms = page_ms.unwrap_or(0),
        dom_ready_ms = dom_ready_ms.unwrap_or(0),
        budget_ms = report.budget_ms,
        // Stated rather than quietly reported as a miss: a run that spent half a second waiting
        // for another service to open its socket measured that service, not this one. Leaving
        // the verdict in would put a `within_budget=false` in the journal at every login for a
        // reason that has nothing to do with the code — which is how a number gets believed.
        within_budget = if report.comparable() {
            Some(report.within_budget)
        } else {
            None
        },
        waited_for_daemon_ms = report.waited_for_daemon_ms,
        comparable = report.comparable(),
        "cold start"
    );
    elapsed
}

#[tauri::command]
fn startup_report(state: tauri::State<'_, AppState>) -> startup::Report {
    state.startup.report()
}

/// The summoning hotkey.
///
/// What a summon does, wherever it came from: a compositor binding, a global hotkey, or a
/// second invocation of the binary.
///
/// Toggling rather than showing, because a key pressed by accident should undo itself, and
/// because the same binding then serves both directions.
pub fn summon<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>, toggle: bool) {
    if toggle && window.is_visible().unwrap_or(false) {
        conceal(window);
        return;
    }

    // The number the resident model exists for. M4 §1's 300 ms is about a cold start and stays
    // red (D41); this is what a person actually waits for once the process is resident.
    let started = std::time::Instant::now();
    let _ = window.show();
    // Showing is not focusing on Wayland, and an overlay that appears without the caret in it
    // makes the first keystroke disappear.
    let _ = window.set_focus();

    // A launcher summoned again starts empty. Keeping the last query would mean every summon
    // begins by deleting somebody else's search.
    let _ = window.emit("window:shown", ());
    let _ = window.emit("window:visible", true);
    tracing::info!(us = started.elapsed().as_micros() as u64, "summoned");
}

/// Put the window away, and say so.
///
/// The event is the point. A hidden GTK window does not stop the page inside it: WebKit keeps
/// running the document, so a CSS animation keeps animating and a `setInterval` keeps firing —
/// for a resident overlay, forever, at whatever they cost.
///
/// Measured before this existed: with the DMABUF renderer disabled, which drops WebKit to
/// software compositing, the breath layer alone held **99% of a core with the window hidden**.
/// The renderer is no longer disabled, so the same layer is now nearly free — but "nearly free
/// while nobody is looking" is still worse than "not running", and the next machine that has to
/// disable it should not pay a core for the privilege.
pub fn conceal<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    // Told first, hidden second: the page gets its last frame to stop things in, rather than
    // being asked to stop after it can no longer be scheduled.
    let _ = window.emit("window:visible", false);
    let _ = window.hide();
}

/// A launcher that has to be found in a window list is not a launcher. Under a tiling
/// compositor the window is opened and closed constantly, so the binding toggles: pressed while
/// the overlay is up, it puts it away again, which is what a person who pressed it by accident
/// expects.
///
/// `Ctrl+Alt+Space` because it collides with nothing on a stock desktop.
///
/// **On Wayland this is not registered at all**, and that is not a limitation being worked
/// around — it is how Wayland works. A client cannot grab a key globally; the compositor
/// dispatches input and only it can decide that a combination means something. The library
/// behind this plugin falls back to an X11 grab, which under a Wayland session lands in
/// XWayland and never fires, while reporting success. Registering it anyway would mean logging
/// "summon with the hotkey" for a hotkey that cannot work, which is exactly what happened here
/// once and cost an evening.
const HOTKEY: &str = "ctrl+alt+space";

/// How long to keep trying to reach the daemon before giving up.
///
/// Long enough to cover a daemon that is starting alongside this process, short enough that a
/// machine with no daemon at all does not appear to hang. It costs nothing when the socket is
/// already there, which is every case except the first few hundred milliseconds after a login.
const CONNECT_PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

/// The interval between attempts. Small, because the usual wait is one or two of them.
const CONNECT_RETRY: std::time::Duration = std::time::Duration::from_millis(100);

/// Connect, allowing for a daemon that is still on its way up.
///
/// The first failure is not logged. It is expected at every login and a warning that appears
/// every time and means nothing is a warning nobody reads. A failure that persists past the
/// first retry is logged once, so a machine where the daemon is genuinely absent says so.
async fn connect_with_patience(
    socket: &std::path::Path,
    emitter: tauri::AppHandle,
    startup: &startup::Startup,
) -> Result<Bridge, BridgeError> {
    let began = std::time::Instant::now();
    let deadline = began + CONNECT_PATIENCE;
    let mut announced = false;

    loop {
        let emitter = emitter.clone();
        let attempt = Bridge::connect(socket, move |method, params| {
            // A better answer arriving after the first one (D7) is an event, not a return
            // value — there is nothing left waiting to return it to.
            let _ = emitter.emit(&method.replace('.', ":"), params);
        })
        .await;

        match attempt {
            Ok(bridge) => {
                if announced {
                    // The cold-start number for this run includes the wait, so it is not a
                    // measurement of this application starting. Recording that is what keeps
                    // it out of the budget verdict.
                    startup.mark_waited(began.elapsed().as_millis() as u64);
                }
                return Ok(bridge);
            }
            Err(e) if std::time::Instant::now() >= deadline => return Err(e),
            Err(_) => {
                if !announced {
                    announced = true;
                    tracing::info!("waiting for the daemon's socket");
                }
                tokio::time::sleep(CONNECT_RETRY).await;
            }
        }
    }
}

/// Is this a Wayland session, where the compositor owns the key bindings?
fn on_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").map(|t| t == "wayland").unwrap_or(false)
}

fn global_shortcut<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_global_shortcut::ShortcutState;

    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            // Press, not release: acting on both would toggle twice per keypress.
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let Some(window) = app.get_webview_window("overlay") else {
                return;
            };
            summon(&window, true);
        })
        .build()
}

// ---------------------------------------------------------------------------

/// Should the window appear at startup?
///
/// Resident by default: the overlay is started once, at login, and summoned with the hotkey.
/// `--show` is for running it by hand to look at it, which is the only case where a window
/// appearing unasked is what somebody wanted.
///
/// Parsed by hand rather than with `clap`: it is one flag, and a dependency for one flag in the
/// binary whose startup time is a milestone gate would be an odd trade.
fn show_at_startup() -> bool {
    std::env::args().any(|arg| arg == "--show" || arg == "--toggle")
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hippocampus_ui_lib=info".into()),
        )
        .init();

    let startup = startup::Startup::begin();

    tauri::Builder::default()
        // Registered first, so it runs before anything else has a chance to fail: a second
        // invocation must reach the resident process even if that process is unhappy.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let Some(window) = app.get_webview_window("overlay") else {
                return;
            };
            // Only an explicit flag moves the window. A bare second launch does nothing, and
            // that asymmetry is deliberate: somebody who wanted the window types one more word,
            // whereas a stray invocation — a shell completion, a script, a restart race —
            // otherwise puts a search box in front of whoever is sitting there. It cost an
            // interruption to learn that once.
            if argv.iter().any(|a| a == "--hide") {
                conceal(&window);
            } else if argv.iter().any(|a| a == "--show" || a == "--toggle") {
                summon(&window, argv.iter().any(|a| a == "--toggle"));
            } else {
                tracing::info!("already running; `hippocampus --show` summons it");
            }
        }))
        .plugin(global_shortcut())
        .setup(move |app| {
            startup.mark_setup();
            let handle = app.handle().clone();
            let paths = Paths::from_env()?;
            let socket = paths.socket();

            // Connecting is the one thing that can fail in a way the person needs to know
            // about. It is also the one thing that fails for a moment at every login and then
            // stops: both services start together, the daemon is "started" the instant it is
            // spawned, and its socket appears a moment later — so the first attempt loses a
            // race that the second one wins. Waiting a little is the whole fix, and without it
            // the overlay aborts at login and is restarted by systemd, which works and looks
            // like a crash in the journal because it is one.
            let emitter = handle.clone();
            let waiting = startup.clone();
            let bridge = tauri::async_runtime::block_on(async move {
                connect_with_patience(&socket, emitter, &waiting).await
            });

            match bridge {
                Ok(bridge) => {
                    startup.mark_connected();
                    app.manage(AppState {
                        bridge,
                        startup: startup.clone(),
                        thumbnailer: preview::Thumbnailer::beside_this_binary(),
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "no daemon");
                    // The frontend renders this rather than a blank list.
                    let _ = handle.emit("daemon:unavailable", e.to_string());
                    return Err(Box::new(e) as Box<dyn std::error::Error>);
                }
            }
            if on_wayland() {
                // Say what to do, with the line to paste. A message that only says "not
                // supported" leaves somebody to find out for themselves that their compositor
                // is where the binding belongs.
                tracing::info!(
                    "Wayland: the compositor owns key bindings, so no global hotkey is \
                     registered. Bind one to `hippocampus --show`. For niri, in config.kdl: \
                     Ctrl+Alt+Space hotkey-overlay-title=\"Hippocampus\" {{ spawn \
                     \"hippocampus\" \"--show\"; }}"
                );
            } else {
                match app.global_shortcut().register(HOTKEY) {
                    Ok(()) => tracing::info!(hotkey = HOTKEY, "summon with the hotkey"),
                    Err(e) => tracing::warn!(
                        hotkey = HOTKEY,
                        error = %e,
                        "the hotkey is taken; bind one to `hippocampus --show` instead"
                    ),
                }
            }

            if show_at_startup() {
                if let Some(window) = handle.get_webview_window("overlay") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }

            if let Some(window) = handle.get_webview_window("overlay") {
                let hidden = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        // Hide, do not close. Tauri exits with its last window, and a launcher
                        // that quits when the compositor asks it to close has stopped being
                        // resident without telling anybody.
                        api.prevent_close();
                        conceal(&hidden);
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            search,
            status,
            extract_status,
            neighbors,
            timeline,
            node_get,
            list,
            places,
            transfer,
            cancel_operation,
            mkdir,
            rename_entry,
            trash,
            undo_last,
            read_text,
            preview,
            explain,
            zones,
            activity_ping,
            open_path,
            dismiss,
            clipboard_write,
            clipboard_read,
            clipboard_clear,
            mark_interactive,
            startup_report,
        ])
        .run(tauri::generate_context!())
        .expect("the overlay failed to start");
}
