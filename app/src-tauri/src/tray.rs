//! Tray icon, menu and the 1 s ticker (spec 2.1, 7.7; FR-1, FR-13,
//! FR-48). D-086: "Check for updates" and "Copy diagnostic report" land
//! in Phase 6 with their features.

use std::sync::Mutex;
use std::time::Duration;

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON;

use crate::automation::{self, Action};
use crate::switches::SwitchId;
use crate::{SessionStatus, SharedState};

/// Managed handles the dispatcher needs to refresh the tray.
pub struct TrayHandles {
    pub icon: TrayIcon,
    pub start_restore_item: MenuItem<Wry>,
    /// One check item per switch, registry order (FR-48): the click
    /// dispatches a Switch toggle; `update_tray` re-syncs the check from
    /// the real state in case an apply failed.
    pub switch_items: Vec<(SwitchId, CheckMenuItem<Wry>)>,
}

pub fn build(app: &AppHandle) -> tauri::Result<TrayHandles> {
    let start_restore_item = MenuItem::with_id(
        app,
        "start-restore",
        "Start Recording Mode",
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    // One check item per switch, registry order = the engine's row order
    // (FR-48 says all switches, hidden-shelf ones included).
    let state = app.state::<SharedState>();
    let b = state.lock().unwrap_or_else(|p| p.into_inner());
    let rows = b.engine.switch_states();
    drop(b);
    let mut switch_items = Vec::with_capacity(rows.len());
    for row in &rows {
        let item = CheckMenuItem::with_id(
            app,
            format!("switch:{}", row.id),
            switch_display_name(row.id),
            true,
            row.on,
            None::<&str>,
        )?;
        switch_items.push((row.id, item));
    }

    let mut items: Vec<&dyn IsMenuItem<Wry>> = vec![&start_restore_item];
    let separator = PredefinedMenuItem::separator(app)?;
    let separator2 = PredefinedMenuItem::separator(app)?;
    if !switch_items.is_empty() {
        items.push(&separator);
    }
    for (_, item) in &switch_items {
        items.push(item);
    }
    if !switch_items.is_empty() {
        items.push(&separator2);
    }
    items.push(&settings);
    items.push(&quit);
    let menu = Menu::with_items(app, &items)?;

    let icon = TrayIconBuilder::with_id("main")
        // The menu must not also open on a left click (spec 2.5 instruction).
        .show_menu_on_left_click(false)
        .menu(&menu)
        .icon(idle_icon())
        .tooltip("button")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "start-restore" => {
                if let Err(message) = automation::run(app, Action::Toggle) {
                    automation::notify(app, &message.to_string());
                }
            }
            "settings" => crate::settings_window::open(app, None),
            "quit" => app.exit(0),
            id => {
                if let Some(switch_id) = id.strip_prefix("switch:") {
                    let Ok(switch_id) = automation::parse_switch_id(switch_id) else {
                        return;
                    };
                    if let Err(message) = automation::run(
                        app,
                        Action::Switch {
                            id: switch_id,
                            on: crate::automation::SwitchOnOff::Toggle,
                        },
                    ) {
                        automation::notify(app, &message.to_string());
                    }
                }
            }
        })
        .on_tray_icon_event(|tray, event| match event {
            // The dropdown's visibility is sampled on the press and acted on
            // on the release; the enum is non_exhaustive (spec 2.5).
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Down,
                ..
            } => left_click_pressed(tray.app_handle()),
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } => {
                if let Err(message) = left_click_released(tray.app_handle(), click_rect(&rect)) {
                    automation::notify(tray.app_handle(), &message);
                }
            }
            TrayIconEvent::Click {
                button: MouseButton::Middle,
                button_state: MouseButtonState::Up,
                ..
            } => {
                if let Err(message) = automation::run(tray.app_handle(), Action::Toggle) {
                    automation::notify(tray.app_handle(), &message.to_string());
                }
            }
            _ => {}
        })
        .build(app)?;
    Ok(TrayHandles {
        icon,
        start_restore_item,
        switch_items,
    })
}

/// The tray bitmap size the shell draws (D-097): SM_CXSMICON at the
/// system DPI, so 16 px at 100 % scale, 20 at 125 %, 24 at 150 %, 32 at
/// 200 %, 48 at 300 %, 64 at 400 %. Handing over that exact size keeps the
/// mark sharp instead of letting the shell resample one bitmap.
fn tray_icon_px() -> i32 {
    // SAFETY: two metric queries that take and return plain integers.
    unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }
}

