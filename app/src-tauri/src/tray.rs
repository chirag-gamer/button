//! Tray icon, menu and the 1 s ticker (spec 2.1, 7.7; FR-1, FR-13).

use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::automation::{self, Action};
use crate::{SessionStatus, SharedState};

/// Managed handles the dispatcher needs to refresh the tray.
pub struct TrayHandles {
    pub icon: TrayIcon,
    pub start_restore_item: MenuItem<Wry>,
}

pub fn build(app: &AppHandle) -> tauri::Result<TrayHandles> {
    let start_restore_item = MenuItem::with_id(
        app,
        "start-restore",
        "Start Recording Mode",
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", "Settings", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&start_restore_item, &settings, &quit])?;

    let icon = TrayIconBuilder::with_id("main")
        // The menu must not also open on a left click (spec 2.5 instruction).
        .show_menu_on_left_click(false)
        .menu(&menu)
        .icon(idle_icon())
        .tooltip("button")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "start-restore" => {
                if let Err(message) = automation::run(app, Action::Toggle) {
                    automation::notify(app, &message);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            // Act only on the release of a click; the enum is non_exhaustive
            // (spec 2.5 instruction).
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } => {
                if let Err(message) = automation::run(
                    tray.app_handle(),
                    Action::ShowDropdown {
                        click: Some(click_rect(&rect)),
                    },
                ) {
                    automation::notify(tray.app_handle(), &message);
                }
            }
            TrayIconEvent::Click {
                button: MouseButton::Middle,
                button_state: MouseButtonState::Up,
                ..
            } => {
                if let Err(message) = automation::run(tray.app_handle(), Action::Toggle) {
                    automation::notify(tray.app_handle(), &message);
                }
            }
            _ => {}
        })
        .build(app)?;
    Ok(TrayHandles {
        icon,
        start_restore_item,
    })
}

pub fn idle_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("icons/tray-idle.png")
}

pub fn active_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("icons/tray-active.png")
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

/// The 1 s ticker: a plain std::thread with a 1 s sleep (no tokio dep). It
/// only works while a session is active (spec 7.7, 10); timers fire here
/// from Phase 4 on.
pub fn start_ticker(app: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let state = app.state::<SharedState>();
            let b = state.lock().unwrap_or_else(|p| p.into_inner());
            let active = b.engine.state().status == SessionStatus::Active;
            drop(b);
            if active {
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

/// Toggle the dropdown window, placing it next to the tray icon when the
/// click position is known (spec 7.13).
///
/// Blur-close + the 250 ms guard: when the dropdown is open, clicking the
/// tray icon first blurs it (auto-hide), then the toggle would show it
/// again. The guard treats a recent auto-hide as "the user just closed it",
/// so the tray click closes instead of re-opening.
pub fn toggle_dropdown(app: &AppHandle, click: Option<crate::position::ClickRect>) {
    let Some(window) = app.get_webview_window("dropdown") else {
        log::warn!("dropdown window not built yet");
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    let guard = app.state::<AutoHide>().inner().clone();
    if guard
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
    {
        return;
    }
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
    *guard.lock().unwrap_or_else(|p| p.into_inner()) = None;
    let _ = window.show();
    let _ = window.set_focus();
}

/// Called by the dispatcher after Recording Mode starts (spec 7.1.4).
pub fn hide_dropdown(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("dropdown") {
        let _ = window.hide();
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

type AutoHide = std::sync::Arc<std::sync::Mutex<Option<std::time::Instant>>>;

/// Wire blur-close on the dropdown window: hide when it loses focus and
/// remember when (for the 250 ms tray-click guard).
pub fn attach_blur_close(window: &tauri::WebviewWindow) {
    let handle = window.clone();
    let guard: AutoHide = std::sync::Arc::new(std::sync::Mutex::new(None));
    window.app_handle().manage(guard.clone());
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event {
            *guard.lock().unwrap_or_else(|p| p.into_inner()) = Some(std::time::Instant::now());
            let _ = handle.hide();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::format_elapsed;
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
    fn debug_registry_has_two_persistent_fakes_and_keep_awake() {
        let dir = std::env::temp_dir().join(format!("button-registry-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let registry = build_registry(&dir);
        let ids: Vec<SwitchId> = registry.iter().map(|s| s.id()).collect();
        assert_eq!(
            ids,
            vec![
                SwitchId::DesktopIcons,
                SwitchId::Taskbar,
                SwitchId::KeepAwake
            ]
        );
        // D-063: desktop icons and taskbar persist across restarts via their
        // fake state file; keep-awake is memory-only.
        let persistent: Vec<SwitchId> = registry
            .iter()
            .filter(|s| s.persistent())
            .map(|s| s.id())
            .collect();
        assert_eq!(persistent, vec![SwitchId::DesktopIcons, SwitchId::Taskbar]);
    }
}
