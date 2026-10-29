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
    /// The current Windows wallpaper (D-087), read at apply time.
    Wallpaper,
}

/// Gradient preset ids (D-087): `backdrop.gradient` stores one of these,
/// rendered by the matching CSS class in backdrop.css (FR-46 "preset
/// gradients").
pub const GRADIENT_PRESETS: [&str; 6] = ["dusk", "ocean", "ember", "forest", "violet", "slate"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackdropMonitors {
    All,
    Front,
}

fn deserialize_optional_gradient<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt {
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        _ => Ok(None),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Backdrop {
    pub style: BackdropStyle,
    /// "#rrggbb" (validated).
    pub color: String,
    /// A preset id from GRADIENT_PRESETS when style = gradient (D-087).
    /// Accepts older schema-v1 raw JSON values during load to avoid quarantining.
    #[serde(deserialize_with = "deserialize_optional_gradient", default)]
    pub gradient: Option<String>,
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

/// The switch ids Recording Mode may run, in spec 7.1 apply order.
const ACTION_IDS: [SwitchId; 10] = [
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

/// Is `name` a bare executable file name like "Zoom.exe" (spec 5.1)?
fn is_exe_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.is_empty()
        && lower.ends_with(".exe")
        && name.len() > 4
        && !name
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

fn is_absolute_path(path: &str) -> bool {
    // Windows absolute: "C:\..." or a UNC path. No networked expansion here.
    let bytes = path.as_bytes();
    (bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/'))
        || path.starts_with("\\\\")
}

fn dedupe_ci(list: &mut Vec<String>) {
    let mut seen: Vec<String> = Vec::new();
    list.retain(|value| {
        let lower = value.to_ascii_lowercase();
        if seen.contains(&lower) {
            false
        } else {
            seen.push(lower);
            true
        }
    });
}

/// Save-time validation + normalisation for the Phase 5 writer (spec 6.1).
/// Field-specific messages; the caller maps them to INVALID_ARGUMENT.
/// Normalises in place: dedupes, drops empty hotkeys, fills dropdown.
pub fn normalize_saving(settings: &mut Settings) -> Result<(), String> {
    // Recording Mode actions: subset of the §7.1 ids, no duplicates.
    let mut actions: Vec<SwitchId> = Vec::new();
    for id in &settings.recording_mode.actions {
        if !ACTION_IDS.contains(id) {
            return Err(format!("'{id}' is not a Recording Mode action"));
        }
        if actions.contains(id) {
            return Err(format!("'{id}' is listed twice"));
        }
        actions.push(*id);
    }
    settings.recording_mode.actions = actions;

    match settings.recording_mode.time_limit_minutes {
        None | Some(1..=480) => {}
        Some(other) => {
            return Err(format!("time limit must be 1-480 minutes, not {other}"));
        }
    }

    // Empty bindings are removed (the editor's Clear), then validated.
    settings
        .hotkeys
        .switches
        .retain(|_, binding| !binding.is_empty());
    validate_hotkeys(&settings.hotkeys)?;

    // Dropdown: every switch exactly once; missing ones land in "more".
    let all: [SwitchId; 14] = [
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
    let mut placed: Vec<SwitchId> = Vec::new();
    for list in [
        &mut settings.dropdown.shown,
        &mut settings.dropdown.more,
        &mut settings.dropdown.hidden,
    ] {
        list.retain(|id| {
            if placed.contains(id) {
                return false; // duplicate across shelves: keep the first
            }
            if !all.contains(id) {
                return false; // cannot happen through the enum, belt only
            }
            placed.push(*id);
            true
        });
    }
    for id in all.iter().filter(|id| !placed.contains(id)) {
        settings.dropdown.more.push(*id);
    }

    // Apps: bare exe names, deduped case-insensitively, never on both lists.
    for name in settings
        .apps
        .stays_on_screen
        .iter()
        .chain(&settings.apps.hidden)
    {
        if !is_exe_name(name) {
            return Err(format!(
                "'{name}' is not an executable file name like Zoom.exe"
            ));
        }
    }
    let lower_stays: Vec<String> = settings
        .apps
        .stays_on_screen
        .iter()
        .map(|n| n.to_ascii_lowercase())
        .collect();
    for name in &settings.apps.hidden {
        if lower_stays.contains(&name.to_ascii_lowercase()) {
            return Err(format!(
                "'{name}' is on both shelves; an app is either on Stays on Screen or Hidden"
            ));
        }
    }
    dedupe_ci(&mut settings.apps.stays_on_screen);
    dedupe_ci(&mut settings.apps.hidden);

    for path in &settings.tray_icons.visible {
        if !is_absolute_path(path) {
            return Err(format!("'{path}' is not an absolute path"));
        }
    }
    dedupe_ci(&mut settings.tray_icons.visible);

    validate_backdrop(&settings.backdrop)?;

    if !(1..=1440).contains(&settings.keep_awake.default_minutes) {
        return Err(format!(
            "Keep Awake default must be 1-1440 minutes, not {}",
            settings.keep_awake.default_minutes
        ));
    }
    if settings
        .bluetooth
        .device_id
        .as_ref()
        .is_some_and(|id| id.is_empty())
    {
        return Err("Bluetooth device id cannot be empty".into());
    }
    Ok(())
}

/// Every binding parses as an accelerator, has Ctrl/Alt/Win (Shift-only
/// would hijack typing), is not Escape (the fixed Lock Keyboard unlock
/// combo), and no two share a parsed id (spec 7.11). Empty = removed.
fn validate_hotkeys(hotkeys: &Hotkeys) -> Result<(), String> {
    let mut bindings: Vec<(String, global_hotkey::hotkey::HotKey)> = Vec::new();
    let mut check = |label: &str, binding: &str| -> Result<(), String> {
        if binding.is_empty() {
            return Ok(()); // cleared binding
        }
        let Ok(shortcut) = binding.parse::<global_hotkey::hotkey::HotKey>() else {
            return Err(format!("'{binding}' is not a valid {label} hotkey"));
        };
        use global_hotkey::hotkey::{Code, Modifiers};
        let wanted = Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER;
        if !shortcut.mods.intersects(wanted) {
            return Err(format!(
                "'{binding}' needs Ctrl, Alt or Win so it cannot hijack plain typing"
            ));
        }
        if shortcut.key == Code::Escape {
            return Err("Escape is reserved: it unlocks the keyboard".into());
        }
        if let Some((_, first)) = bindings.iter().find(|(_, s)| s.id == shortcut.id) {
            return Err(format!("'{binding}' is already used by '{first}'"));
        }
        bindings.push((binding.to_string(), shortcut));
        Ok(())
    };
    // The recording-mode toggle must always have a binding (FR-9: the
    // system-wide toggle is core; an empty binding would silently disable
    // it). Only switch hotkeys may be cleared.
    if hotkeys.recording_mode.is_empty() {
        return Err("the Recording Mode hotkey cannot be empty".into());
    }
    check("recording mode", &hotkeys.recording_mode)?;
    for (id, binding) in &hotkeys.switches {
        if crate::automation::parse_switch_id(id).is_err() {
            return Err(format!("'{id}' is not a switch"));
        }
        check(&format!("'{id}' hotkey"), binding)?;
    }
    Ok(())
}

fn validate_backdrop(backdrop: &Backdrop) -> Result<(), String> {
    if !is_valid_hex_colour(&backdrop.color) {
        return Err(format!("'{}' is not a #rrggbb colour", backdrop.color));
    }
    match backdrop.style {
        BackdropStyle::Gradient => {
            let Some(id) = &backdrop.gradient else {
                return Err("a gradient style needs a gradient preset".into());
            };
            if !GRADIENT_PRESETS.contains(&id.as_str()) {
                return Err(format!("'{id}' is not a gradient preset"));
            }
        }
        BackdropStyle::Picture if backdrop.picture.as_ref().is_none_or(String::is_empty) => {
            return Err("a picture style needs a picture path".into());
        }
        _ => {}
    }
    if backdrop.blur > 40 {
        return Err(format!("blur must be 0-40 px, not {}", backdrop.blur));
    }
    if backdrop.dim > 80 {
        return Err(format!("dim must be 0-80 %, not {}", backdrop.dim));
    }
    Ok(())
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
    fn type_mismatch_is_quarantined_like_corrupt_files() {
        let dir = test_dir("type-mismatch");
        fs::write(settings_path(&dir), r#"{"schemaVersion":1,"hotkeys":5}"#).unwrap();
        match load(&dir, now()).unwrap() {
            SettingsRead::Invalid(_) => {}
            other => panic!("expected Invalid, got {other:?}"),
        }
        // Quarantined out of the way, defaults take over.
        assert!(!settings_path(&dir).exists());
        let stamp = now().unix_timestamp();
        assert!(dir.join(format!("settings.invalid-{stamp}.json")).exists());
    }

    #[test]
    fn invalid_utf8_is_quarantined_like_corrupt_files() {
        let dir = test_dir("invalid-utf8");
        fs::write(settings_path(&dir), b"{\"schemaVersion\":1,\xff\xfe}").unwrap();
        match load(&dir, now()).unwrap() {
            SettingsRead::Invalid(_) => {}
            other => panic!("expected Invalid, got {other:?}"),
        }
        assert!(!settings_path(&dir).exists());
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

    // ---- save-time validation + normalisation (Phase 5 writer) ----

    fn normalize_ok(mut settings: Settings) -> Settings {
        normalize_saving(&mut settings).expect("settings should be valid");
        settings
    }

    fn normalize_err(mut settings: Settings) -> String {
        normalize_saving(&mut settings).expect_err("settings should be rejected")
    }

    #[test]
    fn defaults_pass_the_save_time_validation() {
        normalize_ok(Settings::default());
    }

    #[test]
    fn actions_must_be_a_unique_subset_of_the_71_ids() {
        let mut settings = Settings::default();
        settings.recording_mode.actions.push(SwitchId::ScreenSaver);
        assert!(normalize_err(settings.clone()).contains("not a Recording Mode action"));
        // Drop the invalid id, then duplicate a valid one.
        settings.recording_mode.actions.pop();
        settings.recording_mode.actions.push(SwitchId::Dnd);
        assert!(normalize_err(settings).contains("listed twice"));
    }

    #[test]
    fn time_limit_bounds() {
        let mut settings = Settings::default();
        settings.recording_mode.time_limit_minutes = Some(0);
        assert!(normalize_err(settings.clone()).contains("1-480"));
        settings.recording_mode.time_limit_minutes = Some(480);
        normalize_ok(settings.clone());
        settings.recording_mode.time_limit_minutes = Some(481);
        assert!(normalize_err(settings).contains("1-480"));
    }

    #[test]
    fn hotkeys_need_ctrl_alt_or_win() {
        let mut settings = Settings::default();
        settings.hotkeys.recording_mode = "Shift+K".into();
        assert!(normalize_err(settings.clone()).contains("needs Ctrl, Alt or Win"));
        settings.hotkeys.recording_mode = "Ctrl+K".into();
        normalize_ok(settings);
    }

    #[test]
    fn a_hotkey_clash_is_detected_by_the_parsed_id_not_the_string() {
        let mut settings = Settings::default();
        // Same combo spelled differently: both parse to the same id, so
        // a string comparison would pass this test.
        settings.hotkeys.recording_mode = "Ctrl+K".into();
        settings
            .hotkeys
            .switches
            .insert("keep-awake".into(), "CONTROL+K".into());
        let message = normalize_err(settings);
        assert!(message.contains("already used by"), "message: {message}");
    }

    #[test]
    fn escape_is_reserved_for_unlocking_the_keyboard() {
        let mut settings = Settings::default();
        settings.hotkeys.recording_mode = "Ctrl+Escape".into();
        assert!(normalize_err(settings).contains("Escape is reserved"));
    }

    #[test]
    fn gradient_accepts_raw_json_value_from_older_schema_v1() {
        let json =
            r##"{"style": "color", "color": "#202020", "gradient": {"stops": ["#000", "#fff"]}}"##;
        let backdrop: Backdrop = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(backdrop.gradient, None);

        let json_preset =
            r##"{"style": "gradient", "color": "#202020", "gradient": "gradient-ember"}"##;
        let backdrop_preset: Backdrop =
            serde_json::from_str(json_preset).expect("should deserialize");
        assert_eq!(backdrop_preset.gradient, Some("gradient-ember".into()));
    }

    #[test]
    fn empty_switch_hotkeys_are_removed_and_unknown_ids_rejected() {
        let mut settings = Settings::default();
        settings
            .hotkeys
            .switches
            .insert("keep-awake".into(), String::new());
        let normalized = normalize_ok(settings);
        assert!(normalized.hotkeys.switches.is_empty());

        let mut settings = Settings::default();
        settings
            .hotkeys
            .switches
            .insert("not-a-switch".into(), "Ctrl+K".into());
        assert!(normalize_err(settings).contains("is not a switch"));
    }

    #[test]
    fn an_unavailable_binding_parses_but_is_rejected_nowhere() {
        // A syntactically valid binding that Windows would refuse at
        // registration is accepted by validation; registration failure is
        // a runtime error with a message (spec 7.11). The parser spells
        // the Win key "Super".
        let mut settings = Settings::default();
        settings.hotkeys.recording_mode = "Ctrl+Alt+Shift+Super+K".into();
        normalize_ok(settings);
    }

    #[test]
    fn dropdown_fills_missing_switches_into_more_and_dedupes() {
        let mut settings = Settings::default();
        settings.dropdown.shown.push(SwitchId::KeepAwake); // duplicate
        settings.dropdown.more.clear();
        let normalized = normalize_ok(settings);
        assert_eq!(
            normalized
                .dropdown
                .shown
                .iter()
                .filter(|id| **id == SwitchId::KeepAwake)
                .count(),
            1
        );
        // Every switch appears exactly once across the three shelves.
        let mut placed: Vec<String> = normalized
            .dropdown
            .shown
            .iter()
            .chain(&normalized.dropdown.more)
            .chain(&normalized.dropdown.hidden)
            .map(|id| id.to_string())
            .collect();
        placed.sort();
        placed.dedup();
        assert_eq!(placed.len(), 14);
    }

    #[test]
    fn app_names_must_be_bare_exe_names() {
        let mut settings = Settings::default();
        settings.apps.stays_on_screen = vec!["C:\\evil\\x.exe".into()];
        assert!(normalize_err(settings.clone()).contains("executable file name"));
        settings.apps.stays_on_screen = vec!["Zoom".into()];
        assert!(normalize_err(settings.clone()).contains("executable file name"));
        settings.apps.stays_on_screen = vec!["Zoom.exe".into()];
        normalize_ok(settings);
    }

    #[test]
    fn an_app_is_never_on_both_shelves() {
        let mut settings = Settings::default();
        settings.apps.stays_on_screen.push("Zoom.exe".into());
        settings.apps.hidden.push("zoom.exe".into());
        assert!(normalize_err(settings).contains("both shelves"));
    }

    #[test]
    fn app_lists_dedupe_case_insensitively() {
        let mut settings = Settings::default();
        settings.apps.stays_on_screen = vec!["Zoom.exe".into(), "ZOOM.EXE".into()];
        let normalized = normalize_ok(settings);
        assert_eq!(
            normalized.apps.stays_on_screen,
            vec!["Zoom.exe".to_string()]
        );
    }

    #[test]
    fn tray_icon_paths_must_be_absolute_and_dedupe() {
        let mut settings = Settings::default();
        settings
            .tray_icons
            .visible
            .push("relative\\icon.exe".into());
        assert!(normalize_err(settings.clone()).contains("absolute path"));
        settings.tray_icons.visible = vec![
            "C:\\Program Files\\x\\x.exe".into(),
            "c:\\program files\\x\\x.exe".into(),
        ];
        let normalized = normalize_ok(settings);
        assert_eq!(normalized.tray_icons.visible.len(), 1);
    }

    #[test]
    fn gradient_must_be_a_preset_id_when_style_is_gradient() {
        let mut settings = Settings::default();
        settings.backdrop.style = BackdropStyle::Gradient;
        settings.backdrop.gradient = Some("nope".into());
        assert!(normalize_err(settings.clone()).contains("not a gradient preset"));
        settings.backdrop.gradient = Some("ocean".into());
        normalize_ok(settings.clone());
        // Other styles do not read the gradient field.
        settings.backdrop.style = BackdropStyle::Color;
        settings.backdrop.gradient = None;
        normalize_ok(settings);
    }

    #[test]
    fn blur_and_dim_have_bounds() {
        let mut settings = Settings::default();
        settings.backdrop.blur = 41;
        assert!(normalize_err(settings.clone()).contains("0-40"));
        settings.backdrop.blur = 40;
        settings.backdrop.dim = 81;
        assert!(normalize_err(settings).contains("0-80"));
    }

    #[test]
    fn keep_awake_default_minutes_are_bounded() {
        let mut settings = Settings::default();
        settings.keep_awake.default_minutes = 0;
        assert!(normalize_err(settings.clone()).contains("1-1440"));
        settings.keep_awake.default_minutes = 1441;
        assert!(normalize_err(settings).contains("1-1440"));
    }

    #[test]
    fn a_full_non_default_settings_roundtrip_survives_the_writer() {
        // 5.9 evidence: every field set to a non-default value, normalised,
        // saved and loaded back unchanged.
        let mut settings = Settings::default();
        settings.recording_mode.actions = vec![
            SwitchId::Dnd,
            SwitchId::KeepAwake,
            SwitchId::OtherApps,
            SwitchId::Backdrop,
            SwitchId::Taskbar,
        ];
        settings.recording_mode.countdown = true;
        settings.recording_mode.time_limit_minutes = Some(90);
        settings.hotkeys.recording_mode = "Ctrl+Alt+F9".into();
        settings
            .hotkeys
            .switches
            .insert("keep-awake".into(), "Ctrl+Alt+F10".into());
        settings.dropdown.shown = vec![SwitchId::Microphone, SwitchId::DarkMode];
        settings.dropdown.more = vec![
            SwitchId::Dnd,
            SwitchId::DesktopIcons,
            SwitchId::OtherApps,
            SwitchId::Taskbar,
            SwitchId::TrayIcons,
            SwitchId::Backdrop,
            SwitchId::KeepAwake,
            SwitchId::AlertSounds,
            SwitchId::ScreenSaver,
            SwitchId::LockKeyboard,
            SwitchId::Bluetooth,
        ];
        settings.dropdown.hidden = vec![SwitchId::NightLight];
        settings.apps.stays_on_screen = vec!["Obs64.exe".into(), "Zoom.exe".into()];
        settings.apps.hidden = vec!["Spotify.exe".into()];
        settings.tray_icons.visible = vec!["C:\\Tools\\mytray.exe".into()];
        settings.backdrop.style = BackdropStyle::Wallpaper;
        settings.backdrop.color = "#101010".into();
        settings.backdrop.blur = 12;
        settings.backdrop.dim = 30;
        settings.backdrop.monitors = BackdropMonitors::Front;
        settings.keep_awake.default_minutes = 45;
        settings.keep_awake.allow_display_off = true;
        settings.bluetooth.device_id = Some("{0.0.0.00000000}.{a}".into());
        settings.general.start_with_windows = true;
        settings.general.check_for_updates = false;
        settings.general.onboarding_done = true;
        let normalized = normalize_ok(settings);

        let dir = test_dir("full-roundtrip");
        save(&normalized, &dir).unwrap();
        match load(&dir, now()).unwrap() {
            SettingsRead::Loaded(loaded) => assert_eq!(loaded, normalized),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }
}
