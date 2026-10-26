//! Backdrop windows (FR-20, spec 7.9): one solid-colour webview per
//! monitor present at launch ([D-071]), shown behind the front app while a
//! Recording Mode session is active. Shell-owned — these are button's own
//! windows, like the dropdown; the switch in `switches/backdrop.rs` only
//! decides when they show (spec 2.1). The colour is applied once at
//! creation from settings; changing it later is a Phase 5 concern (FR-46).
//!
//! Z-order, proven by the U-4 spike (2026-10-27, D:\button-probe `zorder`):
//! `SetWindowPos` with `hWndInsertAfter = anchor` and `SWP_NOACTIVATE`
//! makes the backdrop the topmost *visible* window below the anchor —
//! invisible system windows (IME etc.) may legally interleave. A
//! `WS_EX_NOACTIVATE` window (Tauri `focusable(false)`) never takes focus.

use std::sync::Mutex;
#[cfg(not(test))]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::AppHandle;
#[cfg(not(test))]
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
};

#[cfg(not(test))]
static APP: OnceLock<AppHandle> = OnceLock::new();
static SHOWN: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct BackdropWindow {
    #[cfg_attr(test, allow(dead_code))]
    label: String,
    hwnd: isize,
}

static WINDOWS: Mutex<Vec<BackdropWindow>> = Mutex::new(Vec::new());

/// Fallback when settings hold an unparsable colour (validated on save,
/// but the file can be hand-edited): the spec default #202020.
#[cfg(not(test))]
const DEFAULT_COLOR: (u8, u8, u8) = (0x20, 0x20, 0x20);

/// "#rrggbb" → (r, g, b); None when malformed.
fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Creates one hidden backdrop window per monitor present at launch
/// ([D-071]). Runs in setup: WebviewWindowBuilder deadlocks inside sync
/// commands and handlers on Windows.
///
/// Test builds get a stub below: pulling the window-creation code into a
/// test binary would import comctl32 v6-only functions (TaskDialogIndirect
/// via muda/tauri-runtime-wry), and test executables have no comctl32 v6
/// application manifest, so the loader kills them with
/// STATUS_ENTRYPOINT_NOT_FOUND before any test runs. Unit tests can't
/// create webview windows anyway — `show` reports that instead.
#[cfg(not(test))]
pub fn create(app: &AppHandle, color: &str) {
    let _ = APP.set(app.clone());
    let monitors = match app.available_monitors() {
        Ok(m) if !m.is_empty() => m,
        Ok(_) => {
            log::warn!("backdrop: no monitors reported; backdrop unavailable");
            return;
        }
        Err(e) => {
            log::warn!("backdrop: monitor enumeration failed: {e}");
            return;
        }
    };
    let (r, g, b) = parse_hex_color(color).unwrap_or_else(|| {
        log::warn!("backdrop: unparsable colour {color:?}; using the default");
        DEFAULT_COLOR
    });
    let mut windows = WINDOWS.lock().unwrap_or_else(|p| p.into_inner());
    for (i, monitor) in monitors.iter().enumerate() {
        let label = format!("backdrop-{i}");
        match WebviewWindowBuilder::new(app, label.clone(), WebviewUrl::App("backdrop.html".into()))
            .decorations(false)
            .focusable(false)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .visible(false)
            .background_color(tauri::window::Color(r, g, b, 255))
            .build()
        {
            Ok(window) => {
                let _ = window.set_position(*monitor.position());
                let _ = window.set_size(*monitor.size());
                match window.hwnd() {
                    Ok(hwnd) => windows.push(BackdropWindow {
                        label,
                        hwnd: hwnd.0 as isize,
                    }),
                    Err(e) => log::warn!("backdrop: {label} has no hwnd: {e}"),
                }
            }
            Err(e) => log::error!("backdrop: {label} could not be created: {e}"),
        }
    }
    log::info!("backdrop: {} window(s) created", windows.len());
}

