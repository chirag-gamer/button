//! Lock Keyboard (FR-34, spec 7.10, task 4.6). A dedicated thread installs
//! `WH_KEYBOARD_LL` and swallows every key except Esc, which unlocks. The
//! callback does no I/O (LowLevelHooksTimeout, research/02). An overlay
//! window (lock.html) offers a mouse Unlock button — built lazily on the
//! hook thread, because `WebviewWindowBuilder` deadlocks in sync commands
//! on the main thread (D-071's finding).
//!
//! Limits (spec 7.10): Ctrl+Alt+Del and Win+L cannot be blocked; elevated
//! windows still receive keys (UIPI); Alt+R is swallowed while locked; if
//! button exits, the keyboard works again (the hook dies with the process).

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
#[cfg(not(test))]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

// Everything Windows-hook related lives on the real lock thread, which
// test builds do not spawn (D-074).
#[cfg(not(test))]
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
#[cfg(not(test))]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(not(test))]
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
#[cfg(not(test))]
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, PeekMessageW,
    PostQuitMessage, PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN, WM_USER,
};

use super::{Availability, Ctx, Switch, SwitchError, SwitchId, parse_state};
use crate::os::OsInfo;

#[cfg(not(test))]
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static LOCKED: AtomicBool = AtomicBool::new(false);
/// The hook thread's id while a lock is installed; WM_QUIT is posted here
/// to unlock. Never joined: the thread finishes by itself.
static HOOK_THREAD: Mutex<Option<u32>> = Mutex::new(None);

/// The overlay and the post-unlock refresh need window access (D-074's
/// cfg split applies to everything below that touches Tauri windows).
#[cfg(not(test))]
use tauri::Manager;

/// State shape: `{"locked": bool}` (spec 5.3).
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub locked: bool,
}

pub struct LockKeyboard;

/// Hand the switch the AppHandle for the overlay + post-unlock refresh
/// (same pattern as the backdrop Shell module). Setup calls this.
pub fn set_app(app: tauri::AppHandle) {
    #[cfg(not(test))]
    let _ = APP.set(app);
    #[cfg(test)]
    let _ = &app; // no overlay in test binaries (D-074)
}

/// Unlock now (the IPC command, or the overlay's Unlock button): post
/// WM_QUIT to the hook thread; it unhooks, clears the flag, destroys the
/// overlay and refreshes the UI.
pub fn unlock() {
    let tid = HOOK_THREAD.lock().unwrap_or_else(|p| p.into_inner()).take();
    #[cfg(not(test))]
    if let Some(tid) = tid {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
    #[cfg(test)]
    {
        // Test builds carry no hook thread (D-074); the flag is the state.
        let _ = tid;
        LOCKED.store(false, Ordering::SeqCst);
    }
}

impl Switch for LockKeyboard {
    fn id(&self) -> SwitchId {
        SwitchId::LockKeyboard
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available
    }

    fn persistent(&self) -> bool {
        false // the hook dies with the process
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        Ok(serde_json::json!(State {
            locked: LOCKED.load(Ordering::SeqCst),
        }))
    }

    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "lock keyboard")
            .map(|s| s.locked)
            .unwrap_or(false)
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!(State { locked: true })
    }

    fn target_off(&self, _current: &serde_json::Value, _ctx: &Ctx) -> Option<serde_json::Value> {
        Some(serde_json::json!(State { locked: false }))
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "lock keyboard")?;
        if target.locked == LOCKED.load(Ordering::SeqCst) {
            return Ok(()); // idempotent
        }
        if target.locked {
            lock()
        } else {
            unlock();
            Ok(())
        }
    }

    // restore keeps the trait default: unlock only when still locked.
}

