//! Floating recording bar: a small always-on-top window that keeps the live
//! capture's clock and stop button in sight while the user is in another app.
//!
//! The window is declared in `tauri.conf.json` (`floatbar`) and stays hidden
//! until a capture starts, so it costs nothing while idle. Visibility is driven
//! from [`menubar::on_phase`], not from the frontend: the same bar has to appear
//! for a recording started by the hotkey, by the tray or by meeting detection,
//! none of which have a window open to ask.
//!
//! It reuses the main window's commands through the same IPC, so the bar is a
//! second view of one recording, never a second recorder.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow};

use crate::db;
use crate::recording::Phase;
use crate::AppState;

/// Settings key: `true` (default) or `false`.
pub(crate) const SETTING_KEY: &str = "floating_bar";
pub(crate) const WINDOW_LABEL: &str = "floatbar";

/// Placed once per launch: a bar the user has dragged somewhere is their
/// layout decision, and re-centering it on every recording would undo it.
static PLACED: AtomicBool = AtomicBool::new(false);

pub(crate) fn sync(app: &AppHandle, phase: Phase) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else { return };
    let visible = matches!(phase, Phase::Recording | Phase::Paused) && enabled(app);
    if visible {
        place_once(&window);
        let _ = window.show();
    } else {
        let _ = window.hide();
    }
}

fn enabled(app: &AppHandle) -> bool {
    app.try_state::<AppState>()
        .map(|state| {
            let conn = state.db.lock();
            db::get_setting(&conn, SETTING_KEY)
                .ok()
                .flatten()
                .unwrap_or_else(|| "true".to_string())
                != "false"
        })
        .unwrap_or(true)
}

/// Bottom-centre of the screen the bar opens on: over the Dock's left/right
/// space rather than on top of it.
fn place_once(window: &WebviewWindow) {
    if PLACED.swap(true, Ordering::Relaxed) {
        return;
    }
    let Ok(Some(monitor)) = window.current_monitor() else { return };
    let screen = monitor.size();
    let origin = monitor.position();
    let width = window.outer_size().map(|s| s.width).unwrap_or(360);
    let x = origin.x + (screen.width as i32 - width as i32) / 2;
    let y = origin.y + screen.height as i32 - 132;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}