/// The mark at the smallest pre-rendered size that covers the tray's.
pub fn idle_icon() -> tauri::image::Image<'static> {
    match tray_icon_px() {
        ..=16 => tauri::include_image!("icons/tray/idle-16.png"),
        17..=20 => tauri::include_image!("icons/tray/idle-20.png"),
        21..=24 => tauri::include_image!("icons/tray/idle-24.png"),
        25..=32 => tauri::include_image!("icons/tray/idle-32.png"),
        33..=40 => tauri::include_image!("icons/tray/idle-40.png"),
        41..=48 => tauri::include_image!("icons/tray/idle-48.png"),
        _ => tauri::include_image!("icons/tray/idle-64.png"),
    }
}

/// The tray menu's switch names (FR-48): the same display names as the
/// dropdown (FR-3's names, spec 7.5 catalogue, mirrored from
/// dropdown/main.ts LABELS).
fn switch_display_name(id: SwitchId) -> &'static str {
    match id {
        SwitchId::Dnd => "Do Not Disturb",
        SwitchId::DesktopIcons => "Hide Desktop Icons",
        SwitchId::OtherApps => "Hide Other Apps",
        SwitchId::Taskbar => "Hide Taskbar",
        SwitchId::TrayIcons => "Hide Tray Icons",
        SwitchId::Backdrop => "Backdrop",
        SwitchId::KeepAwake => "Keep Awake",
        SwitchId::AlertSounds => "Mute Alert Sounds",
        SwitchId::DarkMode => "Dark Mode",
        SwitchId::NightLight => "Night Light",
        SwitchId::ScreenSaver => "Screen Saver",
        SwitchId::Microphone => "Mute Microphone",
        SwitchId::LockKeyboard => "Lock Keyboard",
        SwitchId::Bluetooth => "Bluetooth Headphones",
    }
}

/// The red Recording Mode mark, sized like `idle_icon`.
pub fn active_icon() -> tauri::image::Image<'static> {
    match tray_icon_px() {
        ..=16 => tauri::include_image!("icons/tray/active-16.png"),
        17..=20 => tauri::include_image!("icons/tray/active-20.png"),
        21..=24 => tauri::include_image!("icons/tray/active-24.png"),
        25..=32 => tauri::include_image!("icons/tray/active-32.png"),
        33..=40 => tauri::include_image!("icons/tray/active-40.png"),
        41..=48 => tauri::include_image!("icons/tray/active-48.png"),
        _ => tauri::include_image!("icons/tray/active-64.png"),
    }
}

/// One refresh function after every state change (spec 2.5 instruction):
/// icon, tooltip ("button" idle, elapsed time active) and menu text.
pub fn update_tray(app: &AppHandle) {
    let Some(handles) = app.try_state::<TrayHandles>() else {
        return;
    };
    let state = app.state::<SharedState>();
    let b = state.lock().unwrap_or_else(|p| p.into_inner());
    let active = b.engine.state().status == SessionStatus::Active;
    let tooltip = if active {
        let started = b
            .engine
            .state()
            .started_at
            .expect("an active session has startedAt");
        let seconds = (time::OffsetDateTime::now_utc() - started).whole_seconds();
        format!("button - {}", format_elapsed(seconds))
    } else {
        "button".to_string()
    };
    drop(b);
    let _ = handles.icon.set_tooltip(Some(tooltip.as_str()));
    let _ = handles
        .icon
        .set_icon(Some(if active { active_icon() } else { idle_icon() }));
    let _ = handles.start_restore_item.set_text(if active {
        "Restore"
    } else {
        "Start Recording Mode"
    });
    // Re-sync the switch check items from the real state: a menu click
    // flips its own check, but a failed apply must not leave the menu
    // lying (FR-48 acceptance). Set only on change.
    let state = app.state::<SharedState>();
    let b = state.lock().unwrap_or_else(|p| p.into_inner());
    let rows = b.engine.switch_states();
    drop(b);
    for (id, item) in &handles.switch_items {
        let Some(row) = rows.iter().find(|r| r.id == *id) else {
            continue;
        };
        let on = row.on;
        let enabled = row.availability == crate::switches::Availability::Available;
        if item.is_checked().unwrap_or(!on) != on {
            let _ = item.set_checked(on);
        }
        if item.is_enabled().unwrap_or(!enabled) != enabled {
            let _ = item.set_enabled(enabled);
        }
    }
}

