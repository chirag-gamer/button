//! Hide Tray Icons (FR-30, spec 7.5 catalogue): Win11 only. Each tray
//! icon's promotion state lives in
//! `HKCU\Control Panel\NotifyIconSettings\<key>\IsPromoted` (1 = on the
//! taskbar, 0 or absent = in the overflow). Apply demotes everything not
//! on the Visible shelf, keeps button's own icon promoted ([D-072]) and
//! journals the per-key prior values so restore puts every icon back
//! exactly ([D-064]; the mechanism is undocumented, probed read-only on
//! the owner's PC).

use serde::{Deserialize, Serialize};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows_registry::{CURRENT_USER, Key};

use super::{
    Availability, Ctx, RestoreOutcome, Switch, SwitchError, SwitchId, parse_state, same_exe,
};
use crate::journal::JournalEntry;
use crate::os::OsInfo;

const ROOT: &str = r"Control Panel\NotifyIconSettings";

/// State shape: `{"icons": [{"key", "isPromoted": 0|1|null}]}` (null =
/// value absent, icon in overflow) — the per-key prior values restore
/// re-applies (spec 7.5).
#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub icons: Vec<IconEntry>,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IconEntry {
    pub key: String,
    pub is_promoted: Option<u32>,
    /// This entry is button's own tray icon (matched by exe basename).
    /// Additive with `#[serde(default)]`: journals written before this
    /// field parse with `false`, which only affects `is_on` on stale
    /// entries — restore never reads it.
    #[serde(default)]
    pub is_own: bool,
}

pub struct TrayIcons;

impl Switch for TrayIcons {
    fn id(&self) -> SwitchId {
        SwitchId::TrayIcons
    }

    fn availability(&self, os: &OsInfo) -> Availability {
        if os.is_windows_11 {
            Availability::Available
        } else {
            // FR-29: greyed out with "Needs Windows 11", skipped by
            // Recording Mode ([D-042]).
            Availability::NeedsWindows11
        }
    }

