//! Foreground tracker (spec 7.8). The WinEvent hook keeps the last few
//! foreground windows; `front` picks the newest one (the live foreground
//! window counts as the newest) that is still a valid window and not on
//! Stays on Screen. The hook callback touches only its own mutex via
//! `try_lock`: no app state, no blocking, no panic. One
//! exception to the no-I/O rule: while a backdrop is visible, the callback
//! resolves the foreground window's exe (`backdrop_event_ok`, spec 7.9) —
//! that path runs only during a backdrop session and never blocks on app
//! state.

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
#[derive(Debug, Clone)]
struct Entry {
    hwnd: isize,
    pid: u32,
}

#[derive(Debug, Default)]
struct State {
    ring: VecDeque<Entry>,
}

static STATE: Mutex<State> = Mutex::new(State {
    ring: VecDeque::new(),
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
        // Seed the ring only when the hook works: without it nothing keeps
        // the entry fresh and front() would hand out a stale app forever.
        record(unsafe { GetForegroundWindow() });
    }
}

pub fn unhook() {
    let raw = HOOK.swap(0, Ordering::SeqCst);
    if raw != 0 {
        let _ = unsafe { UnhookWinEvent(HWINEVENTHOOK(raw as _)) };
    }
}

/// The newest foreground window that is still a valid window, whose process
/// still resolves to an exe, and that is neither on Stays on Screen nor a
/// shell flyout host (spec 7.8, D-066, D-073). The exe lookup runs here,
/// once per start — never in the hook.
pub fn front(stays_on_screen: &[String]) -> Option<FrontApp> {
    pick(&ring_now(), stays_on_screen, is_window, exe_of_pid)
}

/// The backdrop's z-order anchor (spec 7.9): the newest ring entry that
/// passes the class and exe ignores, WITHOUT the Stays-on-Screen filter —
/// the backdrop sits behind the recorder too; that is the point of a clean
/// recording background. Hide Other Apps restacks its restored windows
/// below the same window.
pub(crate) fn backdrop_anchor() -> Option<isize> {
    pick(&ring_now(), &[], is_window, exe_of_pid).map(|f| f.hwnd)
}

/// The ring as of now: the live foreground window counts as the newest
/// entry. The hook alone can trail the screen — it never sees foreground
/// changes button makes itself (WINEVENT_SKIPOWNPROCESS; Hide Other Apps
/// hands the focus back on restore) and drops events while the ring is
/// locked — and a stale entry would make the next start hide the app the
/// user is on.
fn ring_now() -> VecDeque<Entry> {
    let ring = STATE.lock().unwrap_or_else(|p| p.into_inner()).ring.clone();
    with_live(ring, entry_of(unsafe { GetForegroundWindow() }))
}

/// Records the window button just gave the focus to (the Hide Other Apps
/// hand-back). The hook may never see that change (it skips button's own
/// foreground changes, and GetForegroundWindow can read 0 while the switch
/// settles), so without this the ring's newest entry could stay on the
/// window that took the focus during the restore, and the next start from
/// the tray (where the live foreground is button's own dropdown) would keep
/// that window in front instead of the one the user is on.
pub(crate) fn note_foreground(hwnd: HWND) {
    let live = entry_of(hwnd);
    let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    let ring = std::mem::take(&mut state.ring);
    state.ring = with_live(ring, live);
}

fn with_live(mut ring: VecDeque<Entry>, live: Option<Entry>) -> VecDeque<Entry> {
    if let Some(live) = live
        && ring
            .back()
            .is_none_or(|newest| newest.hwnd != live.hwnd || newest.pid != live.pid)
    {
        push_entry(&mut ring, live);
    }
    ring
}

