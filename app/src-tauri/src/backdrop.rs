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

/// Mirror of `settings.backdrop.monitors` (0 = all, 1 = front). Written
/// by `cache_monitors` (startup and every settings commit) and read by
/// `show()`/`on_foreground()`, which run under the shared-state lock or
/// the foreground hook and must not take the state lock themselves.
static MONITORS_FRONT: AtomicBool = AtomicBool::new(false);

/// Refresh the monitors mirror from a settings snapshot.
pub fn cache_monitors(setting: crate::settings::BackdropMonitors) {
    MONITORS_FRONT.store(
        matches!(setting, crate::settings::BackdropMonitors::Front),
        Ordering::Relaxed,
    );
}

/// The cached monitor routing.
fn monitors_setting() -> crate::settings::BackdropMonitors {
    if MONITORS_FRONT.load(Ordering::Relaxed) {
        crate::settings::BackdropMonitors::Front
    } else {
        crate::settings::BackdropMonitors::All
    }
}

#[derive(Clone)]
struct BackdropWindow {
    #[cfg_attr(test, allow(dead_code))]
    label: String,
    hwnd: isize,
    /// The monitor's name at creation, so `show` can re-match after
    /// monitors are added, removed or reordered (falls back to the
    /// creation index when the monitor reports no name).
    #[cfg_attr(test, allow(dead_code))]
    monitor: Option<String>,
    /// The monitor's index in the launch enumeration — the unnamed
    /// fallback. Stored, not derived from the snapshot position: earlier
    /// windows that failed to build would shift the compacted indices.
    #[cfg_attr(test, allow(dead_code))]
    monitor_index: usize,
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
                        monitor: monitor.name().cloned(),
                        monitor_index: i,
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
///
/// Requires a front anchor: without one the backdrop would land on
/// HWND_TOP and cover the screen, so a missing anchor fails the switch
/// instead (the session keeps going without the backdrop, FR-7). A
/// window whose z-order change fails is hidden again right away for the
/// same reason.
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
    let anchor = crate::front::backdrop_anchor()
        .ok_or("no front app to anchor the backdrop; not covering the screen")?;
    // The engine holds the shared-state lock when it calls apply(); show()
    // must never take that lock (non-reentrant deadlock). The setting is
    // mirrored into an atomic by commit_shared / startup instead.
    let setting = monitors_setting();
    // FR-46/D-049: with monitors = front, only the front app's monitor is
    // covered (primary when no front app is tracked).
    let front_rect = match setting {
        crate::settings::BackdropMonitors::Front => monitor_rect_of(anchor),
        _ => None,
    };
    let app2 = app.clone();
    app.run_on_main_thread(move || {
        // Monitor getters are main-thread round trips; they must run
        // inside this closure (inline on the main thread), never outside
        // it — show() runs under the shared-state lock, and a blocking
        // getter here could deadlock against main-thread handlers that
        // want the same lock (tray menu, onboarding destroy).
        let monitors = app2.available_monitors().ok();
        let primary_rect = app2
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| monitor_rect(&m));
        for w in snapshot.iter() {
            let Some(window) = app2.get_webview_window(&w.label) else {
                continue;
            };
            let mut show_window = setting != crate::settings::BackdropMonitors::Front;
            let window_monitor = monitor_rect_of(w.hwnd);
            let effective_primary = primary_rect.or(window_monitor);
            if let Some(list) = monitors.as_ref() {
                match monitor_for(list, w) {
                    // Fresh bounds for the monitor this window was
                    // created for.
                    Some(monitor) => {
                        let _ = window.set_position(*monitor.position());
                        let _ = window.set_size(*monitor.size());
                        if let Some(primary) = effective_primary {
                            show_window = shows_on_monitor(
                                setting,
                                front_rect,
                                primary,
                                monitor_rect(monitor),
                            );
                        }
                    }
                    // The window's monitor is gone since launch: showing
                    // it at stale bounds would put a dark rectangle
                    // nowhere useful, so it stays hidden.
                    None if w.monitor.is_some() => {
                        let _ = window.hide();
                        continue;
                    }
                    // Unnamed monitor: route with available HWND bounds.
                    None => {
                        if let (Some(primary), Some(m)) = (effective_primary, window_monitor) {
                            show_window = shows_on_monitor(setting, front_rect, primary, m);
                        }
                    }
                }
            } else if let (Some(primary), Some(m)) = (effective_primary, window_monitor) {
                show_window = shows_on_monitor(setting, front_rect, primary, m);
            }
            if show_window {
                let _ = window.show();
                if place_below(w.hwnd, anchor).is_err() {
                    // The anchor died between the pick and this call: without
                    // the z-order fix the backdrop would sit on top.
                    let _ = window.hide();
                }
            } else {
                let _ = window.hide();
            }
        }
    })
    .map_err(|e| format!("backdrop show failed: {e}"))?;
    SHOWN.store(true, Ordering::Relaxed);
    Ok(())
}

