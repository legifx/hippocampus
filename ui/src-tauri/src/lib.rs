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
fn open_path(window: tauri::Window, path: String) -> Result<(), String> {
    let target = std::path::Path::new(&path);
    if !target.exists() {
        return Err(format!("{path} is not there any more"));
    }
    std::process::Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map_err(|e| format!("could not open {path}: {e}"))?;
    let _ = window.hide();
    Ok(())
}

/// Copy without a clipboard dependency: the frontend owns the clipboard, this is for the path
/// of the *selected* row, which the frontend already has. Kept as a command only so that the
/// window can be dismissed in the same gesture.
#[tauri::command]
fn dismiss(window: tauri::Window) {
    let _ = window.hide();
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
        within_budget = report.within_budget,
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
        let _ = window.hide();
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
    tracing::info!(us = started.elapsed().as_micros() as u64, "summoned");
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
                let _ = window.hide();
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
            // about, and it must not stop the window from appearing: a window that says "the
            // daemon is not running" is useful, and no window at all is not.
            let emitter = handle.clone();
            let bridge = tauri::async_runtime::block_on(async move {
                Bridge::connect(&socket, move |method, params| {
                    // A better answer arriving after the first one (D7) is an event, not a
                    // return value — there is nothing left waiting to return it to.
                    let _ = emitter.emit(&method.replace('.', ":"), params);
                })
                .await
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
                        let _ = hidden.hide();
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
            read_text,
            preview,
            explain,
            zones,
            activity_ping,
            open_path,
            dismiss,
            mark_interactive,
            startup_report,
        ])
        .run(tauri::generate_context!())
        .expect("the overlay failed to start");
}
