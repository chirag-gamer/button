//! Global hotkey registration (spec 7.11, FR-9, FR-45).
//!
//! The recording-mode hotkey toggles Recording Mode; each switch hotkey
//! toggles its switch through the one dispatcher. Registration is
//! transactional: `apply_all` unregisters everything, registers the new
//! set, and on any failure restores the previous set.

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::SharedState;
use crate::automation::{self, Action, SwitchOnOff};
use crate::settings::Hotkeys;

/// Register the whole hotkey set, replacing whatever is registered now.
/// Transactional: on any failure the previous set is re-registered and the
/// error names the binding that failed (FR-45 "rejected with a message").
pub fn apply_all(app: &AppHandle, previous: &Hotkeys, next: &Hotkeys) -> Result<(), String> {
    let parse = |binding: &str| -> Result<tauri_plugin_global_shortcut::Shortcut, String> {
        binding
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .map_err(|_| format!("'{binding}' is not a valid hotkey"))
    };

    let old = hotkey_list(previous);
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| format!("unregistering the old hotkeys failed: {e}"))?;

    let mut registered: Vec<(String, tauri_plugin_global_shortcut::Shortcut)> = Vec::new();
    for (binding, action) in hotkey_list(next) {
        let shortcut = match parse(&binding) {
            Ok(shortcut) => shortcut,
            Err(message) => return rollback(app, &old, &message),
        };
        let result = app
            .global_shortcut()
            .on_shortcut(shortcut, move |app, _s, event| {
                if event.state == ShortcutState::Pressed
                    && let Err(message) = automation::run(app, action.clone())
                {
                    automation::notify(app, &message.to_string());
                }
            });
        if let Err(e) = result {
            let message = format!("'{binding}' is not available: {e}");
            return rollback(app, &old, &message);
        }
        registered.push((binding, shortcut));
    }

    // Record the recording-mode registration state for the UI (FR-9).
    let rm_binding = next.recording_mode.clone();
    let registered_ok = parse(&rm_binding)
        .map(|shortcut| registered.iter().any(|(_, s)| *s == shortcut))
        .unwrap_or(false);
    set_registered(app, &rm_binding, registered_ok);
    Ok(())
}

/// The (binding, dispatcher action) pairs of a hotkey set, skipping empty
/// bindings (cleared) and unknown switch ids (already rejected by
/// settings::normalize_saving; skipped here so launch never panics).
fn hotkey_list(hotkeys: &Hotkeys) -> Vec<(String, Action)> {
    let mut list = Vec::new();
    if !hotkeys.recording_mode.is_empty() {
        list.push((hotkeys.recording_mode.clone(), Action::Toggle));
    }
    for (id, binding) in &hotkeys.switches {
        if binding.is_empty() {
            continue;
        }
        let Ok(id) = automation::parse_switch_id(id) else {
            continue;
        };
        list.push((
            binding.clone(),
            Action::Switch {
                id,
                on: SwitchOnOff::Toggle,
            },
        ));
    }
    list
}

/// Put the previous set back and report why the new one failed.
fn rollback(app: &AppHandle, previous: &[(String, Action)], message: &str) -> Result<(), String> {
    let _ = app.global_shortcut().unregister_all();
    for (binding, action) in previous {
        let action = action.clone();
        if let Ok(shortcut) = binding.parse::<tauri_plugin_global_shortcut::Shortcut>()
            && let Err(e) = app
                .global_shortcut()
                .on_shortcut(shortcut, move |app, _s, event| {
                    if event.state == ShortcutState::Pressed
                        && let Err(message) = automation::run(app, action.clone())
                    {
                        automation::notify(app, &message.to_string());
                    }
                })
        {
            log::warn!("hotkey '{binding}' could not be restored after a failed save: {e}");
        }
    }
    Err(message.to_string())
}

/// Sync the recording-mode registration state into the shared state for
/// the UI (FR-9).
pub fn set_registered(app: &AppHandle, binding: &str, registered: bool) {
    let state = app.state::<SharedState>();
    let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
    b.hotkey.binding = binding.to_string();
    b.hotkey.registered = registered;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::SwitchOnOff;
    use crate::switches::SwitchId;

    #[test]
    fn hotkey_list_maps_the_recording_toggle_and_switch_toggles() {
        let hotkeys = Hotkeys {
            recording_mode: "Alt+R".into(),
            switches: [("keep-awake".to_string(), "Ctrl+Alt+F10".to_string())]
                .into_iter()
                .collect(),
        };
        let list = hotkey_list(&hotkeys);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, "Alt+R");
        assert_eq!(list[0].1, Action::Toggle);
        assert_eq!(
            list[1].1,
            Action::Switch {
                id: SwitchId::KeepAwake,
                on: SwitchOnOff::Toggle
            }
        );
    }

    #[test]
    fn empty_bindings_and_unknown_switch_ids_are_skipped() {
        let hotkeys = Hotkeys {
            recording_mode: String::new(),
            switches: [
                ("keep-awake".to_string(), String::new()),
                ("not-a-switch".to_string(), "Ctrl+K".to_string()),
            ]
            .into_iter()
            .collect(),
        };
        assert!(hotkey_list(&hotkeys).is_empty());
    }
}