    fn persistent(&self) -> bool {
        true // a saved Explorer setting, like the taskbar auto-hide
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        Ok(serde_json::json!(State {
            icons: enumerate_states()?,
        }))
    }
    /// Re-enumerates (the journaled state carries no exe paths) and
    /// decides per key: button's own key stays promoted, Visible-shelf
    /// exes keep their prior value, everything else is demoted. On an
    /// enumeration error the target stays equal to `before`, so the
    /// session journals no change and apply touches nothing (the error is
    /// logged — G-4).
    fn target_on(&self, before: &serde_json::Value, ctx: &Ctx) -> serde_json::Value {
        let before =
            parse_state::<State>(before, "tray icons").unwrap_or(State { icons: Vec::new() });
        let icons = match snapshot() {
            Ok(keys) => keys
                .into_iter()
                .map(|(key, raw_exe)| {
                    let prior = before
                        .icons
                        .iter()
                        .find(|e| e.key == key)
                        .and_then(|e| e.is_promoted);
                    let expanded = raw_exe.as_deref().and_then(expand_known_folder);
                    let is_own = exe_matches_own(expanded.as_deref(), &ctx.own_exe);
                    IconEntry {
                        key,
                        is_promoted: Some(decide(
                            is_own,
                            expanded.as_deref(),
                            prior,
                            &ctx.tray_visible,
                        )),
                        is_own,
                    }
                })
                .collect(),
            Err(e) => {
                log::warn!("tray-icons: enumeration failed, leaving icons untouched: {e}");
                before.icons
            }
        };
        serde_json::json!(State { icons })
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "tray icons")?;
        // Values we already wrote, so a later failure can put them back:
        // the engine drops the whole entry on an apply error, and an
        // unjournaled half-applied switch would strand those icons.
        let mut written: Vec<(String, Option<u32>)> = Vec::new();
        for entry in &target.icons {
            let Some(key) = open_key(&entry.key) else {
                log::info!("tray-icons: key {} vanished since target_on", entry.key);
                continue;
            };
            let prior = key.get_u32("IsPromoted").ok();
            let result = match entry.is_promoted {
                Some(v) => key.set_u32("IsPromoted", v).map_err(|e| SwitchError::Os {
                    api: "RegSetValueEx",
                    code: e.code().0,
                }),
                None => {
                    // Absent is the desired state; removing an absent
                    // value is not an error.
                    if let Err(e) = key.remove_value("IsPromoted") {
                        log::info!("tray-icons: IsPromoted already absent: {e}");
                    }
                    Ok(())
                }
            };
            match result {
                Ok(()) => written.push((entry.key.clone(), prior)),
                Err(e) => {
                    for (key_name, prior) in written.iter().rev() {
                        rollback_key(key_name, *prior);
                    }
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Per key (spec 7.3): gone keys are skipped, a value the user changed
    /// away from ours is left alone (user-reverted), everything else gets
    /// its prior value back.
    fn restore(&self, e: &JournalEntry) -> Result<RestoreOutcome, SwitchError> {
        let before = parse_state::<State>(&e.before, "tray icons")?;
        let after =
            parse_state::<State>(&e.after, "tray icons").unwrap_or(State { icons: Vec::new() });
        let mut restored = 0;
        let mut user_changed = 0;
        for entry in &before.icons {
            let Some(key) = open_key(&entry.key) else {
                continue; // key gone (app uninstalled): nothing to restore
            };
            let current = key.get_u32("IsPromoted").ok();
            let our_value = after
                .icons
                .iter()
                .find(|a| a.key == entry.key)
                .and_then(|a| a.is_promoted);
            if current != our_value {
                user_changed += 1;
                log::info!(
                    "tray-icons: key {} changed by the user; leaving it",
                    entry.key
                );
                continue;
            }
            match entry.is_promoted {
                Some(v) => key.set_u32("IsPromoted", v).map_err(|e| SwitchError::Os {
                    api: "RegSetValueEx",
                    code: e.code().0,
                })?,
                None => {
                    if let Err(e) = key.remove_value("IsPromoted") {
                        log::info!("tray-icons: IsPromoted already absent: {e}");
                    }
                }
            }
            restored += 1;
        }
        Ok(outcome(restored, user_changed))
    }

    /// "On" reads as "nothing but button's own icon is promoted onto the
    /// taskbar" — the own icon stays promoted by rule ([D-072]), so it
    /// never counts against the switch.
    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "tray icons")
            .map(|state| {
                !state
                    .icons
                    .iter()
                    .any(|e| !e.is_own && e.is_promoted == Some(1))
            })
            .unwrap_or(false)
    }
}

/// Button's own key stays promoted so the stop control stays reachable
/// ([D-072]); Visible-shelf exes keep their prior value; everything else
/// is demoted. `is_own` comes from matching the expanded `ExecutablePath`
/// by basename, because the registry stores full paths while
/// `ctx.own_exe` is a bare exe name. Visible entries hold full paths
/// (FR-30) and are compared as full paths.
fn decide(is_own: bool, exe: Option<&str>, prior: Option<u32>, visible: &[String]) -> u32 {
    let exe = exe.unwrap_or("");
    if is_own {
        1
    } else if visible.iter().any(|v| same_exe(v, exe)) {
        prior.unwrap_or(0)
    } else {
        0
    }
}

/// The exe name after the last path separator; quotes tolerated.
fn exe_name(path: &str) -> &str {
    let path = path.trim_matches('"');
    let start = path.rfind(['\\', '/']).map_or(0, |i| i + 1);
    &path[start..]
}

/// True when the expanded `ExecutablePath` is button's own exe, compared
/// by basename: the registry stores full paths while `ctx.own_exe` is a
/// bare exe name.
fn exe_matches_own(exe: Option<&str>, own_exe: &str) -> bool {
    exe.is_some_and(|e| same_exe(exe_name(e), own_exe))
}

fn outcome(restored: usize, user_changed: usize) -> RestoreOutcome {
    if restored > 0 {
        RestoreOutcome::Restored
    } else if user_changed > 0 {
        RestoreOutcome::UserChanged
    } else {
        RestoreOutcome::Gone
    }
}

fn open_key(name: &str) -> Option<Key> {
    CURRENT_USER
        .options()
        .read()
        .write()
        .open(format!(r"{ROOT}\{name}"))
        .ok()
}

/// (key name, raw `ExecutablePath`) for every tray-icon entry.
fn snapshot() -> Result<Vec<(String, Option<String>)>, SwitchError> {
    let root = CURRENT_USER.open(ROOT).map_err(|e| SwitchError::Os {
        api: "RegOpenKeyEx",
        code: e.code().0,
    })?;
    let mut out = Vec::new();
    for key in root.keys().map_err(|e| SwitchError::Os {
        api: "RegEnumKeyEx",
        code: e.code().0,
    })? {
        let exe = root
            .open(&key)
            .ok()
            .and_then(|k| k.get_string("ExecutablePath").ok());
        out.push((key, exe));
    }
    Ok(out)
}

/// (key name, current `IsPromoted`; None = value absent) for `read()`,
/// plus `is_own` so `is_on` can exclude button's icon. `own_exe` is the
/// bare exe name of the running button.
fn enumerate_states() -> Result<Vec<IconEntry>, SwitchError> {
    let own_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let root = CURRENT_USER.open(ROOT).map_err(|e| SwitchError::Os {
        api: "RegOpenKeyEx",
        code: e.code().0,
    })?;
    let mut out = Vec::new();
    for key in root.keys().map_err(|e| SwitchError::Os {
        api: "RegEnumKeyEx",
        code: e.code().0,
    })? {
        let opened = root.open(&key).ok();
        let is_promoted = opened.as_ref().and_then(|k| k.get_u32("IsPromoted").ok());
        let expanded = opened
            .as_ref()
            .and_then(|k| k.get_string("ExecutablePath").ok())
            .as_deref()
            .and_then(expand_known_folder);
        let is_own = exe_matches_own(expanded.as_deref(), &own_exe);
        out.push(IconEntry {
            key,
            is_promoted,
            is_own,
        });
    }
    Ok(out)
}

/// Best effort: put one key's `IsPromoted` back to its prior value
/// (None = remove) after a failed apply. Failures are logged, not
/// returned — the original apply error is what reaches the session.
fn rollback_key(key_name: &str, prior: Option<u32>) {
    let Some(key) = open_key(key_name) else {
        log::warn!("tray-icons: rollback: key {key_name} vanished");
        return;
    };
    let result = match prior {
        Some(v) => key.set_u32("IsPromoted", v),
        None => key.remove_value("IsPromoted"),
    };
    if let Err(e) = result {
        log::warn!("tray-icons: rollback of {key_name} failed: {e}");
    }
}

/// One row for the tray-icon Visible shelf (FR-30, list_tray_icons).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayIconInfo {
    /// NotifyIconSettings registry key (the switch's write handle).
    pub key: String,
    /// Expanded absolute executable path.
    pub exe: String,
    /// IsPromoted == 1 (shown on the taskbar, not in overflow).
    pub promoted: bool,
}

