//! The one dispatcher every trigger goes through: tray menu, tray clicks,
//! hotkey, dropdown and the command line (spec 2.2, 6.3).

use tauri::{AppHandle, Emitter, Manager};
use time::OffsetDateTime;

use crate::switches::{Availability, SwitchId};
use crate::tray;
use crate::{AppState, SessionStatus, SharedState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Toggle,
    /// Toggle the dropdown window (tray left click, `button.exe` with no args).
    ShowDropdown,
    /// `button.exe switch <id> on|off|toggle`. Quick switches arrive in
    /// Phase 4; until then every switch reports NotBuilt (FR-27, FR-28).
    Switch {
        id: SwitchId,
        on: SwitchOnOff,
    },
    /// `button.exe restore`: restore any leftover journal (spec 6.3, U-7).
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchOnOff {
    On,
    Off,
    Toggle,
}

/// Strict parser for `button.exe ...` (spec 6.3). The exe path must already
/// be stripped. Anything else than the exact grammar is an error.
///
/// ```text
/// button.exe []                    -> ShowDropdown
/// button.exe start|stop|toggle     -> Start | Stop | Toggle
/// button.exe switch <id> on|off|toggle
/// button.exe restore               -> Restore
/// ```
pub fn parse(args: &[String]) -> Result<Action, String> {
    let mut args = args.iter().map(String::as_str);
    let first = args.next();
    match first {
        None => Ok(Action::ShowDropdown),
        Some("start") | Some("stop") | Some("toggle") | Some("restore") => {
            if args.next().is_some() {
                return Err(format!(
                    "unexpected extra argument after '{}'",
                    first.unwrap()
                ));
            }
            Ok(match first.unwrap() {
                "start" => Action::Start,
                "stop" => Action::Stop,
                "toggle" => Action::Toggle,
                _ => Action::Restore,
            })
        }
        Some("switch") => {
            let id = args
                .next()
                .ok_or_else(|| "switch needs an id: switch <id> on|off|toggle".to_string())?;
            let id = parse_switch_id(id)?;
            let on = args
                .next()
                .ok_or_else(|| "switch needs on|off|toggle".to_string())?;
            let on = match on {
                "on" => SwitchOnOff::On,
                "off" => SwitchOnOff::Off,
                "toggle" => SwitchOnOff::Toggle,
                other => return Err(format!("expected on|off|toggle, got '{other}'")),
            };
            if let Some(extra) = args.next() {
                return Err(format!("unexpected extra argument '{extra}'"));
            }
            Ok(Action::Switch { id, on })
        }
        Some(other) => Err(format!(
            "unknown command '{other}': use start|stop|toggle|switch <id> on|off|toggle|restore"
        )),
    }
}

/// Switch ids in the CLI use the kebab-case names from the dropdown (5.4).
pub fn parse_switch_id(id: &str) -> Result<SwitchId, String> {
    serde_json::from_value(serde_json::json!(id)).map_err(|_| format!("unknown switch '{id}'"))
}

/// What launch-time recovery did; the FR-6 notification (task 2.9) reads it.
#[allow(dead_code)]
pub struct RecoveryNotice(pub crate::session::RecoveryReport);

/// The work after every state change, in one place: tray icon/tooltip/menu
/// text, then the state-changed event to the UI (spec 2.5 instruction).
pub fn after_change(app: &AppHandle) {
    tray::update_tray(app);
    let state = app.state::<SharedState>();
    let b = state.lock().unwrap_or_else(|p| p.into_inner());
    let app_state: AppState = crate::build_app_state(&b);
    drop(b);
    let _ = app.emit("state-changed", app_state);
}

pub fn run(app: &AppHandle, action: Action) {
    let state = app.state::<SharedState>();
    let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
    let now = OffsetDateTime::now_utc();

    let active = b.engine.state().status == SessionStatus::Active;
    let result = match action {
        Action::Start if !active => b.engine.start(now).map(|_| ()).map_err(|e| e.to_string()),
        Action::Start => Err(session_active()),
        Action::Stop if active => b.engine.stop(now).map(|_| ()).map_err(|e| e.to_string()),
        Action::Stop => Err(no_session()),
        Action::Toggle if active => b.engine.stop(now).map(|_| ()).map_err(|e| e.to_string()),
        Action::Toggle => b.engine.start(now).map(|_| ()).map_err(|e| e.to_string()),
        Action::ShowDropdown => {
            drop(b);
            tray::toggle_dropdown(app);
            return;
        }
        Action::Switch { id, on } => {
            match b.engine.switch_availability(id) {
                Some(Availability::Available) => {
                    // Registered in this build (debug fakes); execution of
                    // quick switches is Phase 4 (FR-27, FR-28).
                    notify(&format!("{id} {on:?}: quick switches arrive in Phase 4"));
                }
                _ => notify(&format!("{id}: not available in this build")),
            }
            drop(b);
            return;
        }
        Action::Restore => {
            match b.engine.recover(now) {
                Ok(_) => notify("Journal restored (if anything was left)"),
                Err(e) => notify(&format!("Restore failed: {e}")),
            }
            drop(b);
            after_change(app);
            return;
        }
    };
    drop(b);
    match result {
        Ok(()) => after_change(app),
        Err(e) => notify(&e),
    }
}

fn session_active() -> String {
    "A session is already active".into()
}

fn no_session() -> String {
    "No session is active".into()
}

/// Failures and CLI results go to a notification (spec 6.3: button has no
/// console output). The notification plugin is wired up in task 2.9; until
/// then these land in the log crate (no-op until 2.9 installs the logger).
fn notify(message: &str) {
    log::warn!("{message}");
}

#[cfg(test)]
mod tests {
    use super::{Action, SwitchOnOff, parse, parse_switch_id};

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_grammar_form_parses() {
        assert_eq!(parse(&args(&[])), Ok(Action::ShowDropdown));
        assert_eq!(parse(&args(&["start"])), Ok(Action::Start));
        assert_eq!(parse(&args(&["stop"])), Ok(Action::Stop));
        assert_eq!(parse(&args(&["toggle"])), Ok(Action::Toggle));
        assert_eq!(parse(&args(&["restore"])), Ok(Action::Restore));
        assert_eq!(
            parse(&args(&["switch", "desktop-icons", "on"])),
            Ok(Action::Switch {
                id: crate::SwitchId::DesktopIcons,
                on: SwitchOnOff::On
            })
        );
        assert_eq!(
            parse(&args(&["switch", "keep-awake", "off"])),
            Ok(Action::Switch {
                id: crate::SwitchId::KeepAwake,
                on: SwitchOnOff::Off
            })
        );
        assert_eq!(
            parse(&args(&["switch", "taskbar", "toggle"])),
            Ok(Action::Switch {
                id: crate::SwitchId::Taskbar,
                on: SwitchOnOff::Toggle
            })
        );
    }

    #[test]
    fn junk_is_rejected_with_a_message() {
        assert!(parse(&args(&["start", "x"])).is_err());
        assert!(parse(&args(&["frobnicate"])).is_err());
        assert!(parse(&args(&["switch"])).is_err());
        assert!(parse(&args(&["switch", "no-such-switch", "on"])).is_err());
        assert!(parse(&args(&["switch", "taskbar", "maybe"])).is_err());
        assert!(parse(&args(&["switch", "taskbar", "on", "extra"])).is_err());
        assert!(parse(&args(&["SWITCH"])).is_err(), "case sensitive");
        // Junk that parses as a kebab id name fails id validation.
        assert!(parse_switch_id("desktop icons").is_err());
    }
}
