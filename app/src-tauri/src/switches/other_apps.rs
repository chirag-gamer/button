//! Hide Other Apps (FR-16, spec 7.8). Minimizes every eligible top-level
//! window except the front app, button itself and the Stays on Screen apps;
//! each window's `WINDOWPLACEMENT` is journaled so restore puts it back
//! exactly. Restore only touches windows that are still theirs, still
//! exist and are still minimized ([D-016]).

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongPtrW, GetWindowPlacement,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, SW_MINIMIZE, SetWindowPlacement,
    ShowWindowAsync, WINDOWPLACEMENT, WINDOWPLACEMENT_FLAGS, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use super::{
    Availability, Ctx, RestoreOutcome, Switch, SwitchError, SwitchId, parse_state, same_exe,
};
use crate::journal::JournalEntry;
use crate::os::OsInfo;

/// State shape: `{"windows": [...]}` — one entry per eligible window with
/// its saved placement.
#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub windows: Vec<Entry>,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub hwnd: isize,
    pub pid: u32,
    pub exe: String,
    pub placement: Placement,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub flags: u32,
    pub show_cmd: u32,
    pub min_position: Point,
    pub max_position: Point,
    pub normal_position: Rect,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

pub struct OtherApps;

impl Switch for OtherApps {
    fn id(&self) -> SwitchId {
        SwitchId::OtherApps
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available
    }

    fn persistent(&self) -> bool {
        // Minimization outlives the process (the windows stay minimized),
        // so crash recovery must put them back (spec 7.3).
        true
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        Ok(serde_json::json!(State {
            windows: candidates()?,
        }))
    }

    /// The visible candidates minus the front app's process, button itself
    /// and Stays on Screen, each with a fresh `WINDOWPLACEMENT` (read-only
    /// work stays in target_on; apply only mutates, spec 7.8). When no
    /// front app is tracked (hook failure), the switch does nothing:
    /// without a front process there is no way to tell the app being
    /// shown apart from the ones to hide, and minimizing the foreground
    /// app would be wrong.
    fn target_on(&self, before: &serde_json::Value, ctx: &Ctx) -> serde_json::Value {
        let before = parse_state::<State>(before, "other apps").unwrap_or(State {
            windows: Vec::new(),
        });
        let Some(front_pid) = ctx.front.as_ref().map(|f| f.pid) else {
            log::warn!("other-apps: no front app tracked; leaving windows alone");
            return serde_json::json!(before);
        };
        let windows = before
            .windows
            .into_iter()
            .filter(|entry| keep(entry, Some(front_pid), ctx.own_pid, &ctx.stays_on_screen))
            .filter_map(|entry| {
                // Re-validate what could have changed since read(): the
                // window must still exist, be visible and not be minimized
                // already; the placement is captured fresh.
                still_a_candidate(&entry)
                    .then_some(entry)
                    .and_then(|entry| {
                        hwnd_of(entry.hwnd)
                            .and_then(placement_of)
                            .map(|placement| Entry { placement, ..entry })
                    })
            })
            .collect();
        serde_json::json!(State { windows })
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "other apps")?;
        for entry in target.windows {
            let Some(hwnd) = hwnd_of(entry.hwnd) else {
                continue; // gone since target_on: nothing to minimize
            };
            // Async so a hung app can't block us (spec 7.8, KNOW).
            unsafe {
                let _ = ShowWindowAsync(hwnd, SW_MINIMIZE);
            }
        }
        Ok(())
    }

    fn restore(&self, e: &JournalEntry) -> Result<RestoreOutcome, SwitchError> {
        let after = parse_state::<State>(&e.after, "other apps")?;
        let mut restored = 0;
        let mut user_changed = 0;
        let mut last_error: Option<SwitchError> = None;
        for entry in after.windows {
            let Some(hwnd) = hwnd_of(entry.hwnd) else {
                continue;
            };
            unsafe {
                if !IsWindow(Some(hwnd)).as_bool() {
                    continue; // gone
                }
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid != entry.pid
                    || crate::front::exe_of_pid(pid).is_some_and(|exe| !same_exe(&exe, &entry.exe))
                {
                    continue; // hwnd recycled to another process
                }
                if !IsIconic(hwnd).as_bool() {
                    user_changed += 1;
                    let raw = entry.hwnd;
                    log::info!("other-apps: hwnd {raw:#x} is no longer minimized; leaving it");
                    continue;
                }
                match SetWindowPlacement(hwnd, &placement_of_entry(&entry)) {
                    Ok(()) => restored += 1,
                    Err(err) => {
                        // Swallowing this would drop the journaled entry
                        // with the window still minimized and no record
                        // of it (the engine reports failed entries to
                        // the user, spec 7.3).
                        let raw = entry.hwnd;
                        log::warn!("other-apps: hwnd {raw:#x} did not restore: {err}");
                        last_error = Some(SwitchError::Os {
                            api: "SetWindowPlacement",
                            code: err.code().0,
                        });
                    }
                }
            }
        }
        if let Some(e) = last_error {
            return Err(e);
        }
        Ok(outcome(restored, user_changed))
    }

    /// "On" reads as "no candidate is visible any more". Best effort: the
    /// trait's `is_on` gets no `Ctx`, so the front app's and Stays on
    /// Screen windows still count as candidates here — during a session
    /// the flag can read off even though every eligible window is
    /// minimized (spec 7.8; the 4.1 switch-list UI replaces this with a
    /// ctx-aware check).
    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "other apps")
            .map(|state| state.windows.is_empty())
            .unwrap_or(false)
    }
}

