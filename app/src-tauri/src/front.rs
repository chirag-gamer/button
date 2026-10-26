//! Foreground tracker (spec 7.8). The WinEvent hook keeps the last few
//! foreground windows; `front` picks the newest one that is still a valid
//! window and not on Stays on Screen. The hook callback touches only its
//! own mutex via `try_lock`: no app state, no blocking, no I/O, no panic.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
    IsWindow, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};
use windows::core::PWSTR;

/// The front app the switches see (D-066).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontApp {
    pub hwnd: isize,
    pub pid: u32,
    pub exe: String,
}

const RING_CAP: usize = 8;
const CENSUS_CAP: usize = 64;

#[derive(Debug)]
struct Entry {
    hwnd: isize,
    pid: u32,
    exe: String,
}

#[derive(Debug, Default)]
struct State {
    ring: VecDeque<Entry>,
    // Temporary (task 3.0): every foreground (class, exe) pair, deduped, so
    // the owner can tell us which shell popups to ignore.
    census: Vec<(String, String)>,
}

static STATE: Mutex<State> = Mutex::new(State {
    ring: VecDeque::new(),
    census: Vec::new(),
});

// HWINEVENTHOOK is not Sync, so the raw handle is stored as an isize
// (0 = not installed) and reconstructed on unhook.
static HOOK: AtomicIsize = AtomicIsize::new(0);

/// Installs the foreground hook. Must run on the main thread: out-of-context
/// hooks deliver to the installing thread's message pump, which is Tauri's
/// event loop here (spec 2.3).
pub fn install() {
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(fg_cb),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    if hook.is_invalid() {
        log::warn!("foreground hook could not be installed; front stays empty");
    } else {
        HOOK.store(hook.0 as isize, Ordering::SeqCst);
    }
    record(unsafe { GetForegroundWindow() });
}

pub fn unhook() {
    let raw = HOOK.swap(0, Ordering::SeqCst);
    if raw != 0 {
        let _ = unsafe { UnhookWinEvent(HWINEVENTHOOK(raw as _)) };
    }
}

/// The newest foreground window that is still a valid window and not on
/// Stays on Screen (spec 7.8, D-066).
pub fn front(stays_on_screen: &[String]) -> Option<FrontApp> {
    let state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    pick(&state.ring, stays_on_screen, is_window).map(|e| FrontApp {
        hwnd: e.hwnd,
        pid: e.pid,
        exe: e.exe.clone(),
    })
}

/// Temporary (task 3.0): log and clear the foreground census so the owner
/// can tell us which shell popup class+exe pairs to ignore. Removed once
/// the ignore list is settled.
pub fn dump_census() {
    let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    for (class, exe) in state.census.drain(..) {
        log::info!("foreground census: class={class} exe={exe}");
    }
}

unsafe extern "system" fn fg_cb(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    record(hwnd);
}

fn record(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }
    let class = class_of(hwnd);
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 || pid == std::process::id() {
        return;
    }
    let Some(exe) = exe_of_pid(pid) else {
        return;
    };
    if let Ok(mut state) = STATE.try_lock() {
        let known = state.census.iter().any(|(c, e)| c == &class && e == &exe);
        if !known && state.census.len() < CENSUS_CAP {
            state.census.push((class.clone(), exe.clone()));
        }
        if !is_ignored_class(&class) {
            push_entry(
                &mut state.ring,
                Entry {
                    hwnd: hwnd.0 as isize,
                    pid,
                    exe,
                },
            );
        }
    }
}

fn pick<'a>(
    ring: &'a VecDeque<Entry>,
    stays_on_screen: &[String],
    valid: impl Fn(isize) -> bool,
) -> Option<&'a Entry> {
    ring.iter().rev().find(|e| {
        valid(e.hwnd)
            && !stays_on_screen
                .iter()
                .any(|s| crate::switches::same_exe(s, &e.exe))
    })
}

fn is_window(hwnd: isize) -> bool {
    unsafe { IsWindow(Some(HWND(hwnd as *mut core::ffi::c_void))).as_bool() }
}

fn push_entry(ring: &mut VecDeque<Entry>, entry: Entry) {
    ring.push_back(entry);
    while ring.len() > RING_CAP {
        ring.pop_front();
    }
}

/// Classes that must never be the front app (spec 7.8): the taskbars and
/// the desktop itself.
fn is_ignored_class(class: &str) -> bool {
    matches!(
        class,
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    )
}

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn exe_of_pid(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let read = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        read.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Some(path.rsplit('\\').next().unwrap_or(&path).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(hwnd: isize, exe: &str) -> Entry {
        Entry {
            hwnd,
            pid: 1,
            exe: exe.to_string(),
        }
    }

    #[test]
    fn picks_the_newest_valid_window_not_on_stays_on_screen() {
        let mut ring = VecDeque::new();
        push_entry(&mut ring, entry(1, "explorer.exe"));
        push_entry(&mut ring, entry(2, "obs64.exe")); // Stays on Screen
        push_entry(&mut ring, entry(0, "code.exe")); // dead window
        push_entry(&mut ring, entry(3, "chrome.exe"));
        let stays = ["obs64.exe".to_string()];
        let picked = pick(&ring, &stays, |hwnd| hwnd != 0).unwrap();
        assert_eq!(picked.exe, "chrome.exe");
    }

    #[test]
    fn stays_on_screen_matches_case_insensitively() {
        let mut ring = VecDeque::new();
        push_entry(&mut ring, entry(1, "obs64.exe"));
        let stays = ["OBS64.EXE".to_string()];
        assert!(pick(&ring, &stays, |_| true).is_none());
    }

    #[test]
    fn empty_ring_or_no_valid_window_gives_none() {
        let ring: VecDeque<Entry> = VecDeque::new();
        assert!(pick(&ring, &[], |_| true).is_none());
        let mut ring = VecDeque::new();
        push_entry(&mut ring, entry(0, "chrome.exe"));
        assert!(pick(&ring, &[], |hwnd| hwnd != 0).is_none());
    }

    #[test]
    fn the_ring_keeps_only_the_last_eight() {
        let mut ring = VecDeque::new();
        for hwnd in 0..12isize {
            push_entry(&mut ring, entry(hwnd, "app.exe"));
        }
        assert_eq!(ring.len(), RING_CAP);
        assert_eq!(ring.front().unwrap().hwnd, 4); // oldest survivor
        assert_eq!(ring.back().unwrap().hwnd, 11);
    }

    #[test]
    fn ignored_classes_are_exactly_the_documented_four() {
        for class in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
        ] {
            assert!(is_ignored_class(class));
        }
        assert!(!is_ignored_class("Chrome_WidgetWin_1"));
    }
}
