//! IPC commands (spec 6.1). Recording Mode commands plus the Phase 4
//! quick-switch set: set_switch, unlock_keyboard, list_bt_devices,
//! set_bluetooth_device, set_keep_awake_options (D-078). Read-only
//! get_settings stays; save_settings arrives with the Settings UI (Phase 5).
//! Phase 6 adds check_update, install_update and copy_diagnostics.
//! Typed wrappers live in `app/src/shared/ipc.ts`.

use tauri::{AppHandle, Emitter, Manager, State};
use time::OffsetDateTime;

use crate::automation::{self, Action};
use crate::session::OffAfter;
use crate::settings::Settings;
use crate::switches::{SwitchError, SwitchId};
use crate::{AppState, SharedState, build_app_state};

/// Error shape for every failing command (spec 6.1).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "code", content = "message", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IpcError {
    SessionActive(String),
    NoSession(String),
    NotAvailable(String),
    InvalidArgument(String),
    OsError(String),
}

type IpcResult<T> = Result<T, IpcError>;

/// Maps a dispatcher failure to its IPC code (spec 6.1) by variant — the
/// old version lowercased and matched the error prose, so any wording
/// change silently demoted SESSION_ACTIVE/NO_SESSION to NOT_AVAILABLE.
fn map_error(e: automation::DispatchError) -> IpcError {
    match e {
        automation::DispatchError::SessionActive => IpcError::SessionActive(e.to_string()),
        automation::DispatchError::NoSession => IpcError::NoSession(e.to_string()),
        automation::DispatchError::Failed(message) => IpcError::NotAvailable(message),
    }
}

/// Maps a SwitchError to the spec 6.1 codes (task 4.2).
fn map_switch_error(e: SwitchError) -> IpcError {
    match e {
        SwitchError::NotAvailable(message) => IpcError::NotAvailable(message),
        SwitchError::Os { .. } => IpcError::OsError(e.to_string()),
        SwitchError::Invalid(message) => IpcError::InvalidArgument(message),
    }
}

fn current_state(state: &State<'_, SharedState>) -> AppState {
    let b = state.lock().unwrap_or_else(|p| p.into_inner());
    build_app_state(&b)
}

#[tauri::command]
pub fn get_state(state: State<'_, SharedState>) -> AppState {
    current_state(&state)
}

/// Read-only settings for the dropdown's shelves and the card count. The
/// save command arrives with the Settings UI (Phase 5).
#[tauri::command]
pub fn get_settings(state: State<'_, SharedState>) -> Settings {
    let b = state.lock().unwrap_or_else(|p| p.into_inner());
    b.settings.clone()
}

#[tauri::command]
pub fn start_recording(app: AppHandle) -> IpcResult<AppState> {
    dispatch(&app, Action::Start)?;
    Ok(current_state(&app.state::<SharedState>()))
}

#[tauri::command]
pub fn stop_recording(app: AppHandle) -> IpcResult<AppState> {
    dispatch(&app, Action::Stop)?;
    Ok(current_state(&app.state::<SharedState>()))
}

#[tauri::command]
pub fn toggle_recording(app: AppHandle) -> IpcResult<AppState> {
    dispatch(&app, Action::Toggle)?;
    Ok(current_state(&app.state::<SharedState>()))
}

/// Runs the action through the one dispatcher; failures map to IPC codes.
fn dispatch(app: &AppHandle, action: Action) -> IpcResult<()> {
    automation::run(app, action).map_err(map_error)
}

/// Turn one switch on or off, optionally with an off-timer (FR-27, FR-28).
#[tauri::command]
pub fn set_switch(
    app: AppHandle,
    state: State<'_, SharedState>,
    id: SwitchId,
    on: bool,
    off_after: Option<OffAfter>,
) -> IpcResult<AppState> {
    let now = OffsetDateTime::now_utc();
    let result = {
        let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
        let ctx = automation::manual_ctx(&b.settings);
        b.engine.set_switch(id, on, off_after, now, &ctx)
    };
    let outcome = result.map_err(map_switch_error);
    // Sync tray + UI on success and on error alike: the engine may have
    // changed state even when the turn failed (the dispatcher's rule).
    automation::after_change(&app);
    outcome?;
    Ok(current_state(&app.state::<SharedState>()))
}

/// The lock overlay's Unlock button (FR-34). The hook thread does the real
/// cleanup and pushes state-changed; this returns the state as it is.
#[tauri::command]
pub fn unlock_keyboard(app: AppHandle) -> AppState {
    crate::switches::lock_keyboard::unlock();
    automation::after_change(&app);
    current_state(&app.state::<SharedState>())
}