/// The pure part of the ctx filter (spec 7.8): not the front app's process,
/// not button, not Stays on Screen.
fn keep(entry: &Entry, front_pid: Option<u32>, own_pid: u32, stays_on_screen: &[String]) -> bool {
    Some(entry.pid) != front_pid
        && entry.pid != own_pid
        && !stays_on_screen.iter().any(|s| same_exe(s, &entry.exe))
}

/// Restored something, someone else touched a window, or nothing was left.
fn outcome(restored: usize, user_changed: usize) -> RestoreOutcome {
    if restored > 0 {
        RestoreOutcome::Restored
    } else if user_changed > 0 {
        RestoreOutcome::UserChanged
    } else {
        RestoreOutcome::Gone
    }
}

/// Enumerate the visible candidates (spec 7.8): visible, not minimized, no
/// owner, no `WS_EX_TOOLWINDOW`, not DWM-cloaked, not button's process.
/// Elevated processes are skipped in read ([D-070]); a failed token read
/// counts as elevated, and the check is skipped when button itself is
/// elevated. Also the source of `list_apps` (FR-43).
pub(crate) fn candidates() -> Result<Vec<Entry>, SwitchError> {
    let mut found: Vec<(HWND, u32)> = Vec::new();
    unsafe {
        EnumWindows(Some(collect), LPARAM(&mut found as *mut _ as isize)).map_err(|e| {
            SwitchError::Os {
                api: "EnumWindows",
                code: e.code().0,
            }
        })?;
    }
    let check_elevation = !self_elevated();
    let mut windows = Vec::new();
    for (hwnd, pid) in found {
        if pid == std::process::id() {
            continue;
        }
        if check_elevation && matches!(elevated(pid), None | Some(true)) {
            log::info!("other-apps: skipping elevated process {pid}");
            continue;
        }
        let Some(exe) = crate::front::exe_of_pid(pid) else {
            continue;
        };
        let Some(placement) = placement_of(hwnd) else {
            continue;
        };
        windows.push(Entry {
            hwnd: hwnd.0 as isize,
            pid,
            exe,
            placement,
        });
    }
    Ok(windows)
}

unsafe extern "system" fn collect(h: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let out = &mut *(lparam.0 as *mut Vec<(HWND, u32)>);
        // Minimized windows are still "visible" in the IsWindowVisible
        // sense but are not candidates: they are already where Recording
        // Mode would put them, and counting them keeps is_on off after
        // apply.
        if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
            return BOOL(1);
        }
        if GetWindow(h, GW_OWNER).is_ok() {
            return BOOL(1);
        }
        if GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL(1);
        }
        // A failed cloak query keeps the window (fail-open, probe pattern).
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            h,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut core::ffi::c_void,
            4,
        );
        if cloaked != 0 {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == std::process::id() {
            return BOOL(1);
        }
        out.push((h, pid));
        BOOL(1)
    }
}

fn still_a_candidate(entry: &Entry) -> bool {
    hwnd_of(entry.hwnd)
        .map(|hwnd| unsafe {
            IsWindow(Some(hwnd)).as_bool()
                && IsWindowVisible(hwnd).as_bool()
                && !IsIconic(hwnd).as_bool()
        })
        .unwrap_or(false)
}