/// The low-level keyboard callback. No I/O, no locks, no logging — Windows
/// drops slow hooks (LowLevelHooksTimeout, max 1000 ms).
#[cfg(not(test))]
unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 {
            let key_down = wparam.0 == WM_KEYDOWN as usize || wparam.0 == WM_SYSKEYDOWN as usize;
            if key_down
                && let Some(kb) = (lparam.0 as *const KBDLLHOOKSTRUCT).as_ref()
                && kb.vkCode == VK_ESCAPE.0 as u32
            {
                // Esc unlocks: quit the thread's message loop (the cleanup
                // after the loop unhooks and clears the flag).
                PostQuitMessage(0);
            }
            return LRESULT(1); // swallow every key
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

#[cfg(not(test))]
fn lock() -> Result<(), SwitchError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let app = APP.get().cloned();
    let spawned = std::thread::Builder::new()
        .name("lock-keyboard".into())
        .spawn(move || hook_thread(app, tx));
    let _ = spawned.map_err(|e| SwitchError::Os {
        api: "CreateThread(lock-keyboard)",
        code: e.raw_os_error().unwrap_or(-1),
    })?;
    match rx.recv() {
        Ok(Ok(tid)) => {
            *HOOK_THREAD.lock().unwrap_or_else(|p| p.into_inner()) = Some(tid);
            LOCKED.store(true, Ordering::SeqCst);
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err(SwitchError::NotAvailable(
            "lock-keyboard thread is gone".into(),
        )),
    }
}

#[cfg(test)]
fn lock() -> Result<(), SwitchError> {
    // Test builds carry no window/hook code (D-074); the flag alone keeps
    // the state machine testable.
    LOCKED.store(true, Ordering::SeqCst);
    Ok(())
}

/// The lock's own thread: message queue, hook, overlay, message loop. On
/// WM_QUIT (Esc or the Unlock button): unhook, clear the flag, destroy the
/// overlay, refresh the UI.
#[cfg(not(test))]
fn hook_thread(
    app: Option<tauri::AppHandle>,
    tx: std::sync::mpsc::Sender<Result<u32, SwitchError>>,
) {
    unsafe {
        // Create the message queue first, so the hook can post to it
        // (probe pattern, 2026-10-27).
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
        let hmod = GetModuleHandleW(None)
            .ok()
            .map(|m| HINSTANCE(m.0))
            .unwrap_or_default();
        let Ok(hook) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(hmod), 0) else {
            let _ = tx.send(Err(SwitchError::Os {
                api: "SetWindowsHookExW(WH_KEYBOARD_LL)",
                code: 0,
            }));
            return;
        };

        // The overlay is a convenience; a failed build must not stop the
        // lock (Esc still unlocks).
        if let Some(app) = &app
            && let Err(e) = build_overlay(app)
        {
            log::warn!("lock-keyboard: overlay failed: {e}");
        }

        let _ = tx.send(Ok(windows::Win32::System::Threading::GetCurrentThreadId()));
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
    }
    LOCKED.store(false, Ordering::SeqCst);
    *HOOK_THREAD.lock().unwrap_or_else(|p| p.into_inner()) = None;
    if let Some(app) = &app {
        let for_closure = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(window) = for_closure.get_webview_window("lock") {
                let _ = window.destroy();
            }
        });
        crate::automation::refresh_switches(app);
    }
}

/// "Keyboard locked. Press Esc or click Unlock." — small, centred, never
/// focused so it cannot steal a keystroke from the hook.
#[cfg(not(test))]
fn build_overlay(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::WebviewUrl;
    if app.get_webview_window("lock").is_some() {
        return Ok(()); // a leftover window from an interrupted unlock
    }
    tauri::WebviewWindowBuilder::new(app, "lock", WebviewUrl::App("lock.html".into()))
        .title("button")
        .inner_size(360.0, 120.0)
        .center()
        .decorations(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .focusable(false)
        .build()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shape_round_trips() {
        assert!(
            parse_state::<State>(&serde_json::json!({ "locked": true }), "t")
                .unwrap()
                .locked
        );
        assert!(parse_state::<State>(&serde_json::json!({}), "t").is_err());
    }

    #[test]
    fn lock_unlock_round_trip_through_the_trait() {
        // Test builds stub the hook away (D-074); the state machine is
        // still exercised through apply/read.
        LockKeyboard
            .apply(&serde_json::json!(State { locked: true }))
            .unwrap();
        assert!(LockKeyboard.is_on(&LockKeyboard.read().unwrap()));
        LockKeyboard
            .apply(&serde_json::json!(State { locked: false }))
            .unwrap();
        assert!(!LockKeyboard.is_on(&LockKeyboard.read().unwrap()));
    }

    #[test]
    fn double_lock_and_double_unlock_are_no_ops() {
        LockKeyboard
            .apply(&serde_json::json!(State { locked: true }))
            .unwrap();
        LockKeyboard
            .apply(&serde_json::json!(State { locked: true }))
            .unwrap();
        unlock();
        unlock();
        assert!(!LockKeyboard.is_on(&LockKeyboard.read().unwrap()));
    }
}