/// The Bluetooth picker's list (FR-38): id, name, connected, battery
/// (None -> "n/a").
#[tauri::command]
pub fn list_bt_devices() -> IpcResult<Vec<crate::switches::bluetooth::BtDevice>> {
    crate::switches::bluetooth::list_devices().map_err(map_switch_error)
}

/// Choose the Bluetooth device the switch connects (D-078). Saved through
/// the Phase 5 writer path; the narrow writer only covers its own field.
#[tauri::command]
pub fn set_bluetooth_device(
    app: AppHandle,
    state: State<'_, SharedState>,
    id: Option<String>,
) -> IpcResult<Settings> {
    let candidate = {
        let b = state.lock().unwrap_or_else(|p| p.into_inner());
        let mut candidate = b.settings.clone();
        candidate.bluetooth.device_id = id;
        candidate
    };
    // The chosen-device static updates only after a successful save, so a
    // failed save cannot fork the state.
    let saved = commit_settings(&app, &state, candidate)?;
    crate::switches::bluetooth::set_chosen(saved.bluetooth.device_id.clone());
    Ok(saved)
}

/// The Keep Awake "Let the screen turn off" checkbox (FR-18, D-081), saved
/// through the same writer path.
#[tauri::command]
pub fn set_keep_awake_options(
    app: AppHandle,
    state: State<'_, SharedState>,
    allow_display_off: bool,
) -> IpcResult<Settings> {
    let candidate = {
        let b = state.lock().unwrap_or_else(|p| p.into_inner());
        let mut candidate = b.settings.clone();
        candidate.keep_awake.allow_display_off = allow_display_off;
        candidate
    };
    commit_settings(&app, &state, candidate)
}

/// Validate + normalise, persist atomically, commit the in-memory copy,
/// refresh the tray and backdrop, and emit settings-changed. Shared tail
/// of every settings-writing path; side effects (hotkeys, autostart) are
/// the general writer's job and happen before this is called.
fn commit_settings(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    candidate: Settings,
) -> IpcResult<Settings> {
    commit_shared(app, state.inner(), candidate)
}

/// The same writer tail over a raw `SharedState` reference, so callers
/// outside a command (the onboarding close path) can reuse it.
pub(crate) fn commit_shared(
    app: &AppHandle,
    shared: &SharedState,
    mut candidate: Settings,
) -> IpcResult<Settings> {
    {
        let b = shared.lock().unwrap_or_else(|p| p.into_inner());
        if b.settings_read_only {
            return Err(IpcError::NotAvailable(
                "the settings file was written by a newer button; not saving over it".into(),
            ));
        }
    }
    candidate.schema_version = crate::settings::SCHEMA_VERSION;
    crate::settings::normalize_saving(&mut candidate).map_err(IpcError::InvalidArgument)?;
    {
        // Held across the write so a concurrent save cannot interleave.
        let _lock = shared.lock().unwrap_or_else(|p| p.into_inner());
        app.path()
            .app_local_data_dir()
            .map_err(|e| IpcError::OsError(e.to_string()))
            .and_then(|dir| {
                crate::settings::save(&candidate, &dir)
                    .map_err(|e| IpcError::OsError(e.to_string()))
            })?;
    }
    let backdrop_changed = {
        let mut b = shared.lock().unwrap_or_else(|p| p.into_inner());
        // Restyle only on a real backdrop change (spec 7.9): every other
        // save must not reload the backdrop webviews.
        let changed = b.settings.backdrop != candidate.backdrop;
        b.settings = candidate.clone();
        changed
    };
    // Infallible effects; failures are logged, never reported (the file is
    // already saved). The Bluetooth switch reads its target from the
    // process-wide chosen device, not settings, so every commit syncs it
    // (the same refresh set_bluetooth_device does).
    crate::switches::bluetooth::set_chosen(candidate.bluetooth.device_id.clone());
    crate::tray::update_tray(app);
    if backdrop_changed {
        crate::backdrop::restyle(app, &candidate.backdrop);
    }
    let _ = app.emit("settings-changed", &candidate);
    Ok(candidate)
}