fn hwnd_of(raw: isize) -> Option<HWND> {
    (raw != 0).then_some(HWND(raw as *mut core::ffi::c_void))
}

fn placement_of(hwnd: HWND) -> Option<Placement> {
    unsafe {
        let mut wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        GetWindowPlacement(hwnd, &mut wp).ok()?;
        Some(Placement {
            flags: wp.flags.0,
            show_cmd: wp.showCmd,
            min_position: Point {
                x: wp.ptMinPosition.x,
                y: wp.ptMinPosition.y,
            },
            max_position: Point {
                x: wp.ptMaxPosition.x,
                y: wp.ptMaxPosition.y,
            },
            normal_position: Rect {
                left: wp.rcNormalPosition.left,
                top: wp.rcNormalPosition.top,
                right: wp.rcNormalPosition.right,
                bottom: wp.rcNormalPosition.bottom,
            },
        })
    }
}

fn placement_of_entry(entry: &Entry) -> WINDOWPLACEMENT {
    WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        flags: WINDOWPLACEMENT_FLAGS(entry.placement.flags),
        showCmd: entry.placement.show_cmd,
        ptMinPosition: POINT {
            x: entry.placement.min_position.x,
            y: entry.placement.min_position.y,
        },
        ptMaxPosition: POINT {
            x: entry.placement.max_position.x,
            y: entry.placement.max_position.y,
        },
        rcNormalPosition: RECT {
            left: entry.placement.normal_position.left,
            top: entry.placement.normal_position.top,
            right: entry.placement.normal_position.right,
            bottom: entry.placement.normal_position.bottom,
        },
    }
}

/// Some(true) elevated, Some(false) not, None = the token can't be opened
/// (treat as elevated, [D-070]).
fn elevated(pid: u32) -> Option<bool> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut token = HANDLE::default();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token);
        let _ = windows::Win32::Foundation::CloseHandle(process);
        opened.ok()?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        let read = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        let _ = windows::Win32::Foundation::CloseHandle(token);
        read.ok()?;
        Some(elevation.TokenIsElevated != 0)
    }
}

fn self_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        let _ = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        let _ = windows::Win32::Foundation::CloseHandle(token);
        elevation.TokenIsElevated != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32, exe: &str) -> Entry {
        Entry {
            hwnd: 1,
            pid,
            exe: exe.to_string(),
            placement: Placement {
                flags: 0,
                show_cmd: 1,
                min_position: Point { x: 0, y: 0 },
                max_position: Point { x: 0, y: 0 },
                normal_position: Rect {
                    left: 0,
                    top: 0,
                    right: 800,
                    bottom: 600,
                },
            },
        }
    }

    #[test]
    fn state_shape_round_trips() {
        let value = serde_json::json!(State {
            windows: vec![entry(7, "app.exe")]
        });
        let state = parse_state::<State>(&value, "t").unwrap();
        assert_eq!(state.windows.len(), 1);
        assert_eq!(state.windows[0].exe, "app.exe");
        assert!(parse_state::<State>(&serde_json::json!({}), "t").is_err());
    }

    #[test]
    fn keep_excludes_front_own_and_stays_on_screen() {
        let stays = ["obs64.exe".to_string()];
        assert!(keep(&entry(5, "app.exe"), Some(9), 1, &stays));
        assert!(
            !keep(&entry(9, "app.exe"), Some(9), 1, &stays),
            "front app's process"
        );
        assert!(
            !keep(&entry(1, "button.exe"), None, 1, &stays),
            "button itself"
        );
        assert!(!keep(&entry(5, "obs64.exe"), None, 1, &stays));
        assert!(
            !keep(&entry(5, "OBS64.EXE"), None, 1, &stays),
            "case-insensitive"
        );
    }

    #[test]
    fn outcome_prefers_restored_then_user_changed_then_gone() {
        assert_eq!(outcome(1, 2), RestoreOutcome::Restored);
        assert_eq!(outcome(0, 2), RestoreOutcome::UserChanged);
        assert_eq!(outcome(0, 0), RestoreOutcome::Gone);
    }

    /// Read-only live probe of the enumeration on a real desktop; CI may
    /// see no windows at all, which is a valid empty state.
    #[test]
    fn read_reports_the_live_candidates() {
        match candidates() {
            Ok(windows) => {
                assert!(windows.iter().all(|e| !e.exe.is_empty()));
                assert!(windows.iter().all(|e| e.pid != std::process::id()));
            }
            Err(SwitchError::Os { .. }) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
