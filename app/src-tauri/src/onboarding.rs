//! First-run onboarding (FR-52, spec 7.14, D-047): four steps — what
//! button does, the Recording Mode hotkey with Change, where the tray
//! icon is, and Start with Windows. Done or closing the window sets
//! `general.onboardingDone`; Settings › About reopens it without
//! changing anything.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "onboarding";

/// Open the onboarding window, or focus the existing one. The builder
/// runs on its own thread (D-071 deadlock rule).
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
            WebviewWindowBuilder::new(&app2, LABEL, WebviewUrl::App("onboarding.html".into()))
                .title("Welcome to button")
                .inner_size(520.0, 420.0)
                .resizable(true)
                .content_protected(true)
                .build()
        {
            log::error!("onboarding window could not be created: {e}");
        }
    });
}

/// At launch (spec 7.14): after recovery and hotkey registration, show
/// onboarding once on a fresh profile. Skipped when the settings file is
/// read-only (a newer button wrote it) so the flag write cannot clobber
/// it (§7.12).
pub fn maybe_open_at_launch(app: &AppHandle) {
    let state = app.state::<crate::SharedState>();
    let b = state.lock().unwrap_or_else(|p| p.into_inner());
    let show = !b.settings.general.onboarding_done && !b.settings_read_only;
    drop(b);
    if show {
        open(app);
    }
}

/// The onboarding window was destroyed (Done button or the close box):
/// set `general.onboardingDone` and save (FR-52). Reopening from
/// Settings › About must not change the flag, and this runs on every
/// destroy, so both guards are needed.
pub fn mark_done_on_destroy(app: &AppHandle) {
    let state = app.state::<crate::SharedState>();
    let b = state.lock().unwrap_or_else(|p| p.into_inner());
    if b.settings.general.onboarding_done || b.settings_read_only {
        return;
    }
    let mut candidate = b.settings.clone();
    drop(b);
    candidate.general.onboarding_done = true;
    if let Err(e) =
        crate::commands::commit_shared(app, app.state::<crate::SharedState>().inner(), candidate)
    {
        log::warn!("saving onboardingDone failed: {e:?}");
    }
}
