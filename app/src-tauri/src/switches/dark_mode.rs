//! Dark Mode (FR-31, spec 7.5, task 4.3). Reads and writes
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`
//! (`AppsUseLightTheme` / `SystemUsesLightTheme`, DWORD 0 = dark; a missing
//! value means light) and broadcasts `WM_SETTINGCHANGE "ImmersiveColorSet"`
//! so apps and the shell pick the theme up immediately (U-3).

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};

use super::{Availability, Ctx, Switch, SwitchError, SwitchId, parse_state};
use crate::os::OsInfo;

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

/// State shape: `{"apps": "light"|"dark", "system": "light"|"dark"}` (spec 5.3).
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub apps: Theme,
    pub system: Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}

pub struct DarkMode;

impl Switch for DarkMode {
    fn id(&self) -> SwitchId {
        SwitchId::DarkMode
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available
    }

    fn persistent(&self) -> bool {
        true // a saved Explorer setting
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        let key = windows_registry::CURRENT_USER
            .open(PERSONALIZE)
            .map_err(reg("open Themes\\Personalize"))?;
        // Missing values mean light (spec 7.5, task 4.3).
        let apps = read_theme(&key, "AppsUseLightTheme")?;
        let system = read_theme(&key, "SystemUsesLightTheme")?;
        Ok(serde_json::json!(State { apps, system }))
    }

    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "dark mode")
            .map(|s| s.apps == Theme::Dark && s.system == Theme::Dark)
            .unwrap_or(false)
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!(State {
            apps: Theme::Dark,
            system: Theme::Dark,
        })
    }

    fn target_off(&self, _current: &serde_json::Value, _ctx: &Ctx) -> Option<serde_json::Value> {
        Some(serde_json::json!(State {
            apps: Theme::Light,
            system: Theme::Light,
        }))
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "dark mode")?;
        let key = windows_registry::CURRENT_USER
            .create(PERSONALIZE)
            .map_err(reg("create Themes\\Personalize"))?;
        key.set_u32("AppsUseLightTheme", light_u32(target.apps))
            .map_err(reg("set AppsUseLightTheme"))?;
        key.set_u32("SystemUsesLightTheme", light_u32(target.system))
            .map_err(reg("set SystemUsesLightTheme"))?;
        broadcast_immersive_color_set();
        Ok(())
    }

    // restore keeps the trait default: re-apply `before` only when the
    // themes still equal `after` (a user change in between is left alone).
}

fn reg(api: &'static str) -> impl Fn(windows_core::Error) -> SwitchError {
    move |e| SwitchError::Os {
        api,
        code: e.code().0,
    }
}

fn read_theme(key: &windows_registry::Key, name: &str) -> Result<Theme, SwitchError> {
    // Missing or unparsable counts as light (the Windows default).
    Ok(key
        .get_u32(name)
        .map(|v| if v == 0 { Theme::Dark } else { Theme::Light })
        .unwrap_or(Theme::Light))
}

fn light_u32(theme: Theme) -> u32 {
    match theme {
        Theme::Light => 1,
        Theme::Dark => 0,
    }
}

/// Tell every top-level window the theme changed (U-3). Best effort: a
/// hung receiver only delays its own pickup (SMTO_ABORTIFHUNG).
fn broadcast_immersive_color_set() {
    let topic: Vec<u16> = "ImmersiveColorSet".encode_utf16().chain([0]).collect();
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(topic.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            1000,
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shape_round_trips() {
        let state = parse_state::<State>(
            &serde_json::json!({ "apps": "dark", "system": "light" }),
            "t",
        )
        .unwrap();
        assert_eq!(state.apps, Theme::Dark);
        assert_eq!(state.system, Theme::Light);
        assert!(parse_state::<State>(&serde_json::json!(true), "t").is_err());
        assert!(parse_state::<State>(&serde_json::json!({}), "t").is_err());
        assert!(
            parse_state::<State>(&serde_json::json!({ "apps": "dark" }), "t").is_err(),
            "system is part of the shape"
        );
    }

    #[test]
    fn is_on_means_both_themes_dark() {
        assert!(DarkMode.is_on(&serde_json::json!({"apps": "dark", "system": "dark"})));
        assert!(!DarkMode.is_on(&serde_json::json!({"apps": "dark", "system": "light"})));
        assert!(!DarkMode.is_on(&serde_json::json!({"apps": "light", "system": "light"})));
    }

    #[test]
    fn targets_are_both_dark_on_and_both_light_off() {
        let on = DarkMode.target_on(&serde_json::json!({}), &Ctx::default());
        assert_eq!(on, serde_json::json!({ "apps": "dark", "system": "dark" }));
        let off = DarkMode
            .target_off(&serde_json::json!({}), &Ctx::default())
            .unwrap();
        assert_eq!(
            off,
            serde_json::json!({ "apps": "light", "system": "light" })
        );
    }

    /// Read-only live probe (U-3 evidence comes from the owner's manual
    /// check, docs/testing.md).
    #[test]
    fn read_reports_the_live_themes() {
        match DarkMode.read() {
            Ok(value) => {
                parse_state::<State>(&value, "t").unwrap();
            }
            Err(e) => panic!("dark mode read failed on a real desktop: {e}"),
        }
    }
}
