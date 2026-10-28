//! The Settings window (FR-39): one instance, opened from the tray's
//! Settings… item and the dropdown's gear. Content is protected from
//! captures (D-032); closing destroys the window and the app keeps
//! running (tray app, see the ExitRequested guard in lib.rs).

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "settings";

/// Open the settings window, or bring the existing one back (unminimize,
/// show, focus). The builder runs on its own thread: WebviewWindowBuilder
/// deadlocks inside sync commands and main-thread handlers (D-071).
pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let app2 = app.clone();
    std::thread::spawn(move || {
        if let Err(e) =
            WebviewWindowBuilder::new(&app2, LABEL, WebviewUrl::App("settings.html".into()))
                .title("button settings")
                .inner_size(760.0, 560.0)
                .resizable(true)
                .content_protected(true)
                .build()
        {
            log::error!("settings window could not be created: {e}");
        }
    });
}