/// Physical bounds of a Tauri monitor as a comparable tuple.
#[cfg(not(test))]
pub(crate) fn monitor_rect(monitor: &tauri::Monitor) -> MonitorRect {
    (
        monitor.position().x,
        monitor.position().y,
        monitor.size().width,
        monitor.size().height,
    )
}

/// The physical bounds of the monitor containing `hwnd` (FR-46 front
/// routing and the countdown's centring), via MonitorFromWindow.
#[cfg(all(not(test), windows))]
pub(crate) fn monitor_rect_of(hwnd: isize) -> Option<MonitorRect> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    let hmon = unsafe {
        MonitorFromWindow(
            HWND(hwnd as *mut core::ffi::c_void),
            MONITOR_DEFAULTTONEAREST,
        )
    };
    if hmon.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
        let RECT {
            left,
            top,
            right,
            bottom,
        } = info.rcMonitor;
        Some((left, top, (right - left) as u32, (bottom - top) as u32))
    } else {
        None
    }
}

#[cfg(all(not(test), not(windows)))]
fn monitor_rect_of(_hwnd: isize) -> Option<MonitorRect> {
    None
}

/// The monitor a backdrop window belongs to: by stored name when the
/// monitor reports one, otherwise by the monitor's creation index
/// (matching by index alone breaks when monitors are added, removed or
/// reordered after launch).
#[cfg(not(test))]
fn monitor_for<'m>(
    monitors: &'m [tauri::Monitor],
    window: &BackdropWindow,
) -> Option<&'m tauri::Monitor> {
    if let Some(name) = &window.monitor {
        return monitors
            .iter()
            .find(|m| m.name().is_some_and(|n| n == name));
    }
    monitors.get(window.monitor_index)
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

/// Applies a saved backdrop change (FR-46): the colour layer stays the
/// fallback under every style, and a reload makes the page re-read the
/// style and render gradients, pictures and the wallpaper (spec 7.9).
#[cfg(not(test))]
pub fn restyle(app: &AppHandle, backdrop: &crate::settings::Backdrop) {
    // Keep the atomic mirror in sync with the committed settings.
    crate::backdrop::cache_monitors(backdrop.monitors);
    let (r, g, b) = parse_hex_color(&backdrop.color).unwrap_or_else(|| {
        log::warn!(
            "backdrop: unparsable colour {:?}; using the default",
            backdrop.color
        );
        DEFAULT_COLOR
    });
    let snapshot: Vec<BackdropWindow> = {
        let windows = WINDOWS.lock().unwrap_or_else(|p| p.into_inner());
        windows.clone()
    };
    if snapshot.is_empty() {
        return;
    }
    let setting = backdrop.monitors;
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let is_shown = SHOWN.load(Ordering::Relaxed);
        let monitors = app2.available_monitors().ok();
        let primary_rect = app2
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| monitor_rect(&m));
        let anchor = if is_shown {
            crate::front::backdrop_anchor()
        } else {
            None
        };
        let front_rect = anchor.and_then(monitor_rect_of);
        for w in snapshot.iter() {
            if let Some(window) = app2.get_webview_window(&w.label) {
                let _ = window.set_background_color(Some(tauri::window::Color(r, g, b, 255)));
                let _ = window.reload();
                if is_shown {
                    let mut show_window = setting != crate::settings::BackdropMonitors::Front;
                    let window_monitor = monitor_rect_of(w.hwnd);
                    let effective_primary = primary_rect.or(window_monitor);
                    if let Some(list) = monitors.as_ref() {
                        match monitor_for(list, w) {
                            Some(monitor) => {
                                let _ = window.set_position(*monitor.position());
                                let _ = window.set_size(*monitor.size());
                                if let Some(primary) = effective_primary {
                                    show_window = shows_on_monitor(
                                        setting,
                                        front_rect,
                                        primary,
                                        monitor_rect(monitor),
                                    );
                                }
                            }
                            None if w.monitor.is_some() => {
                                let _ = window.hide();
                                continue;
                            }
                            None => {
                                if let (Some(primary), Some(m)) =
                                    (effective_primary, window_monitor)
                                {
                                    show_window = shows_on_monitor(setting, front_rect, primary, m);
                                }
                            }
                        }
                    } else if let (Some(primary), Some(m)) = (effective_primary, window_monitor) {
                        show_window = shows_on_monitor(setting, front_rect, primary, m);
                    }
                    if show_window {
                        if let Some(a) = anchor {
                            let _ = window.show();
                            if place_below(w.hwnd, a).is_err() {
                                let _ = window.hide();
                            }
                        } else {
                            let _ = window.hide();
                        }
                    } else {
                        let _ = window.hide();
                    }
                }
            }
        }
    });
}

