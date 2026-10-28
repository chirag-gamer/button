//! The 3·2·1 countdown overlay (FR-41): a small always-on-top window on
//! the front app's monitor, opened by automation's countdown thread and
//! closed when it finishes or the countdown is cancelled. The HTML starts
//! at "3" so a missed first event cannot flash an empty overlay.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// The window label for a countdown generation. One label per countdown:
/// a stale thread can then only close its own window, never a successor's
/// (every trigger cancels by bumping the generation).
fn label(generation: u64) -> String {
    format!("countdown-{generation}")
}

/// Open the countdown window, centred on the front app's monitor (else
/// the primary). The builder runs on its own thread (D-071 deadlock
/// rule); the page must not take focus from the front app. `generation`
/// is the countdown generation this window belongs to: if the countdown
/// was cancelled while the window was still building, the window closes
/// itself right after the build instead of lingering as an orphan.
pub fn open(app: &AppHandle, generation: u64) {
    // Clear a leftover window first (e.g. a previous close lost the race
    // against a fast restart): the fixed-label era failed here with
    // WindowLabelAlreadyExists, which only logged.
    close(app, generation);
    let app2 = app.clone();
    std::thread::spawn(move || {
        let window = match WebviewWindowBuilder::new(
            &app2,
            label(generation),
            WebviewUrl::App("countdown.html".into()),
        )
        .inner_size(240.0, 240.0)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focusable(false)
        .resizable(false)
        .shadow(false)
        .content_protected(true)
        .build()
        {
            Ok(window) => window,
            Err(e) => {
                log::error!("countdown window could not be created: {e}");
                return;
            }
        };
        if crate::automation::countdown_generation() != generation {
            let _ = window.close();
            return;
        }
        if let Some((x, y)) = centred_position(&app2) {
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        }
    });
}

/// Close the countdown window of one generation if it exists; a missing
/// one is fine (the build thread may not have finished, or it already
/// closed itself).
pub fn close(app: &AppHandle, generation: u64) {
    if let Some(window) = app.get_webview_window(&label(generation)) {
        let _ = window.close();
    }
}

/// Window top-left so the 240×240 overlay sits centred on the monitor
/// the front app is on, else the primary monitor.
#[cfg(not(test))]
fn centred_position(app: &AppHandle) -> Option<(i32, i32)> {
    const SIZE: i32 = 240;
    let monitor_rect = front_monitor_rect(app)?;
    let centre_x = monitor_rect.0 + monitor_rect.2 as i32 / 2;
    let centre_y = monitor_rect.1 + monitor_rect.3 as i32 / 2;
    Some((centre_x - SIZE / 2, centre_y - SIZE / 2))
}

#[cfg(test)]
fn centred_position(_app: &AppHandle) -> Option<(i32, i32)> {
    None
}

/// The front app's monitor bounds, else the primary monitor's.
#[cfg(all(not(test), windows))]
fn front_monitor_rect(_app: &AppHandle) -> Option<(i32, i32, u32, u32)> {
    let hwnd = crate::front::backdrop_anchor()?;
    crate::backdrop::monitor_rect_of(hwnd)
}

#[cfg(all(not(test), not(windows)))]
fn front_monitor_rect(_app: &AppHandle) -> Option<(i32, i32, u32, u32)> {
    None
}
