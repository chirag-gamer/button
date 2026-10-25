pub mod journal;
pub mod os;
pub mod session;
pub mod switches;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use os::OsInfo;
use switches::{Availability, SwitchId};

/// The per-switch item the UI sees (spec 5.4).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchState {
    pub id: SwitchId,
    pub on: bool,
    pub availability: Availability,
    pub experimental: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub off_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedSwitch {
    pub id: SwitchId,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Idle,
    Countdown,
    Applying,
    Active,
    Restoring,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub status: SessionStatus,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub started_at: Option<OffsetDateTime>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub ends_at: Option<OffsetDateTime>,
    pub applied: Vec<SwitchId>,
    pub skipped: Vec<SkippedSwitch>,
    pub failed: Vec<SkippedSwitch>,
}

impl SessionState {
    pub fn idle() -> Self {
        Self {
            status: SessionStatus::Idle,
            started_at: None,
            ends_at: None,
            applied: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyState {
    pub binding: String,
    pub registered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub version: String,
}

/// The whole in-memory model shared with the UI (spec 5.4).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub session: SessionState,
    pub switches: Vec<SwitchState>,
    pub hotkey: HotkeyState,
    pub os: OsInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<UpdateState>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use switches::Availability;

    fn test_state() -> AppState {
        AppState {
            session: SessionState::idle(),
            switches: vec![
                SwitchState {
                    id: SwitchId::DesktopIcons,
                    on: false,
                    availability: Availability::Available,
                    experimental: false,
                    off_at: None,
                    detail: None,
                },
                SwitchState {
                    id: SwitchId::TrayIcons,
                    on: false,
                    availability: Availability::NeedsWindows11,
                    experimental: false,
                    off_at: None,
                    detail: Some("Needs Windows 11".into()),
                },
            ],
            hotkey: HotkeyState {
                binding: "Alt+R".into(),
                registered: true,
            },
            os: OsInfo {
                build: 26200,
                is_windows_11: true,
            },
            update: None,
        }
    }

    // spec 5.4: exact key names and casing in the serialized AppState.
    #[test]
    fn app_state_serializes_with_ui_key_names() {
        let json = serde_json::to_string(&test_state()).unwrap();
        assert!(json.contains("\"session\""), "{json}");
        assert!(json.contains("\"status\":\"idle\""), "{json}");
        assert!(json.contains("\"isWindows11\""), "{json}");
        assert!(json.contains("\"id\":\"desktop-icons\""), "{json}");
        assert!(
            json.contains("\"availability\":\"needs-windows-11\""),
            "{json}"
        );
        assert!(json.contains("\"registered\":true"), "{json}");
        assert!(!json.contains("update"), "{json}");
    }
}
