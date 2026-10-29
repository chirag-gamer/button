//! Lock Keyboard (FR-34, spec 7.10, task 4.6). A dedicated thread installs
//! `WH_KEYBOARD_LL` and swallows every key except Esc, which unlocks. The
//! callback does no I/O (LowLevelHooksTimeout, research/02). An overlay
//! window (lock.html) offers a mouse Unlock button. It is built on its own
//! thread (D-095): `WebviewWindowBuilder` deadlocks in sync commands on the
//! main thread (D-071's finding), and building it on the hook thread made
//! the lock wait on a main thread that was waiting on the lock.
//!
//! Limits (spec 7.10): Ctrl+Alt+Del and Win+L cannot be blocked; elevated
//! windows still receive keys (UIPI); Alt+R is swallowed while locked; if
//! button exits, the keyboard works again (the hook dies with the process).

use serde::{Deserialize, Serialize};
#[cfg(not(test))]
use std::sync::Mutex;
#[cfg(not(test))]
use std::sync::OnceLock;
#[cfg(not(test))]
use std::sync::atomic::AtomicU64;
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

/// The live lock, owned by the hook thread alone: it fills the slot when
/// the hook is installed (before signalling readiness) and clears it on
/// exit, so an Esc pressed in between can never race a stale write-back.
/// `done` closes when the thread has finished unlocking.
#[cfg(not(test))]
struct HookHandle {
    /// The hook thread, for WM_QUIT.
    tid: u32,
    /// This lock's number. Ownership checks and the overlay label use it,
    /// not `tid`: the overlay thread can outlive the hook thread, and
    /// Windows may give a later lock thread the same id.
    generation: u64,
    done: std::sync::mpsc::Receiver<()>,
}

#[cfg(not(test))]
static HOOK_THREAD: Mutex<Option<HookHandle>> = Mutex::new(None);