/// Shows every backdrop window on its monitor's current bounds (D-071:
/// rects recomputed on every apply) and drops each directly below the
/// front app in z-order (spec 7.9). The whole sequence runs on the main
/// thread so `show` (which raises to the top) cannot clobber the z-order
/// change.
#[cfg(not(test))]
pub fn show() -> Result<(), String> {
    let app = APP.get().ok_or("backdrop windows were not created")?;
    let snapshot: Vec<BackdropWindow> = {
        let windows = WINDOWS.lock().unwrap_or_else(|p| p.into_inner());
        if windows.is_empty() {
            return Err("no backdrop windows exist".into());
        }
        windows.clone()
    };
    // HWND(0) = HWND_TOP, the fallback when no valid anchor exists.
    let anchor = crate::front::backdrop_anchor().unwrap_or(0);
    let app2 = app.clone();
    app.run_on_main_thread(move || {
        let monitors = app2.available_monitors().ok();
        for (i, w) in snapshot.iter().enumerate() {
            let Some(window) = app2.get_webview_window(&w.label) else {
                continue;
            };
            if let Some(monitor) = monitors.as_ref().and_then(|m| m.get(i)) {
                let _ = window.set_position(*monitor.position());
                let _ = window.set_size(*monitor.size());
            }
            let _ = window.show();
            place_below(w.hwnd, anchor);
        }
    })
    .map_err(|e| format!("backdrop show failed: {e}"))?;
    SHOWN.store(true, Ordering::Relaxed);
    Ok(())
}

/// Hides every backdrop window. Best effort: hiding what does not exist
/// is not an error.
#[cfg(not(test))]
pub fn hide() {
    SHOWN.store(false, Ordering::Relaxed);
    let Some(app) = APP.get() else {
        return;
    };
    let labels: Vec<String> = {
        let windows = WINDOWS.lock().unwrap_or_else(|p| p.into_inner());
        windows.iter().map(|w| w.label.clone()).collect()
    };
    if labels.is_empty() {
        return;
    }
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        for label in labels {
            if let Some(window) = app2.get_webview_window(&label) {
                let _ = window.hide();
            }
        }
    });
}

/// Test-build stubs: no webview windows exist without the full app (see
/// `create`), so showing fails and hiding is a no-op.
#[cfg(test)]
pub fn create(_app: &AppHandle, _color: &str) {}

#[cfg(test)]
pub fn show() -> Result<(), String> {
    Err("backdrop windows are not built in test binaries".into())
}

#[cfg(test)]
pub fn hide() {}

/// Whether the backdrop is currently shown (the journaled "on" flag).
pub fn shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

/// Hook-path entry (spec 7.9): re-drops every backdrop directly below the
/// newly foregrounded window. Delivered on the main thread; the idle path
/// here is one atomic load.
pub fn on_foreground(hwnd: isize) {
    if !SHOWN.load(Ordering::Relaxed) {
        return;
    }
    if !crate::front::backdrop_event_ok(hwnd) {
        return;
    }
    let snapshot: Vec<BackdropWindow> =
        { WINDOWS.lock().unwrap_or_else(|p| p.into_inner()).clone() };
    for w in snapshot {
        place_below(w.hwnd, hwnd);
    }
}

fn place_below(hwnd: isize, anchor: isize) {
    unsafe {
        let _ = SetWindowPos(
            HWND(hwnd as *mut core::ffi::c_void),
            Some(HWND(anchor as *mut core::ffi::c_void)),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_color_accepts_the_settings_shape() {
        assert_eq!(parse_hex_color("#202020"), Some((0x20, 0x20, 0x20)));
        assert_eq!(parse_hex_color("#FF8800"), Some((255, 136, 0)));
        assert_eq!(parse_hex_color("202020"), None, "missing #");
        assert_eq!(parse_hex_color("#20202"), None, "too short");
        assert_eq!(parse_hex_color("#2020200"), None, "too long");
        assert_eq!(parse_hex_color("#zzzzzz"), None, "not hex");
    }

    #[test]
    fn show_errors_when_windows_were_never_created() {
        // No setup() ran in the test process: APP is unset.
        assert!(show().is_err());
        assert!(!shown());
    }
}
