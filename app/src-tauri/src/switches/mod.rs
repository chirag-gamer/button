//! The switch contract every Windows mechanism implements (spec 7.5).
//! Only `switches/*` may touch Win32/COM (spec 2.1 boundaries).

use serde::{Deserialize, Serialize};

use crate::journal::JournalEntry;
use crate::os::OsInfo;

#[cfg(any(test, debug_assertions))]
pub mod fake;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SwitchId {
    Dnd,
    DesktopIcons,
    OtherApps,
    Taskbar,
    TrayIcons,
    Backdrop,
    KeepAwake,
    AlertSounds,
    DarkMode,
    NightLight,
    ScreenSaver,
    Microphone,
    LockKeyboard,
    Bluetooth,
}

/// Recording Mode's fixed apply order (spec 7.1).
pub const RECORDING_ORDER: [SwitchId; 10] = [
    SwitchId::Dnd,
    SwitchId::AlertSounds,
    SwitchId::KeepAwake,
    SwitchId::OtherApps,
    SwitchId::Backdrop,
    SwitchId::DesktopIcons,
    SwitchId::Taskbar,
    SwitchId::TrayIcons,
    SwitchId::DarkMode,
    SwitchId::NightLight,
];

/// Kebab-case display, same as the serde name (used for log lines and the
/// fake-switches.json keys).
impl std::fmt::Display for SwitchId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Dnd => "dnd",
            Self::DesktopIcons => "desktop-icons",
            Self::OtherApps => "other-apps",
            Self::Taskbar => "taskbar",
            Self::TrayIcons => "tray-icons",
            Self::Backdrop => "backdrop",
            Self::KeepAwake => "keep-awake",
            Self::AlertSounds => "alert-sounds",
            Self::DarkMode => "dark-mode",
            Self::NightLight => "night-light",
            Self::ScreenSaver => "screen-saver",
            Self::Microphone => "microphone",
            Self::LockKeyboard => "lock-keyboard",
            Self::Bluetooth => "bluetooth",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Availability {
    Available,
    // kebab-case alone would produce "needs-windows11" (no hyphen before digits).
    #[serde(rename = "needs-windows-11")]
    NeedsWindows11,
    ProbeFailed,
    NotBuilt,
}

/// Per-switch context for `target_on` (front app, monitor choice).
/// Filled in by Phase 3 tasks; empty on purpose until then.
#[derive(Debug, Clone, Default)]
pub struct Ctx {}

#[derive(Debug, thiserror::Error)]
pub enum SwitchError {
    #[error("{0}")]
    NotAvailable(String),
    #[error("{api} failed with code {code}")]
    Os { api: &'static str, code: i32 },
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcome {
    Restored,
    UserChanged,
    Gone,
}

pub trait Switch: Send + Sync {
    fn id(&self) -> SwitchId;

    fn experimental(&self) -> bool {
        false
    }

    fn availability(&self, os: &OsInfo) -> Availability;

    fn persistent(&self) -> bool;

    fn read(&self) -> Result<serde_json::Value, SwitchError>;

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value;

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError>;

    fn restore(&self, e: &JournalEntry) -> Result<RestoreOutcome, SwitchError> {
        let current = self.read()?;
        if current == e.after {
            self.apply(&e.before)?;
            Ok(RestoreOutcome::Restored)
        } else {
            Ok(RestoreOutcome::UserChanged)
        }
    }
}
