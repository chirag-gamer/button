//! Screen Saver (FR-32, spec 7.5, task 4.4). An action, not a state: "on"
//! starts the configured screen saver now via
//! `SendMessage(GetDesktopWindow(), WM_SYSCOMMAND, SC_SCREENSAVE)`. Never
//! journaled, never timed. A saver is "set" when `SCRNSAVE.EXE` is
//! non-empty in the policy key, else the user key (probe 2026-10-27: the
//! owner's PC has ScreenSaveActive="1" but no SCRNSAVE.EXE — only the exe
//! proves a saver is configured).

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::SC_SCREENSAVE;
use windows::Win32::UI::WindowsAndMessaging::{
    GetDesktopWindow, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SYSCOMMAND,
};

use super::{Availability, Ctx, Switch, SwitchError, SwitchId};
use crate::os::OsInfo;

const POLICY_DESKTOP: &str = r"Software\Policies\Microsoft\Windows\Control Panel\Desktop";
const USER_DESKTOP: &str = r"Control Panel\Desktop";
const NOT_SET: &str = "No screen saver is set. Choose one in Settings \u{203a} Personalization \u{203a} Lock screen \u{203a} Screen saver.";

/// State shape: `{"configured": bool}` — actions carry no on/off state.
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub configured: bool,
}

pub struct ScreenSaver;

impl Switch for ScreenSaver {
    fn id(&self) -> SwitchId {
        SwitchId::ScreenSaver
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        // A missing saver is not a permanent disability: the row stays
        // usable and the click explains what to do (FR-32 acceptance).
        Availability::Available
    }

    fn persistent(&self) -> bool {
        false // never journaled
    }

    fn is_action(&self) -> bool {
        true
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        Ok(serde_json::json!(State {
            configured: configured_saver().is_some(),
        }))
    }

    fn is_on(&self, _value: &serde_json::Value) -> bool {
        false // an action has no on state
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!({ "start": true })
    }

    fn apply(&self, _value: &serde_json::Value) -> Result<(), SwitchError> {
        if configured_saver().is_none() {
            return Err(SwitchError::NotAvailable(NOT_SET.into()));
        }
        // WM_SYSCOMMAND legitimately returns 0 (DefWindowProc), so a zero
        // result alone is not failure; only a recorded last error says the
        // send timed out or was dropped (G-4: no silent failure).
        let sent = unsafe {
            // The 2 s timeout keeps a busy desktop from blocking the caller.
            SendMessageTimeoutW(
                GetDesktopWindow(),
                WM_SYSCOMMAND,
                WPARAM(SC_SCREENSAVE as usize),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                2000,
                None,
            )
        };
        if sent.0 == 0 {
            let code = unsafe { windows::Win32::Foundation::GetLastError() }.0 as i32;
            if code != 0 {
                return Err(SwitchError::Os {
                    api: "SendMessageTimeoutW",
                    code,
                });
            }
        }
        Ok(())
    }
}

/// The configured saver's path, policy key first (probe, task 4.4).
fn configured_saver() -> Option<String> {
    for path in [POLICY_DESKTOP, USER_DESKTOP] {
        if let Ok(key) = windows_registry::CURRENT_USER.open(path)
            && let Ok(exe) = key.get_string("SCRNSAVE.EXE")
            && !exe.is_empty()
        {
            return Some(exe);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::switches::parse_state;

    #[test]
    fn state_shape_round_trips() {
        let state = parse_state::<State>(&serde_json::json!({ "configured": true }), "t").unwrap();
        assert!(state.configured);
        assert!(parse_state::<State>(&serde_json::json!({}), "t").is_err());
    }

    #[test]
    fn is_an_action_and_never_on() {
        assert!(ScreenSaver.is_action());
        assert!(!ScreenSaver.is_on(&serde_json::json!({ "configured": true })));
    }

    /// Read-only: which key wins and whether a saver is configured at all.
    #[test]
    fn read_reports_whether_a_saver_is_configured() {
        let value = ScreenSaver.read().unwrap();
        let state = parse_state::<State>(&value, "t").unwrap();
        assert_eq!(state.configured, configured_saver().is_some());
    }
}