#[cfg(test)]
pub fn restyle(_app: &AppHandle, backdrop: &crate::settings::Backdrop) {
    // The real restyle updates the mirror; tests must stay consistent.
    crate::backdrop::cache_monitors(backdrop.monitors);
}

/// Monitor bounds as a comparable tuple (physical px).
#[cfg_attr(test, allow(dead_code))] // the caller (show) is stubbed in tests
pub type MonitorRect = (i32, i32, u32, u32);

/// FR-46/D-049 monitor routing (pure, unit-tested): does this backdrop
/// window's monitor show the backdrop? With `front` absent (no front app
/// tracked) the front-only mode falls back to the primary monitor.
pub fn shows_on_monitor(
    setting: crate::settings::BackdropMonitors,
    front: Option<MonitorRect>,
    primary: MonitorRect,
    window: MonitorRect,
) -> bool {
    match setting {
        crate::settings::BackdropMonitors::All => true,
        crate::settings::BackdropMonitors::Front => front.unwrap_or(primary) == window,
    }
}

/// Magic-byte sniff for the backdrop image (FR-46): PNG, JPEG, BMP, WebP.
/// Win11's TranscodedWallpaper has no extension, so the content decides.
pub fn sniff_image(bytes: &[u8]) -> bool {
    let png = b"\x89PNG\r\n\x1a\n";
    let webp = b"WEBP";
    bytes.starts_with(png)
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(&[0x42, 0x4D])
        || (bytes.len() >= 12 && &bytes[8..12] == webp)
}

/// The current desktop wallpaper path (D-087), via SPI_GETDESKWALLPAPER.
/// One wallpaper for all monitors (no IDesktopWallpaper). None when the
/// call fails or the path is empty (solid colour or a slideshow the API
/// does not resolve).
#[cfg(windows)]
pub fn wallpaper_path() -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    // The API writes a NUL-terminated path; size it beyond MAX_PATH for
    // long paths.
    let mut buffer = vec![0u16; 520];
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buffer.len() as u32,
            Some(buffer.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if !ok.is_ok() {
        return None;
    }
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(0);
    if len == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..len]))
}

#[cfg(not(windows))]
pub fn wallpaper_path() -> Option<String> {
    None
}

/// The image bytes for the backdrop page: the picture path or the current
/// wallpaper, sniffed and capped at ~50 MB. Empty when there is nothing
/// usable (missing, unsupported, empty) so the page falls back to the
/// colour. Logs never contain the path (spec 8).
#[cfg(not(test))]
pub fn image_bytes(app: &AppHandle) -> Vec<u8> {
    const MAX_IMAGE_BYTES: u64 = 50 * 1024 * 1024;
    let path = {
        let state = app.state::<crate::SharedState>();
        let b = state.lock().unwrap_or_else(|p| p.into_inner());
        match b.settings.backdrop.style {
            crate::settings::BackdropStyle::Picture => b.settings.backdrop.picture.clone(),
            crate::settings::BackdropStyle::Wallpaper => wallpaper_path(),
            crate::settings::BackdropStyle::Color | crate::settings::BackdropStyle::Gradient => {
                None
            }
        }
    };
    let Some(path) = path else {
        return Vec::new();
    };
    let Ok(meta) = std::fs::metadata(&path) else {
        log::warn!("backdrop image is missing; falling back to the colour");
        return Vec::new();
    };
    if meta.len() > MAX_IMAGE_BYTES {
        log::warn!("backdrop image is over the 50 MB cap; falling back to the colour");
        return Vec::new();
    }
    match std::fs::read(&path) {
        Ok(bytes) if sniff_image(&bytes) => bytes,
        Ok(_) => {
            log::warn!("backdrop image is not a supported format; falling back to the colour");
            Vec::new()
        }
        Err(_) => {
            log::warn!("backdrop image is unreadable; falling back to the colour");
            Vec::new()
        }
    }
}

#[cfg(test)]
pub fn image_bytes(_app: &AppHandle) -> Vec<u8> {
    Vec::new()
}

/// Whether the backdrop is currently shown (the journaled "on" flag).
pub fn shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

