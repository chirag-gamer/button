//! Start with Windows (FR-47, D-085). tauri-plugin-autostart wraps the
//! auto-launch crate: on Windows it writes HKCU\Software\Microsoft\Windows\
//! CurrentVersion\Run with the app identifier as the value name and the
//! current exe as the target (no args).

use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

/// Turn the entry on or off, creating or refreshing it as needed.
pub fn apply(app: &AppHandle, on: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    if on {
        autolaunch
            .enable()
            .map_err(|e| format!("enabling Start with Windows failed: {e}"))
    } else {
        autolaunch
            .disable()
            .map_err(|e| format!("disabling Start with Windows failed: {e}"))
    }
}

/// At launch: `enable()` refreshes the registered exe path after an update;
/// when the setting is off but a stale entry exists, remove it (FR-47).
pub fn sync_at_launch(app: &AppHandle, on: bool) {
    let autolaunch = app.autolaunch();
    let enabled = autolaunch.is_enabled().unwrap_or(false);
    let result = if on {
        autolaunch.enable()
    } else if enabled {
        autolaunch.disable()
    } else {
        Ok(())
    };
    if let Err(e) = result {
        log::warn!("Start with Windows sync failed: {e}");
    }
}