/// The Phase 5 settings writer (FR-39, spec 6.1): validate + normalise,
/// apply fallible side effects transactionally (hotkeys, autostart),
/// then persist, commit memory, refresh the tray/backdrop and emit
/// settings-changed. The UI saves one change at a time, so usually only
/// one side effect is involved.
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, SharedState>,
    settings: Settings,
) -> IpcResult<Settings> {
    let (previous, candidate) = {
        let b = state.lock().unwrap_or_else(|p| p.into_inner());
        if b.settings_read_only {
            return Err(IpcError::NotAvailable(
                "the settings file was written by a newer button; not saving over it".into(),
            ));
        }
        if settings.schema_version > crate::settings::SCHEMA_VERSION {
            return Err(IpcError::InvalidArgument(
                "these settings use a newer schemaVersion; update button first".into(),
            ));
        }
        let mut candidate = settings;
        candidate.schema_version = crate::settings::SCHEMA_VERSION;
        crate::settings::normalize_saving(&mut candidate).map_err(IpcError::InvalidArgument)?;
        if candidate.backdrop.style == crate::settings::BackdropStyle::Picture
            && let Some(path) = &candidate.backdrop.picture
            && !std::path::Path::new(path).is_file()
        {
            return Err(IpcError::InvalidArgument(format!(
                "the picture '{path}' does not exist"
            )));
        }
        (b.settings.clone(), candidate)
    };

    // Fallible side effects first; every later failure rolls them back.
    let hotkeys_changed = candidate.hotkeys != previous.hotkeys;
    if hotkeys_changed
        && let Err(message) = crate::hotkeys::apply_all(&app, &previous.hotkeys, &candidate.hotkeys)
    {
        return Err(IpcError::OsError(message));
    }
    let autostart_changed =
        candidate.general.start_with_windows != previous.general.start_with_windows;
    if autostart_changed
        && let Err(message) = crate::autostart::apply(&app, candidate.general.start_with_windows)
    {
        if hotkeys_changed
            && let Err(e) = crate::hotkeys::apply_all(&app, &candidate.hotkeys, &previous.hotkeys)
        {
            log::warn!("restoring hotkeys after a failed autostart apply failed: {e}");
        }
        return Err(IpcError::OsError(message));
    }

    match commit_settings(&app, &state, candidate.clone()) {
        Ok(saved) => Ok(saved),
        Err(error) => {
            if hotkeys_changed
                && let Err(e) =
                    crate::hotkeys::apply_all(&app, &candidate.hotkeys, &previous.hotkeys)
            {
                log::warn!("restoring hotkeys after a failed save failed: {e}");
            }
            if autostart_changed
                && let Err(e) = crate::autostart::apply(&app, previous.general.start_with_windows)
            {
                log::warn!("restoring Start with Windows after a failed save failed: {e}");
            }
            Err(error)
        }
    }
}

/// Open (or focus) the Settings window (FR-39). Runs on the async runtime:
/// WebviewWindowBuilder deadlocks inside sync commands (D-071).
#[tauri::command]
pub async fn open_settings(app: AppHandle, section: Option<String>) {
    crate::settings_window::open(&app, section.as_deref());
}

/// The dropdown fits its window to its content (the switch list and the
/// detail pages differ in height). Height in CSS px; the edge next to the
/// tray stays put, so a dropdown above a bottom taskbar grows upward.
#[tauri::command]
pub fn fit_dropdown(window: tauri::WebviewWindow, height: f64) -> IpcResult<()> {
    if window.label() != "dropdown" {
        return Err(IpcError::InvalidArgument(
            "only the dropdown fits itself".into(),
        ));
    }
    if !(height.is_finite() && height > 0.0) {
        return Err(IpcError::InvalidArgument("height must be positive".into()));
    }
    crate::tray::fit_dropdown(&window, height);
    Ok(())
}

/// Open (or focus) the onboarding window without changing any flag
/// (FR-52, spec 7.14).
#[tauri::command]
pub async fn open_onboarding(app: AppHandle) {
    crate::onboarding::open(&app);
}

/// `check_update`'s result (spec 6.1).
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateCheck {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Check GitHub for a newer release (FR-22). Works with the launch check
/// turned off too: the user asked for it. A network failure is OS_ERROR.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> IpcResult<UpdateCheck> {
    let version = crate::updater::check(&app)
        .await
        .map_err(IpcError::OsError)?;
    Ok(UpdateCheck {
        available: version.is_some(),
        version,
    })
}

/// Install the update the last check found (FR-22, spec 7.12). Refused
/// with SESSION_ACTIVE while a session runs or the journal holds anything;
/// on success the app exits and the installer starts the new version.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> IpcResult<()> {
    crate::updater::install(&app).await.map_err(|e| match e {
        crate::updater::InstallError::SessionActive => IpcError::SessionActive(e.to_string()),
        crate::updater::InstallError::NoUpdate => IpcError::NotAvailable(e.to_string()),
        crate::updater::InstallError::Failed(_) => IpcError::OsError(e.to_string()),
    })
}

/// Copy the diagnostic report to the clipboard (FR-24).
#[tauri::command]
pub fn copy_diagnostics(app: AppHandle) -> IpcResult<()> {
    crate::diagnostics::copy_report(&app).map_err(IpcError::OsError)
}