/// Tooltip time: m:ss under an hour, h:mm:ss from an hour on (FR-13).
pub fn format_elapsed(total_seconds: i64) -> String {
    let s = total_seconds.max(0);
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// The session whose "1 minute left" notice already ran (D-090), keyed by
/// startedAt so a new session re-arms it. `extend_recording` clears it.
static LIMIT_NOTICE_GIVEN: Mutex<Option<i64>> = Mutex::new(None);

/// Pure part of the ticker's time-limit handling (FR-42, D-090;
/// unit-tested): `(due, "1 minute left" due, session identity)` for an
/// active session with a deadline, None otherwise. A deadline in the
/// past is due; the notice covers the last 60 s.
fn limit_decision(
    status: SessionStatus,
    ends_at: Option<time::OffsetDateTime>,
    started_at: Option<time::OffsetDateTime>,
    now: time::OffsetDateTime,
) -> Option<(bool, bool, i64)> {
    if status != SessionStatus::Active {
        return None;
    }
    let ends = ends_at?;
    let started = started_at.map(|t| t.unix_timestamp()).unwrap_or_default();
    let remaining = (ends - now).whole_seconds();
    Some((now >= ends, remaining > 0 && remaining <= 60, started))
}

/// Re-arm the "1 minute left" notice (D-090): an extended session warns
/// again before its new end.
pub fn rearm_limit_notice() {
    *LIMIT_NOTICE_GIVEN.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

/// The 1 s ticker: a plain std::thread with a 1 s sleep (no tokio dep). It
/// only works while a session is active or a timer is pending (spec 7.7,
/// §10); timers fire here (FR-28), and so does a due time limit (FR-42).
/// The emit uses the engine's row cache, so the ticker itself never reads
/// a switch.
pub fn start_ticker(app: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let state = app.state::<SharedState>();
            let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
            let active = b.engine.state().status == SessionStatus::Active;
            let timers = b.engine.has_timers();
            if active || timers {
                // fire_due_timers runs OS restores under the SharedState
                // lock: a slow restore (Bluetooth KsProperty) briefly
                // blocks IPC/tray. Deliberate: start/stop/set_switch hold
                // the same lock across their OS work, and the per-entry
                // restore+journal-write interleaving (spec 7.2) assumes the
                // engine is locked. Latency-only, no correctness impact.
                b.engine.fire_due_timers(time::OffsetDateTime::now_utc());
            }
            // FR-42: a due time limit stops the session. Read after the
            // timers so a due timer and the limit both get their turn.
            let now = time::OffsetDateTime::now_utc();
            let decision = {
                let session = b.engine.state();
                limit_decision(session.status, session.ends_at, session.started_at, now)
            };
            drop(b);
            let (due, almost_due, started) = decision.unwrap_or_default();
            if due {
                automation::notify(&app, "Time limit reached — everything is back");
                if let Err(message) = automation::run(&app, Action::Stop) {
                    automation::notify(&app, &message.to_string());
                }
            } else if almost_due {
                let mut given = LIMIT_NOTICE_GIVEN.lock().unwrap_or_else(|p| p.into_inner());
                if *given != Some(started) {
                    *given = Some(started);
                    drop(given);
                    automation::notify(&app, "1 minute left in Recording Mode");
                }
            }
            if active || timers {
                automation::after_change(&app);
            }
        }
    });
}

/// The tray event's rect in physical pixels, as position.rs wants it.
/// tray-icon builds it from `Shell_NotifyIconGetRect`, which is always
/// physical screen pixels, so `to_physical(1.0)` is a no-op on the
/// Physical variant (icons inside the hidden-icons flyout report a rect
/// with a different origin; the clamp keeps the window on-screen).
fn click_rect(rect: &tauri::Rect) -> crate::position::ClickRect {
    let pos = rect.position.to_physical::<f64>(1.0);
    let size = rect.size.to_physical::<f64>(1.0);
    crate::position::ScreenRect {
        x: pos.x as i32,
        y: pos.y as i32,
        w: size.width as i32,
        h: size.height as i32,
    }
}

/// Whether the dropdown was open when the current left press began; managed
/// via Tauri so the tray closures can share it.
type WasOpenAtDown = std::sync::Arc<std::sync::Mutex<bool>>;

/// Called on the left press: the blur that follows the press hides the
/// dropdown, so the visibility must be sampled here, before it happens.
pub fn left_click_pressed(app: &AppHandle) {
    let open = app
        .get_webview_window("dropdown")
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    if let Some(guard) = app.try_state::<WasOpenAtDown>() {
        *guard.inner().lock().unwrap_or_else(|p| p.into_inner()) = open;
    }
}