/// Whether a foreground event should move the backdrop (spec 7.9): not a
/// shell surface, not button, and the process resolves to a non-ignored
/// exe. This resolves the exe on the hook path — but only while a backdrop
/// is visible; the idle path is one atomic load in backdrop.rs.
pub(crate) fn backdrop_event_ok(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    let h = HWND(hwnd as *mut core::ffi::c_void);
    if is_ignored_class(&class_of(h)) {
        return false;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    if pid == 0 || pid == std::process::id() {
        return false;
    }
    matches!(exe_of_pid(pid), Some(exe) if !is_ignored_exe(&exe))
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
    crate::backdrop::on_foreground(hwnd.0 as isize);
}

/// Hook-callback side: cheap reads only (class name + pid). Opening the
/// process to resolve its exe happens in `front()`, off the event path.
fn record(hwnd: HWND) {
    // Out-of-context events arrive queued. One for a window that has lost
    // the foreground again by now (a restored window that activated itself
    // before Hide Other Apps handed the focus back) is stale and must not
    // become the newest entry.
    if unsafe { GetForegroundWindow() } != hwnd {
        return;
    }
    let Some(entry) = entry_of(hwnd) else {
        return;
    };
    if let Ok(mut state) = STATE.try_lock() {
        push_entry(&mut state.ring, entry);
    }
}

/// The ring entry for a foreground window, or None for button's own
/// windows and the ignored shell classes.
fn entry_of(hwnd: HWND) -> Option<Entry> {
    if hwnd.is_invalid() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 || pid == std::process::id() || is_ignored_class(&class_of(hwnd)) {
        return None;
    }
    Some(Entry {
        hwnd: hwnd.0 as isize,
        pid,
    })
}

fn pick(
    ring: &VecDeque<Entry>,
    stays_on_screen: &[String],
    valid: impl Fn(isize) -> bool,
    exe_of: impl Fn(u32) -> Option<String>,
) -> Option<FrontApp> {
    ring.iter().rev().find_map(|e| {
        if !valid(e.hwnd) {
            return None;
        }
        let exe = exe_of(e.pid)?;
        let excluded = stays_on_screen
            .iter()
            .any(|s| crate::switches::same_exe(s, &exe))
            || is_ignored_exe(&exe);
        if excluded {
            return None;
        }
        Some(FrontApp {
            hwnd: e.hwnd,
            pid: e.pid,
            exe,
        })
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

/// Classes that must never be the front app (spec 7.8): the taskbars, the
/// desktop itself, and the shell popup surfaces from the 2026-10-26 census —
/// the tray overflow flyout; Start, Task View and the Alt+Tab switcher
/// (`XamlExplorerHostIslandWindow`); Quick Settings (`ControlCenterWindow`);
/// and the transient staging window flyout animations push forward.
fn is_ignored_class(class: &str) -> bool {
    matches!(
        class,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "TopLevelWindowForOverflowXamlIsland"
            | "XamlExplorerHostIslandWindow"
            | "ControlCenterWindow"
            | "ForegroundStaging"
    )
}

/// Exes whose windows are shell flyouts even though the window class
/// (`Windows.UI.Core.CoreWindow`) is shared with real apps, so the exe
/// decides (census 2026-10-26; StartMenuExperienceHost is the Win10 Start).
fn is_ignored_exe(exe: &str) -> bool {
    matches!(
        exe,
        "SearchHost.exe" | "ShellExperienceHost.exe" | "StartMenuExperienceHost.exe"
    )
}

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub(crate) fn exe_of_pid(pid: u32) -> Option<String> {
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

    fn entry(hwnd: isize, pid: u32) -> Entry {
        Entry { hwnd, pid }
    }

    /// Test exe resolver: pid 2 poses as OBS, pid 22 has exited, pids 3/5
    /// are shell flyout hosts behind the shared CoreWindow class.
    fn exe_of(pid: u32) -> Option<String> {
        match pid {
            2 => Some("obs64.exe".to_string()),
            3 => Some("SearchHost.exe".to_string()),
            5 => Some("ShellExperienceHost.exe".to_string()),
            22 => None,
            _ => Some("app.exe".to_string()),
        }
    }

    #[test]
    fn picks_the_newest_valid_window_not_on_stays_on_screen() {
        let mut ring = VecDeque::new();
        push_entry(&mut ring, entry(1, 1));
        push_entry(&mut ring, entry(2, 2)); // Stays on Screen (obs64.exe)
        push_entry(&mut ring, entry(0, 3)); // dead window
        push_entry(&mut ring, entry(3, 4));
        let stays = ["obs64.exe".to_string()];
        let picked = pick(&ring, &stays, |hwnd| hwnd != 0, exe_of).unwrap();
        assert_eq!(picked.pid, 4);
        assert_eq!(picked.exe, "app.exe");
    }

    #[test]
    fn stays_on_screen_matches_case_insensitively() {
        let mut ring = VecDeque::new();
        push_entry(&mut ring, entry(1, 2)); // resolves to obs64.exe
        let stays = ["OBS64.EXE".to_string()];
        assert!(pick(&ring, &stays, |_| true, exe_of).is_none());
    }

    #[test]
    fn empty_ring_or_no_valid_window_gives_none() {
        let ring: VecDeque<Entry> = VecDeque::new();
        assert!(pick(&ring, &[], |_| true, exe_of).is_none());
        let mut ring = VecDeque::new();
        push_entry(&mut ring, entry(0, 1));
        assert!(pick(&ring, &[], |hwnd| hwnd != 0, exe_of).is_none());
    }

    #[test]
    fn an_entry_whose_process_no_longer_resolves_is_skipped() {
        let mut ring = VecDeque::new();
        push_entry(&mut ring, entry(1, 21));
        push_entry(&mut ring, entry(2, 22)); // process gone since the event
        let picked = pick(&ring, &[], |hwnd| hwnd != 0, exe_of).unwrap();
        assert_eq!(picked.pid, 21);
    }

    #[test]
    fn the_live_foreground_window_beats_a_stale_ring() {
        let mut ring = VecDeque::new();
        push_entry(&mut ring, entry(1, 1)); // what the hook saw last
        let now = with_live(ring.clone(), Some(entry(2, 4)));
        assert_eq!(pick(&now, &[], |_| true, exe_of).unwrap().hwnd, 2);
        // No usable live window (button's own, a shell surface): the ring.
        let now = with_live(ring.clone(), None);
        assert_eq!(pick(&now, &[], |_| true, exe_of).unwrap().hwnd, 1);
        // Already the newest entry: not pushed twice.
        assert_eq!(with_live(ring.clone(), Some(entry(1, 1))).len(), 1);
        // A recycled hwnd now owned by another process is a new entry.
        let now = with_live(ring, Some(entry(1, 4)));
        assert_eq!(now.len(), 2);
        assert_eq!(pick(&now, &[], |_| true, exe_of).unwrap().pid, 4);
    }

    #[test]
    fn the_ring_keeps_only_the_last_eight() {
        let mut ring = VecDeque::new();
        for hwnd in 0..12isize {
            push_entry(&mut ring, entry(hwnd, hwnd as u32));
        }
        assert_eq!(ring.len(), RING_CAP);
        assert_eq!(ring.front().unwrap().hwnd, 4); // oldest survivor
        assert_eq!(ring.back().unwrap().hwnd, 11);
    }

    #[test]
    fn ignored_classes_are_exactly_the_documented_list() {
        for class in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
            "TopLevelWindowForOverflowXamlIsland",
            "XamlExplorerHostIslandWindow",
            "ControlCenterWindow",
            "ForegroundStaging",
        ] {
            assert!(is_ignored_class(class));
        }
        // The CoreWindow class is shared with real apps: the exe decides.
        assert!(!is_ignored_class("Chrome_WidgetWin_1"));
        assert!(!is_ignored_class("Windows.UI.Core.CoreWindow"));
    }

    #[test]
    fn shell_flyout_hosts_are_ignored_even_through_the_shared_class() {
        let mut ring = VecDeque::new();
        push_entry(&mut ring, entry(1, 1));
        push_entry(&mut ring, entry(2, 3)); // Search flyout behind CoreWindow
        push_entry(&mut ring, entry(3, 5)); // Notification Center ditto
        let picked = pick(&ring, &[], |hwnd| hwnd != 0, exe_of).unwrap();
        assert_eq!(picked.pid, 1);
    }
}