/// Source of `HookHandle::generation`.
#[cfg(not(test))]
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

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
/// WM_QUIT to the hook thread and wait (bounded) for its cleanup, so a
/// quick re-lock cannot hit the stale `LOCKED` idempotence check. The
/// thread unhooks, clears the flag, destroys the overlay and refreshes
/// the UI.
pub fn unlock() {
    #[cfg(not(test))]
    {
        // The take is bound in its own `let` on purpose: a MutexGuard
        // temporary in an if-let scrutinee lives through the whole block
        // (verified empirically, edition 2024), which would deadlock the
        // hook thread's exit-path lock() for the full 500 ms wait.
        let handle = HOOK_THREAD.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(handle) = handle {
            unsafe {
                let _ = PostThreadMessageW(handle.tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            let _ = handle
                .done
                .recv_timeout(std::time::Duration::from_millis(500));
        }
    }
    #[cfg(test)]
    {
        // Test builds carry no hook thread (D-074); the flag is the state.
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
    std::thread::Builder::new()
        .name("lock-keyboard".into())
        .spawn(move || hook_thread(app, tx))
        .map_err(|e| SwitchError::Os {
            api: "CreateThread(lock-keyboard)",
            code: e.raw_os_error().unwrap_or(-1),
        })?;
    // The thread owns HOOK_THREAD/LOCKED by the time it signals readiness;
    // the parent only relays the outcome.
    match rx.recv() {
        Ok(Ok(())) => Ok(()),
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

/// The lock's own thread: message queue, hook, overlay thread, message loop. On
/// WM_QUIT (Esc or the Unlock button): unhook, clear the state it owns,
/// destroy the overlay, refresh the UI, and close the done channel.
#[cfg(not(test))]
fn hook_thread(
    app: Option<tauri::AppHandle>,
    tx: std::sync::mpsc::Sender<Result<(), SwitchError>>,
) {
    // Create the message queue first, so the hook can post to it
    // (probe pattern, 2026-10-27).
    let mut msg = MSG::default();
    unsafe {
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
    }
    let hmod = unsafe { GetModuleHandleW(None) }
        .ok()
        .map(|m| HINSTANCE(m.0))
        .unwrap_or_default();
    let hook =
        match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(hmod), 0) } {
            Ok(hook) => hook,
            Err(e) => {
                let _ = tx.send(Err(SwitchError::Os {
                    api: "SetWindowsHookExW(WH_KEYBOARD_LL)",
                    code: e.code().0,
                }));
                return;
            }
        };

    // Own the state BEFORE signalling readiness: an Esc pressed the instant
    // the hook is live must not race the parent's bookkeeping (review
    // finding: the parent used to write these after the handshake).
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let tid = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    *HOOK_THREAD.lock().unwrap_or_else(|p| p.into_inner()) = Some(HookHandle {
        tid,
        generation,
        done: done_rx,
    });
    LOCKED.store(true, Ordering::SeqCst);

    // The overlay is a convenience; a failed build must not stop the
    // lock (Esc still unlocks). Its builder blocks until the main thread
    // creates the window, and the main thread is usually inside lock()
    // waiting for the readiness below, so the build gets its own thread
    // (D-095). The hook thread must keep pumping: the hook runs on it.
    if let Some(app) = app.clone() {
        let spawned = std::thread::Builder::new()
            .name("lock-overlay".into())
            .spawn(move || build_overlay(&app, generation));
        if let Err(e) = spawned {
            log::warn!("lock-keyboard: overlay thread failed: {e}");
        }
    }

    let _ = tx.send(Ok(()));
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
    }
    // A wedged thread that outlives unlock()'s 500 ms wait must not wipe a
    // NEWER lock's state: touch it only when the installed handle is still
    // ours (or gone, i.e. unlock() already took it).
    let newer_lock_installed = {
        let mut slot = HOOK_THREAD.lock().unwrap_or_else(|p| p.into_inner());
        let newer = matches!(*slot, Some(ref handle) if handle.generation != generation);
        if !newer {
            *slot = None;
            LOCKED.store(false, Ordering::SeqCst);
        }
        newer
    };
    let _ = done_tx.send(());
    if let Some(app) = &app {
        // This lock's overlay only (its label carries the generation): a newer
        // lock's overlay is never touched. A build still in flight finds
        // the lock gone and destroys its own window (build_overlay).
        let for_closure = app.clone();
        let label = overlay_label(generation);
        let _ = app.run_on_main_thread(move || {
            if let Some(window) = for_closure.get_webview_window(&label) {
                let _ = window.destroy();
            }
        });
        if !newer_lock_installed {
            crate::automation::refresh_switches(app);
        }
    }
}

/// One overlay label per lock (the countdown's per-generation pattern):
/// each lock only ever destroys its own overlay.
#[cfg(not(test))]
fn overlay_label(generation: u64) -> String {
    format!("lock-{generation}")
}

/// Whether lock `generation` is still the installed one.
#[cfg(not(test))]
fn lock_is_current(generation: u64) -> bool {
    let slot = HOOK_THREAD.lock().unwrap_or_else(|p| p.into_inner());
    matches!(*slot, Some(ref handle) if handle.generation == generation)
}

/// "Keyboard locked. Press Esc or click Unlock." — small, centred, never
/// focused so it cannot steal a keystroke from the hook. Runs on the
/// lock-overlay thread. If the lock ended while the window was building,
/// the hook thread's cleanup found nothing to destroy, so this does it.
#[cfg(not(test))]
fn build_overlay(app: &tauri::AppHandle, generation: u64) {
    use tauri::WebviewUrl;
    let built = tauri::WebviewWindowBuilder::new(
        app,
        overlay_label(generation),
        WebviewUrl::App("lock.html".into()),
    )
    .title("button")
    .inner_size(360.0, 120.0)
    .center()
    .decorations(false)
    .skip_taskbar(true)
    .always_on_top(true)
    .focusable(false)
    .build();
    match built {
        Ok(window) => {
            if !lock_is_current(generation) {
                let _ = window.destroy();
            }
        }
        Err(e) => log::warn!("lock-keyboard: overlay failed: {e}"),
    }
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

    /// One test owns the LOCKED static from start to end (cargo test runs
    /// the module's tests in parallel threads; two writers race).
    #[test]
    fn lock_unlock_round_trip_and_double_calls_are_no_ops() {
        // Test builds stub the hook away (D-074); the state machine is
        // still exercised through apply/read/unlock.
        LockKeyboard
            .apply(&serde_json::json!(State { locked: true }))
            .unwrap();
        assert!(LockKeyboard.is_on(&LockKeyboard.read().unwrap()));
        // Double lock is a no-op, double unlock too.
        LockKeyboard
            .apply(&serde_json::json!(State { locked: true }))
            .unwrap();
        LockKeyboard
            .apply(&serde_json::json!(State { locked: false }))
            .unwrap();
        unlock();
        unlock();
        assert!(!LockKeyboard.is_on(&LockKeyboard.read().unwrap()));
    }
}
