//! settings.json: load, validate, migrate, save (spec 5.1, 7.12, FR-23).

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::switches::SwitchId;
use crate::write_atomic;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub schema_version: u32,
    pub recording_mode: RecordingMode,
    pub hotkeys: Hotkeys,
    pub dropdown: Dropdown,
    pub apps: Apps,
    pub tray_icons: TrayIcons,
    pub backdrop: Backdrop,
    pub keep_awake: KeepAwake,
    pub bluetooth: Bluetooth,
    pub general: General,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            recording_mode: RecordingMode::default(),
            hotkeys: Hotkeys::default(),
            dropdown: Dropdown::default(),
            apps: Apps::default(),
            tray_icons: TrayIcons::default(),
            backdrop: Backdrop::default(),
            keep_awake: KeepAwake::default(),
            bluetooth: Bluetooth::default(),
            general: General::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RecordingMode {
    pub actions: Vec<SwitchId>,
    pub countdown: bool,
    pub time_limit_minutes: Option<u32>,
}

impl Default for RecordingMode {
    fn default() -> Self {
        Self {
            actions: vec![
                SwitchId::Dnd,
                SwitchId::DesktopIcons,
                SwitchId::OtherApps,
                SwitchId::Taskbar,
                SwitchId::TrayIcons,
                SwitchId::Backdrop,
                SwitchId::KeepAwake,
                SwitchId::AlertSounds,
            ],
            countdown: false,
            time_limit_minutes: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hotkeys {
    pub recording_mode: String,
    /// Per-switch accelerators: switch id -> accelerator string (FR-45).
    pub switches: HashMap<String, String>,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            recording_mode: "Alt+R".into(),
            switches: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Dropdown {
    pub shown: Vec<SwitchId>,
    pub more: Vec<SwitchId>,
    pub hidden: Vec<SwitchId>,
}

impl Default for Dropdown {
    fn default() -> Self {
        let all: Vec<SwitchId> = vec![
            SwitchId::Dnd,
            SwitchId::DesktopIcons,
            SwitchId::OtherApps,
            SwitchId::Taskbar,
            SwitchId::TrayIcons,
            SwitchId::Backdrop,
            SwitchId::KeepAwake,
            SwitchId::AlertSounds,
            SwitchId::DarkMode,
            SwitchId::NightLight,
            SwitchId::ScreenSaver,
            SwitchId::Microphone,
            SwitchId::LockKeyboard,
            SwitchId::Bluetooth,
        ];
        let shown: Vec<SwitchId> = vec![
            SwitchId::KeepAwake,
            SwitchId::Microphone,
            SwitchId::DarkMode,
            SwitchId::LockKeyboard,
        ];
        let more: Vec<SwitchId> = all
            .iter()
            .filter(|id| !shown.contains(id))
            .copied()
            .collect();
        Self {
            shown,
            more,
            hidden: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Apps {
    /// Executable file names, matched case-insensitively (FR-43).
    pub stays_on_screen: Vec<String>,
    pub hidden: Vec<String>,
}

impl Default for Apps {
    fn default() -> Self {
        Self {
            stays_on_screen: vec![
                "obs64.exe".into(),
                "SnippingTool.exe".into(),
                "ShareX.exe".into(),
                "Loom.exe".into(),
                "Zoom.exe".into(),
                "ms-teams.exe".into(),
            ],
            hidden: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrayIcons {
    /// Full executable paths kept visible besides button itself (FR-30).
    pub visible: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackdropStyle {
    Color,
    Gradient,
    Picture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackdropMonitors {
    All,
    Front,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Backdrop {
    pub style: BackdropStyle,
    /// "#rrggbb" (validated).
    pub color: String,
    /// Shape is decided in task 5.6 (FR-46); stored as raw JSON until then.
    pub gradient: Option<serde_json::Value>,
    pub picture: Option<String>,
    pub blur: u8,
    pub dim: u8,
    pub monitors: BackdropMonitors,
}

impl Default for Backdrop {
    fn default() -> Self {
        Self {
            style: BackdropStyle::Color,
            color: "#202020".into(),
            gradient: None,
            picture: None,
            blur: 0,
            dim: 0,
            monitors: BackdropMonitors::All,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeepAwake {
    pub default_minutes: u32,
    pub allow_display_off: bool,
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self {
            default_minutes: 120,
            allow_display_off: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Bluetooth {
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct General {
    pub start_with_windows: bool,
    pub check_for_updates: bool,
    pub onboarding_done: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            check_for_updates: true,
            onboarding_done: false,
        }
    }
}

/// What loading produced; the app layer decides notifications (spec 5.1, 7.12).
#[derive(Debug)]
pub enum SettingsRead {
    /// No file yet: defaults, nothing written.
    Fresh(Settings),
    /// Loaded (and, for an older schema, migrated and saved).
    Loaded(Settings),
    /// The file was invalid or failed validation: quarantined, defaults
    /// in memory. The app shows a notification.
    Invalid(Settings),
    /// schemaVersion is newer than this build knows: use read-only in
    /// memory and warn; never save over it.
    Newer(Settings),
}

pub fn settings_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("settings.json")
}

/// Load with defaults, validation, backup + migrate (spec 5.1, 7.12).
pub fn load(data_dir: &Path, now: OffsetDateTime) -> Result<SettingsRead, std::io::Error> {
    let path = settings_path(data_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SettingsRead::Fresh(Settings::default()));
        }
        // read_to_string reports invalid UTF-8 as InvalidData: that is a
        // corrupt file, not an error to surface (quarantine + defaults).
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            quarantine_invalid(data_dir, now)?;
            return Ok(SettingsRead::Invalid(Settings::default()));
        }
        Err(e) => return Err(e),
    };

    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => {
            quarantine_invalid(data_dir, now)?;
            return Ok(SettingsRead::Invalid(Settings::default()));
        }
    };

    let version = value["schemaVersion"]
        .as_u64()
        .unwrap_or(SCHEMA_VERSION as u64);
    if version > SCHEMA_VERSION as u64 {
        // Downgraded build: read-only, never overwrite (spec 7.12).
        let settings: Settings = serde_json::from_value(value).unwrap_or_default();
        return Ok(SettingsRead::Newer(settings));
    }

    if version < SCHEMA_VERSION as u64 {
        // Back up the old file, then migrate in order (spec 7.12).
        let backup = data_dir.join(format!("settings.v{version}.bak.json"));
        std::fs::write(backup, &text)?;
    }

    // A valid-JSON file whose types don't match (e.g. "hotkeys": 5) is
    // corrupt for this build: quarantine it instead of silently starting
    // from defaults while leaving the bad file in place.
    let settings: Settings = match serde_json::from_value(value) {
        Ok(s) => s,
        Err(_) => {
            quarantine_invalid(data_dir, now)?;
            return Ok(SettingsRead::Invalid(Settings::default()));
        }
    };
    let mut settings = settings;
    settings.schema_version = SCHEMA_VERSION;
    if validate(&settings) {
        if version < SCHEMA_VERSION as u64 {
            save(&settings, data_dir)?;
        }
        Ok(SettingsRead::Loaded(settings))
    } else {
        quarantine_invalid(data_dir, now)?;
        Ok(SettingsRead::Invalid(Settings::default()))
    }
}

/// Save atomically (same protocol as the journal, spec 7.4).
pub fn save(settings: &Settings, data_dir: &Path) -> Result<(), std::io::Error> {
    let text = serde_json::to_string_pretty(settings).expect("settings serialize");
    write_atomic(&settings_path(data_dir), text.as_bytes())
}

fn quarantine_invalid(data_dir: &Path, now: OffsetDateTime) -> std::io::Result<()> {
    let stamp = now.unix_timestamp();
    let from = settings_path(data_dir);
    if from.exists() {
        let to = data_dir.join(format!("settings.invalid-{stamp}.json"));
        std::fs::rename(from, to)?;
    }
    Ok(())
}

/// Validation (spec 9): switch ids come from a closed enum so they are valid
/// by construction; the lists must not repeat an id; colours must be #rrggbb;
/// app exe names lowercase. Accelerators are validated where they are used:
/// registration fails and the UI warns (spec 7.11), so no second parser here.
fn validate(settings: &Settings) -> bool {
    let mut seen = Vec::new();
    for list in [
        &settings.dropdown.shown,
        &settings.dropdown.more,
        &settings.dropdown.hidden,
    ] {
        for id in list {
            if seen.contains(id) {
                return false;
            }
            seen.push(*id);
        }
    }
    if !is_valid_hex_colour(&settings.backdrop.color) {
        return false;
    }
    // App exe names are case-insensitive (spec 5.1): any casing is valid.
    // Matching normalises with to_lowercase; save_settings normalises on
    // write (Phase 5).
    true
}

/// "#rrggbb" exactly.
pub fn is_valid_hex_colour(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use time::macros::datetime;

    fn now() -> OffsetDateTime {
        datetime!(2026-10-26 12:00:00 UTC)
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("button-settings-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_gives_fresh_defaults() {
        let dir = test_dir("missing");
        match load(&dir, now()).unwrap() {
            SettingsRead::Fresh(s) => {
                assert_eq!(s.schema_version, SCHEMA_VERSION);
                assert_eq!(s.hotkeys.recording_mode, "Alt+R");
                assert_eq!(s.recording_mode.actions.len(), 8);
                assert_eq!(s.dropdown.shown.len(), 4);
                assert_eq!(s.dropdown.more.len(), 10);
            }
            other => panic!("expected Fresh, got {other:?}"),
        }
        // Nothing is written until the user changes something.
        assert!(!settings_path(&dir).exists());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = test_dir("roundtrip");
        let settings = Settings::default();
        save(&settings, &dir).unwrap();
        match load(&dir, now()).unwrap() {
            SettingsRead::Loaded(s) => assert_eq!(s, settings),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let dir = test_dir("unknown");
        fs::write(
            settings_path(&dir),
            r#"{"schemaVersion":1,"futureField":true,"hotkeys":{"recordingMode":"Ctrl+R"}}"#,
        )
        .unwrap();
        match load(&dir, now()).unwrap() {
            SettingsRead::Loaded(s) => assert_eq!(s.hotkeys.recording_mode, "Ctrl+R"),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn corrupt_file_is_quarantined_and_defaults_are_used() {
        let dir = test_dir("corrupt");
        fs::write(settings_path(&dir), "{ not json").unwrap();
        match load(&dir, now()).unwrap() {
            SettingsRead::Invalid(_) => {}
            other => panic!("expected Invalid, got {other:?}"),
        }
        assert!(!settings_path(&dir).exists());
        let stamp = now().unix_timestamp();
        assert!(dir.join(format!("settings.invalid-{stamp}.json")).exists());
    }

    #[test]
    fn invalid_values_are_quarantined_like_corrupt_files() {
        let dir = test_dir("invalid-colour");
        fs::write(
            settings_path(&dir),
            r#"{"schemaVersion":1,"backdrop":{"color":"red"}}"#,
        )
        .unwrap();
        match load(&dir, now()).unwrap() {
            SettingsRead::Invalid(_) => {}
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn uppercase_exe_names_are_valid_and_defaults_load() {
        // Exe names are case-insensitive (spec 5.1): a user-typed
        // "Zoom.exe" must not quarantine the file, and the shipped
        // defaults (which contain capitals) must validate.
        let dir = test_dir("uppercase-exe");
        fs::write(
            settings_path(&dir),
            r#"{"schemaVersion":1,"apps":{"staysOnScreen":["Zoom.exe","OBS64.EXE"]}}"#,
        )
        .unwrap();
        match load(&dir, now()).unwrap() {
            SettingsRead::Loaded(s) => assert_eq!(
                s.apps.stays_on_screen,
                vec!["Zoom.exe".to_string(), "OBS64.EXE".to_string()]
            ),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn older_schema_is_backed_up_migrated_and_saved() {
        let dir = test_dir("migrate");
        fs::write(
            settings_path(&dir),
            r#"{"schemaVersion":0,"hotkeys":{"recordingMode":"Alt+R"}}"#,
        )
        .unwrap();
        match load(&dir, now()).unwrap() {
            SettingsRead::Loaded(s) => {
                assert_eq!(s.schema_version, SCHEMA_VERSION);
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
        assert!(dir.join("settings.v0.bak.json").exists());
        // The migrated file is saved with the current version.
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(settings_path(&dir)).unwrap()).unwrap();
        assert_eq!(saved["schemaVersion"], 1);
    }

    #[test]
    fn newer_schema_is_read_only_and_never_overwritten() {
        let dir = test_dir("newer");
        let original = r#"{"schemaVersion":2,"hotkeys":{"recordingMode":"F9"}}"#;
        fs::write(settings_path(&dir), original).unwrap();
        match load(&dir, now()).unwrap() {
            SettingsRead::Newer(s) => {
                // Known fields still read; the file is untouched.
                assert_eq!(s.hotkeys.recording_mode, "F9");
            }
            other => panic!("expected Newer, got {other:?}"),
        }
        assert_eq!(fs::read_to_string(settings_path(&dir)).unwrap(), original);
    }
}