/// One running app for the shelves (FR-43): unique exe basename, name is
/// the exe stem, no window titles (privacy, spec 5.1 rationale).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub exe: String,
    pub name: String,
}

/// Running apps for the "Add app" pickers, from the other-apps candidate
/// enumeration (FR-43). Sorted by name, deduped case-insensitively.
#[tauri::command]
pub fn list_apps() -> IpcResult<Vec<AppInfo>> {
    let entries = crate::switches::other_apps::candidates().map_err(map_switch_error)?;
    let mut out: Vec<AppInfo> = Vec::new();
    for entry in entries {
        if out.iter().any(|a| a.exe.eq_ignore_ascii_case(&entry.exe)) {
            continue;
        }
        let stem = entry
            .exe
            .strip_suffix(".exe")
            .or_else(|| entry.exe.strip_suffix(".EXE"))
            .unwrap_or(&entry.exe);
        out.push(AppInfo {
            exe: entry.exe.clone(),
            name: stem.to_string(),
        });
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    Ok(out)
}

/// Third-party tray icons for the Visible shelf (FR-30); NOT_AVAILABLE on
/// Windows 10 (the switch's own rule, FR-29).
#[tauri::command]
pub fn list_tray_icons(
    app: AppHandle,
) -> IpcResult<Vec<crate::switches::tray_icons::TrayIconInfo>> {
    let is_windows_11 = {
        let state = app.state::<SharedState>();
        let b = state.lock().unwrap_or_else(|p| p.into_inner());
        b.os.is_windows_11
    };
    if !is_windows_11 {
        return Err(IpcError::NotAvailable(
            "Windows 10 doesn't support this".into(),
        ));
    }
    crate::switches::tray_icons::list_entries().map_err(map_switch_error)
}

/// Native picture picker for the Backdrop section (FR-46, D-085). Runs on
/// a blocking thread; the dialog must not sit on the async runtime's main
/// dispatcher.
#[tauri::command]
pub async fn pick_backdrop_picture(app: AppHandle) -> IpcResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Pictures", &["png", "jpg", "jpeg", "bmp", "webp"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| IpcError::OsError(e.to_string()))?;
    Ok(picked
        .and_then(|file| file.into_path().ok())
        .and_then(|path| path.to_str().map(str::to_string)))
}

/// The current backdrop configuration for the overlay page (FR-46).
#[tauri::command]
pub fn get_backdrop_style(state: State<'_, SharedState>) -> crate::settings::Backdrop {
    let b = state.lock().unwrap_or_else(|p| p.into_inner());
    b.settings.backdrop.clone()
}

/// Push an active session's time limit back (FR-42): Active + a deadline
/// required, 1-240 minutes. The "+15 min" dropdown action is this command.
#[tauri::command]
pub fn extend_recording(
    app: AppHandle,
    state: State<'_, SharedState>,
    minutes: u32,
) -> IpcResult<AppState> {
    let outcome: Result<(), IpcError> = {
        let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
        if b.engine.state().status != crate::SessionStatus::Active {
            Err(IpcError::NoSession("no session is active".into()))
        } else if !(1..=240).contains(&minutes) {
            Err(IpcError::InvalidArgument("minutes must be 1-240".into()))
        } else if let Some(ends_at) = b.engine.state().ends_at {
            b.engine
                .set_time_limit(ends_at + time::Duration::minutes(i64::from(minutes)));
            Ok(())
        } else {
            Err(IpcError::InvalidArgument("no time limit is set".into()))
        }
    };
    outcome?;
    // D-090: the extended session warns again before its new end.
    crate::tray::rearm_limit_notice();
    automation::after_change(&app);
    Ok(current_state(&app.state::<SharedState>()))
}

/// The backdrop image bytes (picture path or the current Windows
/// wallpaper, D-087): raw body, empty when there is no usable image so
/// the page falls back to the colour. Magic-byte sniffed, size-capped.
#[tauri::command]
pub fn get_backdrop_image(app: AppHandle) -> tauri::ipc::Response {
    tauri::ipc::Response::new(crate::backdrop::image_bytes(&app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_errors_map_to_the_spec_codes() {
        assert!(matches!(
            map_error(automation::DispatchError::SessionActive),
            IpcError::SessionActive(_)
        ));
        assert!(matches!(
            map_error(automation::DispatchError::NoSession),
            IpcError::NoSession(_)
        ));
        assert!(matches!(
            map_error(automation::DispatchError::Failed("x".into())),
            IpcError::NotAvailable(_)
        ));
    }
}
