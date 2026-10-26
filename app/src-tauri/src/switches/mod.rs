//! The switch contract every Windows mechanism implements (spec 7.5).
//! Only `switches/*` may touch Win32/COM (spec 2.1 boundaries).

use serde::{Deserialize, Serialize};

use crate::journal::JournalEntry;
use crate::os::OsInfo;

#[cfg(test)]
pub mod fake;

mod desktop_icons;
mod taskbar;

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

/// The registered switches (spec 2.1). Real switches register here as their
/// Phase 3 tasks land them, in debug and release alike; the list is empty
/// until 3.1 (D-067). Tests get the fakes instead.
pub fn build_registry(data_dir: &std::path::Path) -> Vec<Box<dyn Switch>> {
    // data_dir comes back into use by later fakes (3.3 removes the last one).
    let _ = data_dir;
    let mut switches: Vec<Box<dyn Switch>> = vec![
        Box::new(desktop_icons::DesktopIcons),
        Box::new(taskbar::Taskbar),
    ];
    #[cfg(test)]
    switches.extend(test_fakes());
    #[cfg(debug_assertions)]
    if let Ok(fail_id) = std::env::var("BUTTON_FAIL") {
        switches = switches
            .into_iter()
            .map(|s| {
                if s.id().to_string() == fail_id {
                    Box::new(FailOnApply { inner: s }) as Box<dyn Switch>
                } else {
                    s
                }
            })
            .collect();
    }
    switches
}

/// Debug-only wrapper (D-067): with `BUTTON_FAIL=<id>` the named switch's
/// apply fails so M-6 can be exercised on a normal debug build.
#[cfg(debug_assertions)]
struct FailOnApply {
    inner: Box<dyn Switch>,
}

#[cfg(debug_assertions)]
impl Switch for FailOnApply {
    fn id(&self) -> SwitchId {
        self.inner.id()
    }

    fn experimental(&self) -> bool {
        self.inner.experimental()
    }

    fn availability(&self, os: &OsInfo) -> Availability {
        self.inner.availability(os)
    }

    fn persistent(&self) -> bool {
        self.inner.persistent()
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        self.inner.read()
    }

    fn is_on(&self, value: &serde_json::Value) -> bool {
        self.inner.is_on(value)
    }

    fn target_on(&self, before: &serde_json::Value, ctx: &Ctx) -> serde_json::Value {
        self.inner.target_on(before, ctx)
    }

    fn apply(&self, _value: &serde_json::Value) -> Result<(), SwitchError> {
        Err(SwitchError::NotAvailable(format!(
            "BUTTON_FAIL forced {} to fail",
            self.id()
        )))
    }

    fn restore(&self, e: &JournalEntry) -> Result<RestoreOutcome, SwitchError> {
        self.inner.restore(e)
    }
}

/// The fakes for switches that are not real yet, compiled for tests only
/// (D-063 as updated by D-067). Each fake disappears when its real switch
/// lands (desktop icons went real in 3.1, taskbar in 3.2).
#[cfg(test)]
fn test_fakes() -> Vec<Box<dyn Switch>> {
    vec![Box::new(fake::FakeSwitch::new(
        SwitchId::KeepAwake,
        fake::FakeOptions::default(),
    ))]
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

/// Per-start context for `target_on`, built by automation from settings and
/// the front tracker (D-066). session.rs and the switches stay Tauri-free.
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    pub front: Option<crate::front::FrontApp>,
    pub stays_on_screen: Vec<String>,
    pub tray_visible: Vec<String>,
    pub backdrop_color: String,
    pub own_pid: u32,
    pub own_exe: String,
}

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

    /// How the UI reads "on" from this switch's state (spec 5.4): the
    /// default fits bool states; structured states override it.
    fn is_on(&self, value: &serde_json::Value) -> bool {
        value.as_bool().unwrap_or(false)
    }

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

/// Parses a switch state value into `T`; a wrong shape is `Invalid`, never a
/// panic (task 3.0). Every switch impl parses its state through this helper.
pub fn parse_state<T: serde::de::DeserializeOwned>(
    value: &serde_json::Value,
    what: &str,
) -> Result<T, SwitchError> {
    serde_json::from_value(value.clone()).map_err(|e| SwitchError::Invalid(format!("{what}: {e}")))
}

/// Exe names and full paths compare case-insensitively on Windows (task 3.0).
pub(crate) fn same_exe(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::JournalOwner;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Shape {
        visible: bool,
    }

    #[test]
    fn parse_state_accepts_the_documented_shape() {
        let state = parse_state::<Shape>(&serde_json::json!({ "visible": false }), "test");
        assert!(!state.unwrap().visible);
    }

    #[test]
    fn parse_state_reports_wrong_shapes_as_invalid() {
        for wrong in [
            serde_json::json!({ "visible": "yes" }),
            serde_json::json!(true),
            serde_json::json!({}),
        ] {
            let err = parse_state::<Shape>(&wrong, "test").unwrap_err();
            assert!(matches!(err, SwitchError::Invalid(_)));
        }
    }

    #[test]
    fn exe_names_compare_case_insensitively() {
        assert!(same_exe("OBS64.EXE", "obs64.exe"));
        assert!(!same_exe("obs64.exe", "obs.exe"));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn fail_on_apply_wraps_apply_only() {
        let fake = fake::FakeSwitch::new(SwitchId::KeepAwake, fake::FakeOptions::default());
        let handle = fake.handle();
        handle.set(true); // the fake sits in the `after` state
        let wrapped = FailOnApply {
            inner: Box::new(fake),
        };
        assert!(wrapped.read().is_ok());
        assert!(matches!(
            wrapped.apply(&serde_json::json!(true)),
            Err(SwitchError::NotAvailable(_))
        ));
        // With current == after, a delegated restore applies `before` through
        // the fake and reports Restored. The trait's default restore would
        // call this wrapper's failing apply instead and error out — the
        // assertion catches exactly that regression.
        let entry = JournalEntry {
            seq: 0,
            owner: JournalOwner::Recording,
            switch: SwitchId::KeepAwake,
            before: serde_json::json!(false),
            after: serde_json::json!(true),
            persistent: false,
            off_at: None,
            at: time::OffsetDateTime::UNIX_EPOCH,
        };
        assert!(matches!(
            wrapped.restore(&entry),
            Ok(RestoreOutcome::Restored)
        ));
    }
}