/// Every third-party tray icon with its current promoted state and exe
/// path, button's own icon excluded. Entries without a readable
/// ExecutablePath have no display name and are skipped.
pub(crate) fn list_entries() -> Result<Vec<TrayIconInfo>, SwitchError> {
    let own_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let root = CURRENT_USER.open(ROOT).map_err(|e| SwitchError::Os {
        api: "RegOpenKeyEx",
        code: e.code().0,
    })?;
    let mut out = Vec::new();
    for key in root.keys().map_err(|e| SwitchError::Os {
        api: "RegEnumKeyEx",
        code: e.code().0,
    })? {
        let Some(opened) = root.open(&key).ok() else {
            continue;
        };
        let is_promoted = opened.get_u32("IsPromoted").ok();
        let expanded = opened
            .get_string("ExecutablePath")
            .ok()
            .as_deref()
            .and_then(expand_known_folder);
        let Some(exe) = expanded else {
            continue;
        };
        if exe_matches_own(Some(&exe), &own_exe) {
            continue;
        }
        out.push(TrayIconInfo {
            key,
            exe,
            promoted: is_promoted == Some(1),
        });
    }
    Ok(out)
}

/// `"{GUID}\rest"` → known folder path + rest; plain paths unchanged
/// (probe ground truth: NotifyIconSettings stores both forms).
fn expand_known_folder(raw: &str) -> Option<String> {
    if !raw.starts_with('{') {
        return Some(raw.to_string());
    }
    let end = raw.find('}')?;
    let guid = windows::core::GUID::try_from(&raw[1..end]).ok()?;
    let base = unsafe { SHGetKnownFolderPath(&guid, KF_FLAG_DEFAULT, None) }.ok()?;
    let base_string = unsafe { base.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(base.0 as *const core::ffi::c_void)) };
    Some(format!("{}{}", base_string?, &raw[end + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(own: &str, visible: &[&str]) -> Ctx {
        Ctx {
            own_exe: own.to_string(),
            tray_visible: visible.iter().map(|s| s.to_string()).collect(),
            ..Ctx::default()
        }
    }

    #[test]
    fn state_shape_round_trips() {
        let value = serde_json::json!(State {
            icons: vec![
                IconEntry {
                    key: "abc".into(),
                    is_promoted: Some(1),
                    is_own: false,
                },
                IconEntry {
                    key: "def".into(),
                    is_promoted: None,
                    is_own: false,
                },
            ]
        });
        let state = parse_state::<State>(&value, "t").unwrap();
        assert_eq!(state.icons.len(), 2);
        assert_eq!(state.icons[1].is_promoted, None);
        assert!(parse_state::<State>(&serde_json::json!({}), "t").is_err());
    }

    /// Journals written before `isOwn` existed still parse (additive
    /// field, serde default false).
    #[test]
    fn state_shape_accepts_entries_without_is_own() {
        let value = serde_json::json!({
            "icons": [{ "key": "abc", "isPromoted": 1 }]
        });
        let state = parse_state::<State>(&value, "t").unwrap();
        assert_eq!(state.icons[0].is_promoted, Some(1));
        assert!(!state.icons[0].is_own);
    }

    #[test]
    fn is_on_when_nothing_but_the_own_icon_is_promoted() {
        let promoted = serde_json::json!(State {
            icons: vec![IconEntry {
                key: "k".into(),
                is_promoted: Some(1),
                is_own: false,
            }]
        });
        let own_promoted = serde_json::json!(State {
            icons: vec![
                IconEntry {
                    key: "own".into(),
                    is_promoted: Some(1),
                    is_own: true,
                },
                IconEntry {
                    key: "k".into(),
                    is_promoted: Some(0),
                    is_own: false,
                },
            ]
        });
        assert!(!TrayIcons.is_on(&promoted));
        assert!(TrayIcons.is_on(&own_promoted));
        assert!(!TrayIcons.is_on(&serde_json::json!({})));
    }

    #[test]
    fn decide_keeps_own_icon_promoted() {
        let c = ctx("button.exe", &[]);
        // The real shapes: ExecutablePath is a full path, ctx.own_exe is
        // the bare name (the 3.8 review fix).
        assert!(exe_matches_own(Some(r"C:\button\button.exe"), &c.own_exe));
        assert!(exe_matches_own(Some(r"C:\BUTTON\BUTTON.EXE"), &c.own_exe));
        assert!(!exe_matches_own(Some(r"C:\button\other.exe"), &c.own_exe));
        assert_eq!(
            decide(
                true,
                Some(r"C:\button\button.exe"),
                Some(0),
                &c.tray_visible
            ),
            1
        );
        assert_eq!(decide(true, None, None, &c.tray_visible), 1);
    }

    #[test]
    fn decide_keeps_visible_shelf_at_prior_value() {
        let c = ctx("button.exe", &[r"C:\Program Files\OBS\obs64.exe"]);
        assert_eq!(
            decide(
                false,
                Some(r"c:\program files\obs\OBS64.EXE"),
                Some(1),
                &c.tray_visible
            ),
            1
        );
        assert_eq!(
            decide(
                false,
                Some(r"C:\Program Files\OBS\obs64.exe"),
                None,
                &c.tray_visible
            ),
            0
        );
        // The shelf holds full paths (FR-30): a bare name or a same-basename
        // exe in another folder is demoted.
        let c2 = ctx("button.exe", &["obs64.exe"]);
        assert_eq!(
            decide(
                false,
                Some(r"C:\Program Files\OBS\obs64.exe"),
                Some(1),
                &c2.tray_visible
            ),
            0
        );
        let c3 = ctx("button.exe", &[r"C:\elsewhere\obs64.exe"]);
        assert_eq!(
            decide(
                false,
                Some(r"C:\Program Files\OBS\obs64.exe"),
                Some(1),
                &c3.tray_visible
            ),
            0
        );
    }

    #[test]
    fn decide_demotes_everything_else() {
        let c = ctx("button.exe", &[r"C:\keep\app.exe"]);
        assert_eq!(
            decide(false, Some(r"C:\other\app.exe"), Some(1), &c.tray_visible),
            0
        );
        assert_eq!(
            decide(false, None, None, &c.tray_visible),
            0,
            "unresolvable path"
        );
        assert_eq!(
            decide(false, Some(""), None, &c.tray_visible),
            0,
            "empty path"
        );
    }

    #[test]
    fn availability_needs_windows_11_on_win10() {
        let win10 = OsInfo {
            build: 19045,
            is_windows_11: false,
        };
        let win11 = OsInfo {
            build: 26200,
            is_windows_11: true,
        };
        assert_eq!(TrayIcons.availability(&win10), Availability::NeedsWindows11);
        assert_eq!(TrayIcons.availability(&win11), Availability::Available);
    }

    #[test]
    fn expand_leaves_plain_paths_alone() {
        assert_eq!(
            expand_known_folder(r"C:\Program Files\app.exe"),
            Some(r"C:\Program Files\app.exe".to_string())
        );
    }

    /// Read-only live probe: the Documents known folder expands to a real
    /// path ending in the given suffix.
    #[test]
    fn expand_resolves_known_folder_guids() {
        let expanded = expand_known_folder(r"{FDD39AD0-238F-46AF-ADB4-6C85480369C7}\Sub");
        assert!(expanded.is_some_and(|p| p.ends_with(r"\Sub")));
    }

    /// Read-only live probe of the registry enumeration; a machine (or CI
    /// runner) without tray-icon entries yields an empty state.
    #[test]
    fn read_reports_the_live_promotion_state() {
        match enumerate_states() {
            Ok(icons) => {
                assert!(icons.iter().all(|e| !e.key.is_empty()));
                assert!(
                    icons
                        .iter()
                        .all(|e| matches!(e.is_promoted, None | Some(0) | Some(1)))
                );
            }
            Err(SwitchError::Os { .. }) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
