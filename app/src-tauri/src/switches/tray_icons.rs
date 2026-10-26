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
                    IconEntry {
                        key,
                        is_promoted: Some(decide(
                            expanded.as_deref(),
                            prior,
                            &ctx.own_exe,
                            &ctx.tray_visible,
                        )),
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
        for entry in target.icons {
            let Some(key) = open_key(&entry.key) else {
                log::info!("tray-icons: key {} vanished since target_on", entry.key);
                continue;
            };
            match entry.is_promoted {
                Some(v) => key.set_u32("IsPromoted", v).map_err(|e| SwitchError::Os {
                    api: "RegSetValueEx",
                    code: e.code().0,
                })?,
                None => {
                    // Absent is the desired state; removing an absent
                    // value is not an error.
                    if let Err(e) = key.remove_value("IsPromoted") {
                        log::info!("tray-icons: IsPromoted already absent: {e}");
                    }
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

    /// "On" reads as "nothing is promoted onto the taskbar".
    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "tray icons")
            .map(|state| !state.icons.iter().any(|e| e.is_promoted == Some(1)))
            .unwrap_or(false)
    }
}

/// Button's own key stays promoted so the stop control stays reachable
/// ([D-072]); Visible-shelf exes keep their prior value; everything else
/// is demoted. `exe` is the expanded `ExecutablePath`.
fn decide(exe: Option<&str>, prior: Option<u32>, own_exe: &str, visible: &[String]) -> u32 {
    let exe = exe.unwrap_or("");
    if same_exe(exe, own_exe) {
        1
    } else if visible.iter().any(|v| same_exe(v, exe)) {
        prior.unwrap_or(0)
    } else {
        0
    }
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

/// (key name, current `IsPromoted`; None = value absent) for `read()`.
fn enumerate_states() -> Result<Vec<IconEntry>, SwitchError> {
    let root = CURRENT_USER.open(ROOT).map_err(|e| SwitchError::Os {
        api: "RegOpenKeyEx",
        code: e.code().0,
    })?;
    let mut out = Vec::new();
    for key in root.keys().map_err(|e| SwitchError::Os {
        api: "RegEnumKeyEx",
        code: e.code().0,
    })? {
        let is_promoted = root
            .open(&key)
            .ok()
            .and_then(|k| k.get_u32("IsPromoted").ok());
        out.push(IconEntry { key, is_promoted });
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
                    is_promoted: Some(1)
                },
                IconEntry {
                    key: "def".into(),
                    is_promoted: None
                },
            ]
        });
        let state = parse_state::<State>(&value, "t").unwrap();
        assert_eq!(state.icons.len(), 2);
        assert_eq!(state.icons[1].is_promoted, None);
        assert!(parse_state::<State>(&serde_json::json!({}), "t").is_err());
    }

    #[test]
    fn is_on_when_nothing_is_promoted() {
        let promoted = serde_json::json!(State {
            icons: vec![IconEntry {
                key: "k".into(),
                is_promoted: Some(1)
            }]
        });
        let demoted = serde_json::json!(State {
            icons: vec![
                IconEntry {
                    key: "k".into(),
                    is_promoted: Some(0)
                },
                IconEntry {
                    key: "j".into(),
                    is_promoted: None
                },
            ]
        });
        assert!(!TrayIcons.is_on(&promoted));
        assert!(TrayIcons.is_on(&demoted));
        assert!(!TrayIcons.is_on(&serde_json::json!({})));
    }

    #[test]
    fn decide_keeps_own_icon_promoted() {
        let c = ctx(r"C:\button\button.exe", &[]);
        assert_eq!(
            decide(
                Some(r"C:\BUTTON\BUTTON.EXE"),
                Some(0),
                &c.own_exe,
                &c.tray_visible
            ),
            1
        );
        assert_eq!(
            decide(
                Some(r"C:\button\button.exe"),
                None,
                &c.own_exe,
                &c.tray_visible
            ),
            1
        );
    }

    #[test]
    fn decide_keeps_visible_shelf_at_prior_value() {
        let c = ctx(
            r"C:\button\button.exe",
            &[r"C:\Program Files\OBS\obs64.exe"],
        );
        assert_eq!(
            decide(
                Some(r"c:\program files\obs\OBS64.EXE"),
                Some(1),
                &c.own_exe,
                &c.tray_visible
            ),
            1
        );
        assert_eq!(
            decide(
                Some(r"C:\Program Files\OBS\obs64.exe"),
                None,
                &c.own_exe,
                &c.tray_visible
            ),
            0
        );
    }

    #[test]
    fn decide_demotes_everything_else() {
        let c = ctx(r"C:\button\button.exe", &[r"C:\keep\app.exe"]);
        assert_eq!(
            decide(
                Some(r"C:\other\app.exe"),
                Some(1),
                &c.own_exe,
                &c.tray_visible
            ),
            0
        );
        assert_eq!(
            decide(None, None, &c.own_exe, &c.tray_visible),
            0,
            "unresolvable path"
        );
        assert_eq!(
            decide(Some(""), None, &c.own_exe, &c.tray_visible),
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