/// Called on the left release: close if it was open at the press, otherwise
/// show it next to the tray icon (spec 7.13).
pub fn left_click_released(
    app: &AppHandle,
    click: crate::position::ClickRect,
) -> Result<(), String> {
    let was_open = app
        .try_state::<WasOpenAtDown>()
        .is_some_and(|g| *g.inner().lock().unwrap_or_else(|p| p.into_inner()));
    if was_open {
        hide_dropdown(app);
        Ok(())
    } else {
        show_dropdown(app, Some(click))
    }
}

/// Show the dropdown, placing it next to the tray icon when the click
/// position is known (spec 7.13). `Action::ShowDropdown` is a toggle: a
/// second invocation while the dropdown is visible closes it again.
pub fn show_dropdown(
    app: &AppHandle,
    click: Option<crate::position::ClickRect>,
) -> Result<(), String> {
    let Some(window) = app.get_webview_window("dropdown") else {
        log::warn!("dropdown window not built yet");
        return Ok(());
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return Ok(());
    }
    // The list must be current the moment it shows (FR-27): refresh the row
    // cache AND push it — the dropdown window is long-lived, so its page
    // only updates on state-changed.
    automation::refresh_switches(app);
    if let Some(click) = click {
        place_at(&window, click);
    } else {
        // No click position (e.g. a second bare `button.exe` brought the
        // dropdown up): anchor it at the tray icon instead of the default
        // window position (spec 6.3, 7.13).
        if let Some(handles) = app.try_state::<TrayHandles>()
            && let Ok(Some(rect)) = handles.icon.rect()
        {
            place_at(&window, click_rect(&rect));
        }
    }
    let _ = window.show();
    let _ = window.set_focus();
    Ok(())
}

/// Called by the dispatcher after Recording Mode starts (spec 7.1.4).
pub fn hide_dropdown(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("dropdown") {
        let _ = window.hide();
    }
}

