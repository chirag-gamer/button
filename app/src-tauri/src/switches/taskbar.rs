//! Hide Taskbar (FR-17, spec 7.5). Turns taskbar auto-hide on or off
//! through `SHAppBarMessage(ABM_GETSTATE / ABM_SETSTATE)`. Auto-hide is a
//! saved Explorer setting, so the journal entry is persistent (spec 7.3).

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::LPARAM;
use windows::Win32::UI::Shell::{
    ABM_GETSTATE, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
use windows::core::{PCWSTR, w};

use super::{Availability, Ctx, Switch, SwitchError, SwitchId, parse_state};
use crate::os::OsInfo;

/// State shape: `{"autoHide": bool}`.
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub auto_hide: bool,
}

pub struct Taskbar;

impl Switch for Taskbar {
    fn id(&self) -> SwitchId {
        SwitchId::Taskbar
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available
    }

    fn persistent(&self) -> bool {
        true
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        Ok(serde_json::json!(State {
            auto_hide: auto_hide()?,
        }))
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!(State { auto_hide: true })
    }

    fn target_off(&self, _current: &serde_json::Value, _ctx: &Ctx) -> Option<serde_json::Value> {
        Some(serde_json::json!(State { auto_hide: false }))
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "taskbar")?;
        if auto_hide()? == target.auto_hide {
            return Ok(()); // already in that state (M-7), nothing to set
        }
        set_auto_hide(target.auto_hide)?;
        // Verify the shell took the new state instead of trusting the
        // ABM_SETSTATE return, which is not documented for this message.
        if auto_hide()? != target.auto_hide {
            return Err(SwitchError::NotAvailable(
                "the taskbar did not accept the new auto-hide state".into(),
            ));
        }
        Ok(())
    }

    // restore keeps the trait default: auto-hide is re-set to the old value
    // only when it still reads as ours; a user change in between reports
    // UserChanged (research/02 rule).

    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "taskbar")
            .map(|state| state.auto_hide)
            .unwrap_or(false)
    }
}

/// `ABM_GETSTATE`, auto-hide bit only (the on-top bit is not ours to judge).
fn auto_hide() -> Result<bool, SwitchError> {
    unsafe {
        let mut abd = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            ..Default::default()
        };
        let state = SHAppBarMessage(ABM_GETSTATE, &mut abd);
        Ok((state as u32 & ABS_AUTOHIDE) != 0)
    }
}

fn set_auto_hide(on: bool) -> Result<(), SwitchError> {
    unsafe {
        let mut abd = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            ..Default::default()
        };
        // SETSTATE ignores hWnd, but a missing Shell_TrayWnd means no
        // taskbar to talk about (spec 7.6 spirit: fail loudly).
        abd.hWnd = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null())
            .map_err(|_| SwitchError::NotAvailable("taskbar window not found".into()))?;
        abd.lParam = LPARAM(if on { ABS_AUTOHIDE as isize } else { 0 });
        SHAppBarMessage(ABM_SETSTATE, &mut abd);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shape_is_auto_hide_only() {
        let state = parse_state::<State>(&serde_json::json!({ "autoHide": true }), "t").unwrap();
        assert!(state.auto_hide);
        assert!(parse_state::<State>(&serde_json::json!(false), "t").is_err());
        assert!(parse_state::<State>(&serde_json::json!({}), "t").is_err());
    }

    #[test]
    fn is_on_means_auto_hide_engaged() {
        assert!(Taskbar.is_on(&serde_json::json!(State { auto_hide: true })));
        assert!(!Taskbar.is_on(&serde_json::json!(State { auto_hide: false })));
    }

    #[test]
    fn target_on_is_auto_hide() {
        let after = Taskbar.target_on(
            &serde_json::json!(State { auto_hide: false }),
            &Ctx::default(),
        );
        assert_eq!(after, serde_json::json!(State { auto_hide: true }));
        // Already-set case stays a skip in start (spec 7.1.2, M-7).
        assert_eq!(
            Taskbar.target_on(
                &serde_json::json!(State { auto_hide: true }),
                &Ctx::default()
            ),
            after
        );
    }

    /// Read-only live probe: ABM_GETSTATE never changes anything; the
    /// parsed shape is the assertion.
    #[test]
    fn read_reports_the_live_auto_hide_state() {
        let value = Taskbar.read().unwrap();
        parse_state::<State>(&value, "t").unwrap();
    }
}
