//! The 3·2·1 countdown overlay (FR-41): a small always-on-top window on
//! the front app's monitor, opened by automation's countdown thread and
//! closed when it finishes or the countdown is cancelled. The HTML starts
//! at "3" so a missed first event cannot flash an empty overlay.

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

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
    // Window labels carry the generation, so a label can never repeat
    // (generations only grow): no leftover can collide with this build.
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
        .on_page_load(move |window, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished
                && crate::automation::countdown_generation() == generation
            {
                let tick = crate::automation::current_countdown_tick();
                let _ = window.emit("countdown", serde_json::json!({ "secondsLeft": tick }));
            }
        })
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
        // Positioning runs on the main thread: primary_monitor is a
        // blocking getter off it (the same rule backdrop::show follows).
        let app3 = app2.clone();
        let win = window.clone();
        let _ = app2.run_on_main_thread(move || {
            if crate::automation::countdown_generation() != generation {
                let _ = win.close();
                return;
            }
            if let Some((x, y)) = centred_position(&app3, &win) {
                let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
            }
        });
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

/// Window top-left so the overlay sits centred on the monitor the front app
/// is on, else the primary monitor. Uses real physical outer size from the
/// window handle to account for display scaling (FR-41). Runs on the main
/// thread only (primary_monitor is a blocking getter elsewhere).
#[cfg(not(test))]
fn centred_position(app: &AppHandle, window: &tauri::WebviewWindow) -> Option<(i32, i32)> {
    let size = window.outer_size().unwrap_or_default();
    let width = if size.width > 0 {
        size.width as i32
    } else {
        240
    };
    let height = if size.height > 0 {
        size.height as i32
    } else {
        240
    };
    let monitor_rect = front_monitor_rect(app).or_else(|| primary_rect(app))?;
    let centre_x = monitor_rect.0 + monitor_rect.2 as i32 / 2;
    let centre_y = monitor_rect.1 + monitor_rect.3 as i32 / 2;
    Some((centre_x - width / 2, centre_y - height / 2))
}

#[cfg(test)]
fn centred_position(_app: &AppHandle, _window: &tauri::WebviewWindow) -> Option<(i32, i32)> {
    None
}

/// The front app's monitor bounds, else the primary monitor's.
#[cfg(all(not(test), windows))]
fn front_monitor_rect(_app: &AppHandle) -> Option<(i32, i32, u32, u32)> {
    let hwnd = crate::front::backdrop_anchor()?;
    crate::backdrop::monitor_rect_of(hwnd)
}

#[cfg(all(not(test), windows))]
fn primary_rect(app: &AppHandle) -> Option<(i32, i32, u32, u32)> {
    app.primary_monitor()
        .ok()
        .flatten()
        .map(|m| crate::backdrop::monitor_rect(&m))
}

#[cfg(all(not(test), not(windows)))]
fn front_monitor_rect(_app: &AppHandle) -> Option<(i32, i32, u32, u32)> {
    None
}

#[cfg(all(not(test), not(windows)))]
fn primary_rect(_app: &AppHandle) -> Option<(i32, i32, u32, u32)> {
    None
}
