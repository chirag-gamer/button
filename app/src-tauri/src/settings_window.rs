//! The Settings window (FR-39): one instance, opened from the tray's
//! Settings… item and the dropdown's gear. Content is protected from
//! captures (D-032); closing destroys the window and the app keeps
//! running (tray app, see the ExitRequested guard in lib.rs).

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "settings";

/// The section ids the window can open on (settings.html's
/// `section-<id>` elements); anything else opens the default section.
pub const SECTIONS: [&str; 6] = [
    "general",
    "recording",
    "switches",
    "hotkeys",
    "backdrop",
    "about",
];

/// The event that moves an open window to a section (payload: the id).
pub const SECTION_EVENT: &str = "settings-section";

/// Only whitelisted ids pass: the id ends up in a script string.
fn known_section(section: Option<&str>) -> Option<&'static str> {
    let section = section?;
    SECTIONS.iter().copied().find(|s| *s == section)
}

/// Open the settings window, or bring the existing one back (unminimize,
/// show, focus), optionally on one section (the dropdown's Recording Mode
/// chevron opens "recording"). The builder runs on its own thread:
/// WebviewWindowBuilder deadlocks inside sync commands and main-thread
/// handlers (D-071).
pub fn open(app: &AppHandle, section: Option<&str>) {
    let section = known_section(section);
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        if let Some(section) = section {
            let _ = app.emit_to(LABEL, SECTION_EVENT, section);
        }
        return;
    }
    // A new window reads its first section from this global before its
    // module runs (no event can reach a page that is not loaded yet).
    let script = format!("window.__buttonSection = {:?};", section.unwrap_or(""));
    let app2 = app.clone();
    std::thread::spawn(move || {
        if let Err(e) =
            WebviewWindowBuilder::new(&app2, LABEL, WebviewUrl::App("settings.html".into()))
                .title("button settings")
                .inner_size(760.0, 560.0)
                .resizable(true)
                .content_protected(true)
                .initialization_script(&script)
                .build()
        {
            log::error!("settings window could not be created: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_sections_pass() {
        assert_eq!(known_section(Some("recording")), Some("recording"));
        assert_eq!(known_section(Some("backdrop")), Some("backdrop"));
        assert_eq!(known_section(Some("\";alert(1)//")), None);
        assert_eq!(known_section(Some("")), None);
        assert_eq!(known_section(None), None);
    }
}
