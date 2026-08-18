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
    let emitter = window.clone();
    state.thumbnailer.preview(
        std::path::Path::new(&path),
        pixels.unwrap_or(512),
        move |params| {
            let _ = emitter.emit("preview:partial", params);
        },
    )
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
fn mark_interactive(state: tauri::State<'_, AppState>) -> u64 {
    let elapsed = state.startup.mark_interactive();
    tracing::info!(ms = elapsed, "interactive");
    elapsed
}

#[tauri::command]
fn startup_report(state: tauri::State<'_, AppState>) -> startup::Report {
    state.startup.report()
}

/// The summoning hotkey.
///
/// A launcher that has to be found in a window list is not a launcher. Under a tiling
/// compositor the window is opened and closed constantly, so the binding toggles: pressed while
/// the overlay is up, it puts it away again, which is what a person who pressed it by accident
/// expects.
///
/// `Ctrl+Alt+Space` because it collides with nothing on a stock desktop. The compositor may own
/// the binding first — niri users bind their own — and if it does, registration fails and the
/// overlay says so rather than starting a hotkey nobody can press.
const HOTKEY: &str = "ctrl+alt+space";

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
            match window.is_visible() {
                Ok(true) => {
                    let _ = window.hide();
                }
                _ => {
                    let _ = window.show();
                    // Showing is not focusing on Wayland, and an overlay that appears without
                    // the caret in it makes the first keystroke disappear.
                    let _ = window.set_focus();
                }
            }
        })
        .build()
}

// ---------------------------------------------------------------------------

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hippocampus_ui_lib=info".into()),
        )
        .init();

    let startup = startup::Startup::begin();

    tauri::Builder::default()
        .plugin(global_shortcut())
        .setup(move |app| {
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
            // The compositor may already own this combination. That is a normal outcome, not a
            // failure: the window still works, it just has to be started rather than summoned.
            match app.global_shortcut().register(HOTKEY) {
                Ok(()) => tracing::info!(hotkey = HOTKEY, "summon with the hotkey"),
                Err(e) => tracing::warn!(
                    hotkey = HOTKEY,
                    error = %e,
                    "the hotkey is taken; start the overlay from the compositor instead"
                ),
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
