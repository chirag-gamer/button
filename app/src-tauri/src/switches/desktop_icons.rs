//! Hide Desktop Icons (FR-15, spec 7.5). Toggles Explorer's own "Show
//! desktop icons" command (`WM_COMMAND 0x7402`) on the desktop's
//! `SHELLDLL_DefView` (under `Progman`, else under a `WorkerW`). Icons never
//! move. The toggle lives in Explorer, so it outlives button's process and
//! crash recovery restores it (spec 7.3: persistent).

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{GetLastError, HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, IsWindowVisible, SMTO_ABORTIFHUNG,
    SendMessageTimeoutW, WM_COMMAND,
};
use windows::core::{BOOL, PCWSTR, w};

use super::{Availability, Ctx, Switch, SwitchError, SwitchId, parse_state};
use crate::os::OsInfo;

/// State shape: `{"visible": bool}`.
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct State {
    pub visible: bool,
}

/// 0x7402 is Explorer's "Show desktop icons" toggle command (UNDOC, stable
/// since XP, research/02).
const SHOW_DESKTOP_ICONS: usize = 0x7402;

pub struct DesktopIcons;

impl Switch for DesktopIcons {
    fn id(&self) -> SwitchId {
        SwitchId::DesktopIcons
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available
    }

    fn persistent(&self) -> bool {
        // The toggle is Explorer's state, not button-owned process state:
        // it outlives a crash, so recovery must undo it (spec 7.3).
        true
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        Ok(serde_json::json!(State {
            visible: read_visible()?,
        }))
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!(State { visible: false })
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "desktop icons")?;
        let defview = defview()
            .ok_or_else(|| SwitchError::NotAvailable("desktop shell view not found".into()))?;
        if folder_visible(defview)? == target.visible {
            return Ok(()); // already in that state, nothing to send
        }
        send_toggle(defview)
    }

    // restore keeps the trait default: read() == after means we hid them, so
    // apply(before) toggles back; anything else is the user's own change.

    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "desktop icons")
            .map(|state| !state.visible)
            .unwrap_or(false)
    }
}

/// The desktop's `SHELLDLL_DefView`: first under `Progman`, else under a
/// `WorkerW` sibling (the Wallpaper/Spriteshell split, probe pattern).
fn defview() -> Option<HWND> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null()).ok()?;
        if let Ok(dv) = FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), PCWSTR::null()) {
            return Some(dv);
        }
        let mut found = HWND::default();
        let _ = EnumWindows(
            Some(workerw_defview),
            LPARAM(&mut found as *mut HWND as isize),
        );
        (!found.is_invalid()).then_some(found)
    }
}

unsafe extern "system" fn workerw_defview(top: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        if let Ok(dv) = FindWindowExW(Some(top), None, w!("SHELLDLL_DefView"), PCWSTR::null()) {
            *(lparam.0 as *mut HWND) = dv;
            return BOOL(0); // stop the walk
        }
        BOOL(1)
    }
}

/// The icon list is the DefView's `SysListView32` "FolderView" child; when
/// the icons are hidden Explorer hides it (spec 7.5 state).
fn folder_visible(defview: HWND) -> Result<bool, SwitchError> {
    unsafe {
        let folder = FindWindowExW(Some(defview), None, w!("SysListView32"), w!("FolderView"))
            .ok()
            .ok_or_else(|| SwitchError::NotAvailable("desktop icon list not found".into()))?;
        Ok(IsWindowVisible(folder).as_bool())
    }
}

fn read_visible() -> Result<bool, SwitchError> {
    let defview = defview()
        .ok_or_else(|| SwitchError::NotAvailable("desktop shell view not found".into()))?;
    folder_visible(defview)
}

/// The toggle goes through a timeout send so a hung Explorer can't hang
/// button (SMTO_ABORTIFHUNG).
fn send_toggle(defview: HWND) -> Result<(), SwitchError> {
    unsafe {
        let sent = SendMessageTimeoutW(
            defview,
            WM_COMMAND,
            WPARAM(SHOW_DESKTOP_ICONS),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            2000,
            None,
        );
        if sent.0 != 0 {
            Ok(())
        } else {
            Err(SwitchError::Os {
                api: "SendMessageTimeoutW",
                code: GetLastError().0 as i32,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shape_is_visible_only() {
        let state = parse_state::<State>(&serde_json::json!({ "visible": false }), "t").unwrap();
        assert!(!state.visible);
        assert!(parse_state::<State>(&serde_json::json!(true), "t").is_err());
        assert!(parse_state::<State>(&serde_json::json!({}), "t").is_err());
    }

    #[test]
    fn is_on_means_icons_hidden() {
        assert!(DesktopIcons.is_on(&serde_json::json!(State { visible: false })));
        assert!(!DesktopIcons.is_on(&serde_json::json!(State { visible: true })));
        // The default bool shape (fakes) still reads as off here.
        assert!(!DesktopIcons.is_on(&serde_json::json!(true)));
    }

    #[test]
    fn target_on_is_hidden_regardless_of_before() {
        let after =
            DesktopIcons.target_on(&serde_json::json!(State { visible: true }), &Ctx::default());
        assert_eq!(after, serde_json::json!(State { visible: false }));
        // Start skips a switch that is already in its target state (spec 7.1.2).
        assert_eq!(
            DesktopIcons.target_on(
                &serde_json::json!(State { visible: false }),
                &Ctx::default()
            ),
            after
        );
    }

    /// Read-only live probe: on a real desktop the state parses to a bool;
    /// without a desktop (CI session) the switch reports NotAvailable.
    /// Never sends the toggle.
    #[test]
    fn read_reports_the_live_desktop_or_not_available() {
        match DesktopIcons.read() {
            Ok(value) => {
                let state = parse_state::<State>(&value, "t").unwrap();
                assert_eq!(
                    state.visible,
                    defview().is_some_and(|dv| folder_visible(dv).unwrap_or(false))
                );
            }
            Err(SwitchError::NotAvailable(_)) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
