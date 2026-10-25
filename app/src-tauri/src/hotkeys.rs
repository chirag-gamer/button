//! Global hotkey registration (spec 7.11, FR-9, FR-45).
//!
//! The recording-mode hotkey toggles Recording Mode through the one
//! dispatcher. If registration fails, `hotkey.registered` stays false and
//! the dropdown warns (FR-9); per-switch hotkeys arrive in Phase 4.

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::SharedState;
use crate::automation::{self, Action};

/// Register one hotkey with a Toggle dispatcher. Returns whether it worked.
pub fn register_toggle(app: &AppHandle, binding: &str) -> bool {
    let Ok(shortcut) = binding.parse::<Shortcut>() else {
        log::warn!("hotkey '{binding}' is not a valid accelerator");
        return false;
    };
    let result = app
        .global_shortcut()
        .on_shortcut(shortcut, |app, _s, event| {
            if event.state == ShortcutState::Pressed
                && let Err(message) = automation::run(app, Action::Toggle)
            {
                automation::notify(app, &message);
            }
        });
    match result {
        Ok(()) => true,
        Err(e) => {
            log::warn!("hotkey '{binding}' could not be registered: {e}");
            false
        }
    }
}

/// Sync the registration state into the shared state for the UI.
pub fn set_registered(app: &AppHandle, binding: &str, registered: bool) {
    let state = app.state::<SharedState>();
    let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
    b.hotkey.binding = binding.to_string();
    b.hotkey.registered = registered;
}