/// Resize the dropdown to `css_height` (its content, CSS px) in one
/// SetWindowPos, so a dropdown that grows upward never flashes at the
/// wrong place (see position::fit_height for which edge stays).
pub fn fit_dropdown(window: &tauri::WebviewWindow, css_height: f64) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos};

    let scale = window.scale_factor().unwrap_or(1.0);
    let (Ok(pos), Ok(outer), Ok(inner), Ok(hwnd)) = (
        window.outer_position(),
        window.outer_size(),
        window.inner_size(),
        window.hwnd(),
    ) else {
        return;
    };
    let frame = outer.height as i32 - inner.height as i32;
    let wanted = (css_height * scale).round() as i32 + frame.max(0);
    let win = crate::position::ScreenRect {
        x: pos.x,
        y: pos.y,
        w: outer.width as i32,
        h: outer.height as i32,
    };
    let Some((_, work)) = crate::position::monitor_work_area_at(win.center_x(), win.center_y())
    else {
        return;
    };
    let (y, h) = crate::position::fit_height(&win, &work, wanted);
    if y == win.y && h == win.h {
        return;
    }
    unsafe {
        let _ = SetWindowPos(
            HWND(hwnd.0),
            None,
            win.x,
            y,
            win.w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Position the window next to the tray icon's monitor work area.
fn place_at(window: &tauri::WebviewWindow, click: crate::position::ClickRect) {
    let Some((monitor, work)) =
        crate::position::monitor_work_area_at(click.x + click.w / 2, click.y + click.h / 2)
    else {
        return;
    };
    let size = window.outer_size().unwrap_or_default();
    let (x, y) = crate::position::place(
        &click,
        &monitor,
        &work,
        (size.width as i32, size.height as i32),
    );
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

/// Wire blur-close on the dropdown window: hide whenever another window
/// takes the focus. The tray press handler samples visibility before this
/// can fire, so the tray click still closes a dropdown it intentionally hid.
pub fn attach_blur_close(window: &tauri::WebviewWindow) {
    let handle = window.clone();
    window
        .app_handle()
        .manage::<WasOpenAtDown>(std::sync::Arc::new(std::sync::Mutex::new(false)));
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event {
            // The webview also reports a focus loss when the page starts a
            // window drag (the OS move loop takes the focus from it), but
            // the dropdown is still the foreground window then.
            if is_foreground(&handle) {
                return;
            }
            let _ = handle.hide();
        }
    });
}

fn is_foreground(window: &tauri::WebviewWindow) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let Ok(hwnd) = window.hwnd() else {
        return false;
    };
    unsafe { GetForegroundWindow().0 == hwnd.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::switches::{SwitchId, build_registry};

    #[test]
    fn elapsed_format_matches_the_ui_helper() {
        assert_eq!(format_elapsed(0), "0:00");
        assert_eq!(format_elapsed(59), "0:59");
        assert_eq!(format_elapsed(60), "1:00");
        assert_eq!(format_elapsed(3599), "59:59");
        assert_eq!(format_elapsed(3600), "1:00:00");
        assert_eq!(format_elapsed(3661), "1:01:01");
        assert_eq!(format_elapsed(-3), "0:00");
    }

    #[test]
    fn test_registry_has_every_switch() {
        let dir = std::env::temp_dir().join(format!("button-registry-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let registry = build_registry(&dir);
        let ids: Vec<SwitchId> = registry.iter().map(|s| s.id()).collect();
        assert_eq!(
            ids,
            vec![
                SwitchId::AlertSounds,
                SwitchId::DesktopIcons,
                SwitchId::OtherApps,
                SwitchId::Backdrop,
                SwitchId::Taskbar,
                SwitchId::TrayIcons,
                SwitchId::KeepAwake,
                SwitchId::DarkMode,
                SwitchId::Dnd,
                SwitchId::LockKeyboard,
                SwitchId::Microphone,
                SwitchId::NightLight,
                SwitchId::ScreenSaver,
                SwitchId::Bluetooth,
            ]
        );
        // Spec 7.3 persistence: every effect that outlives button's
        // process is restored by crash recovery — Explorer state
        // (desktop icons, taskbar auto-hide, tray-icon promotion, dark
        // mode, DND profile, Night Light blob, Bluetooth link), minimized
        // windows (other-apps) and audio-service mutes (alert-sounds,
        // microphone). Process-owned state (keep-awake power request,
        // backdrop and lock windows) dies with the process and is dropped.
        // Screen-saver is an action and never journaled.
        let persistent: Vec<SwitchId> = registry
            .iter()
            .filter(|s| s.persistent())
            .map(|s| s.id())
            .collect();
        assert_eq!(
            persistent,
            vec![
                SwitchId::AlertSounds,
                SwitchId::DesktopIcons,
                SwitchId::OtherApps,
                SwitchId::Taskbar,
                SwitchId::TrayIcons,
                SwitchId::DarkMode,
                SwitchId::Dnd,
                SwitchId::Microphone,
                SwitchId::NightLight,
                SwitchId::Bluetooth,
            ]
        );
        let experimental: Vec<SwitchId> = registry
            .iter()
            .filter(|s| s.experimental())
            .map(|s| s.id())
            .collect();
        assert_eq!(
            experimental,
            vec![SwitchId::Dnd, SwitchId::NightLight, SwitchId::Bluetooth],
            "D-013/D-014: the undocumented trio"
        );
        let actions: Vec<SwitchId> = registry
            .iter()
            .filter(|s| s.is_action())
            .map(|s| s.id())
            .collect();
        assert_eq!(actions, vec![SwitchId::ScreenSaver]);
    }

    fn at(minutes_after_epoch: i64) -> time::OffsetDateTime {
        time::OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(minutes_after_epoch)
    }

    #[test]
    fn limit_decision_covers_due_and_the_last_minute() {
        // A 1-minute limit starting at t0: due at t0+1min, the notice is
        // immediately live (remaining <= 60 s).
        let start = at(0);
        let ends = at(1);
        assert_eq!(
            limit_decision(SessionStatus::Active, Some(ends), Some(start), at(0)),
            Some((false, true, 0))
        );
        // The notice fired; after extend the same session is outside the
        // last minute and re-arming is the caller's job.
        assert_eq!(
            limit_decision(SessionStatus::Active, Some(at(2)), Some(start), at(0)),
            Some((false, false, 0))
        );
        // Due: the deadline passed.
        assert_eq!(
            limit_decision(SessionStatus::Active, Some(ends), Some(start), at(2)),
            Some((true, false, 0))
        );
    }

    #[test]
    fn limit_decision_needs_an_active_session_and_a_deadline() {
        assert_eq!(limit_decision(SessionStatus::Idle, None, None, at(0)), None);
        assert_eq!(
            limit_decision(SessionStatus::Active, None, Some(at(0)), at(0)),
            None
        );
        assert_eq!(
            limit_decision(SessionStatus::Countdown, Some(at(1)), Some(at(0)), at(0)),
            None
        );
    }

    #[test]
    fn limit_decision_keys_the_notice_on_the_session_start() {
        // `at()` is minutes since the epoch, so startedAt = 30 min = 1800 s.
        let decision = limit_decision(SessionStatus::Active, Some(at(61)), Some(at(30)), at(0));
        assert_eq!(decision, Some((false, false, 1800)));
    }
}