/// Hook-path entry (spec 7.9): re-drops every backdrop directly below the
/// newly foregrounded window. With monitors = front (FR-46/D-049) it also
/// moves the backdrop when the front app changed monitors: hide the old
/// monitor's window, show the new one. Delivered on the main thread; the
/// idle path here is one atomic load.
pub fn on_foreground(hwnd: isize) {
    if !SHOWN.load(Ordering::Relaxed) {
        return;
    }
    if !crate::front::backdrop_event_ok(hwnd) {
        return;
    }
    let snapshot: Vec<BackdropWindow> =
        { WINDOWS.lock().unwrap_or_else(|p| p.into_inner()).clone() };
    // Hook path: no app state, no blocking (front.rs, spec 7.9) — the
    // setting comes from the atomic mirror, never the state lock.
    // Under test the only consumer is compiled out (D-074).
    #[cfg_attr(test, allow(unused_variables))]
    let setting = monitors_setting();
    #[cfg(not(test))]
    if setting == crate::settings::BackdropMonitors::Front {
        let Some(app) = APP.get() else {
            return;
        };
        let front = monitor_rect_of(hwnd);
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            // Same rule as show(): monitor getters live inside the
            // main-thread closure.
            let primary = app2
                .primary_monitor()
                .ok()
                .flatten()
                .map(|m| monitor_rect(&m));
            for w in snapshot {
                let Some(window) = app2.get_webview_window(&w.label) else {
                    continue;
                };
                let window_monitor = monitor_rect_of(w.hwnd);
                let show_window = match (&front, primary) {
                    (Some(front), Some(primary)) => shows_on_monitor(
                        setting,
                        Some(*front),
                        primary,
                        window_monitor.unwrap_or(primary),
                    ),
                    // No front monitor and no primary: leave visibility
                    // as is.
                    _ => window.is_visible().unwrap_or(false),
                };
                if show_window {
                    let _ = window.show();
                } else {
                    let _ = window.hide();
                }
                // A failure here is benign: the backdrop sits somewhere
                // below the new foreground either way, because the
                // foreground window rises above everything.
                let _ = place_below(w.hwnd, hwnd);
            }
        });
        return;
    }
    for w in snapshot {
        // A failure here is benign: the backdrop sits somewhere below the
        // new foreground either way, because the foreground window rises
        // above everything.
        let _ = place_below(w.hwnd, hwnd);
    }
}

fn place_below(hwnd: isize, anchor: isize) -> windows::core::Result<()> {
    unsafe {
        SetWindowPos(
            HWND(hwnd as *mut core::ffi::c_void),
            Some(HWND(anchor as *mut core::ffi::c_void)),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
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

    #[test]
    fn shows_on_monitor_routes_all_and_front() {
        let primary: MonitorRect = (0, 0, 1920, 1080);
        let other: MonitorRect = (-1920, 0, 1920, 1080);
        // "all" covers every monitor.
        assert!(shows_on_monitor(
            crate::settings::BackdropMonitors::All,
            None,
            primary,
            other
        ));
        // "front": only the front app's monitor.
        assert!(shows_on_monitor(
            crate::settings::BackdropMonitors::Front,
            Some(other),
            primary,
            other
        ));
        assert!(!shows_on_monitor(
            crate::settings::BackdropMonitors::Front,
            Some(other),
            primary,
            primary
        ));
        // No front app tracked: the primary is covered.
        assert!(shows_on_monitor(
            crate::settings::BackdropMonitors::Front,
            None,
            primary,
            primary
        ));
        assert!(!shows_on_monitor(
            crate::settings::BackdropMonitors::Front,
            None,
            primary,
            other
        ));
    }

    #[test]
    fn sniff_image_accepts_the_four_supported_headers() {
        assert!(sniff_image(b"\x89PNG\r\n\x1a\n...."));
        assert!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(sniff_image(b"BM......"));
        assert!(sniff_image(b"RIFFxxxxWEBPVP8 "));
        assert!(!sniff_image(b"GIF89a"));
        assert!(!sniff_image(b""));
        assert!(!sniff_image(b"WEB")); // too short for the WebP box
        assert!(!sniff_image(b"....WEBP...")); // WebP marker not at offset 8
    }

    #[test]
    #[cfg(windows)]
    fn wallpaper_path_is_a_read_only_probe() {
        // Solid colour / slideshow setups resolve to None; otherwise a
        // non-empty path. Either way the FFI call must not panic.
        if let Some(path) = wallpaper_path() {
            assert!(!path.is_empty());
        }
    }
}
