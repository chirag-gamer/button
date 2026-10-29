//! Global hotkey registration (spec 7.11, FR-9, FR-45).
//!
//! The recording-mode hotkey toggles Recording Mode; each switch hotkey
//! toggles its switch through the one dispatcher. Registration is
//! transactional: `apply_all` unregisters everything, registers the new
//! set, and on any failure restores the previous set.

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut, ShortcutState};

use crate::SharedState;
use crate::automation::{self, Action, SwitchOnOff};
use crate::settings::Hotkeys;

/// Parse a hotkey string and validate that it has at least one modifier
/// (Ctrl, Alt, Win) and is not the reserved Escape key.
pub fn parse_valid_hotkey(binding: &str) -> Result<Shortcut, String> {
    let shortcut = binding
        .parse::<Shortcut>()
        .map_err(|e| format!("invalid hotkey '{binding}': {e}"))?;
    let mods = shortcut.mods;
    if !mods.ctrl() && !mods.alt() && !mods.meta() {
        return Err(format!(
            "hotkey '{binding}' must include at least one of Ctrl, Alt, or Win"
        ));
    }
    if shortcut.key == Code::Escape {
        return Err("Escape is reserved: it unlocks the keyboard".into());
    }
    Ok(shortcut)
}

/// Register the whole hotkey set, replacing whatever is registered now.
/// Transactional: on any failure the previous set is re-registered and the
/// error names the binding that failed (FR-45 "rejected with a message").
pub fn apply_all(app: &AppHandle, previous: &Hotkeys, next: &Hotkeys) -> Result<(), String> {
    let old = hotkey_list(previous);
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| format!("unregistering the old hotkeys failed: {e}"))?;
    register_set(app, next, &old)
}

/// Launch registration (spec 7.11): registers each binding independently
/// so that a conflict on one switch does not unregister or disable Recording
/// Mode or other working hotkeys.
pub fn register_at_launch(app: &AppHandle, next: &Hotkeys) -> Result<(), Vec<String>> {
    let _ = app.global_shortcut().unregister_all();
    let mut failures = Vec::new();

    let rm_binding = &next.recording_mode;
    if !rm_binding.is_empty() {
        match parse_valid_hotkey(rm_binding) {
            Ok(shortcut) => {
                let result = app
                    .global_shortcut()
                    .on_shortcut(shortcut, move |app, _s, event| {
                        if event.state == ShortcutState::Pressed
                            && let Err(message) = automation::run(app, Action::Toggle)
                        {
                            automation::notify(app, &message.to_string());
                        }
                    });
                match result {
                    Ok(_) => {
                        set_registered(app, rm_binding, true);
                    }
                    Err(e) => {
                        set_registered(app, rm_binding, false);
                        failures.push(format!(
                            "recording mode hotkey '{rm_binding}' is not available: {e}"
                        ));
                    }
                }
            }
            Err(e) => {
                set_registered(app, rm_binding, false);
                failures.push(e);
            }
        }
    } else {
        set_registered(app, "", false);
    }

    for (id_str, binding) in &next.switches {
        if binding.is_empty() {
            continue;
        }
        let Ok(id) = automation::parse_switch_id(id_str) else {
            continue;
        };
        let shortcut = match parse_valid_hotkey(binding) {
            Ok(s) => s,
            Err(e) => {
                failures.push(e);
                continue;
            }
        };
        let action = Action::Switch {
            id,
            on: SwitchOnOff::Toggle,
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
            failures.push(format!(
                "switch hotkey '{binding}' ({id_str}) is not available: {e}"
            ));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

/// Register the whole hotkey set; on failure rolls back to `previous`.
fn register_set(
    app: &AppHandle,
    next: &Hotkeys,
    previous: &[(String, Action)],
) -> Result<(), String> {
    let mut registered: Vec<(String, tauri_plugin_global_shortcut::Shortcut)> = Vec::new();
    for (binding, action) in hotkey_list(next) {
        let shortcut = match parse_valid_hotkey(&binding) {
            Ok(shortcut) => shortcut,
            Err(message) => return rollback(app, previous, &message),
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
            return rollback(app, previous, &message);
        }
        registered.push((binding, shortcut));
    }

    // Record the recording-mode registration state for the UI (FR-9).
    let rm_binding = next.recording_mode.clone();
    let registered_ok = parse_valid_hotkey(&rm_binding)
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
    let mut rm_restored = false;
    let mut rm_binding = String::new();
    for (binding, action) in previous {
        let action = action.clone();
        let is_rm = matches!(action, Action::Toggle);
        if is_rm {
            rm_binding = binding.clone();
        }
        if let Ok(shortcut) = parse_valid_hotkey(binding) {
            match app
                .global_shortcut()
                .on_shortcut(shortcut, move |app, _s, event| {
                    if event.state == ShortcutState::Pressed
                        && let Err(message) = automation::run(app, action.clone())
                    {
                        automation::notify(app, &message.to_string());
                    }
                }) {
                Ok(_) => {
                    if is_rm {
                        rm_restored = true;
                    }
                }
                Err(e) => {
                    log::warn!("hotkey '{binding}' could not be restored after a failed save: {e}");
                }
            }
        }
    }
    if !rm_binding.is_empty() {
        set_registered(app, &rm_binding, rm_restored);
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
    fn parse_valid_hotkey_rejects_shift_only_and_escape() {
        assert!(parse_valid_hotkey("Shift+K").is_err());
        assert!(parse_valid_hotkey("Ctrl+Escape").is_err());
        assert!(parse_valid_hotkey("Alt+R").is_ok());
        assert!(parse_valid_hotkey("Ctrl+Alt+F10").is_ok());
    }

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
