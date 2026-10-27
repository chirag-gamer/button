//! IPC commands (spec 6.1). Recording Mode commands plus the Phase 4
//! quick-switch set: set_switch, unlock_keyboard, list_bt_devices,
//! set_bluetooth_device, set_keep_awake_options (D-078). Read-only
//! get_settings stays; save_settings arrives with the Settings UI (Phase 5).
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

/// Maps a dispatcher failure to its IPC code (spec 6.1). Matched on the
/// lowercased message so dispatcher and engine casings both map.
fn map_error(message: String) -> IpcError {
    match message.to_lowercase().as_str() {
        "a session is already active" => IpcError::SessionActive(message),
        "no session is active" => IpcError::NoSession(message),
        _ => IpcError::NotAvailable(message),
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

/// Choose the Bluetooth device the switch connects (D-078). Saves the
/// setting right away — the Phase 5 settings window generalizes this.
/// The candidate is persisted before anything in memory or the switch's
/// chosen-device static changes, so a failed save cannot fork the state.
#[tauri::command]
pub fn set_bluetooth_device(
    app: AppHandle,
    state: State<'_, SharedState>,
    id: Option<String>,
) -> IpcResult<Settings> {
    let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
    let mut candidate = b.settings.clone();
    candidate.bluetooth.device_id = id.clone();
    save_settings(&app, b.settings_read_only, &candidate)?;
    b.settings = candidate;
    crate::switches::bluetooth::set_chosen(id);
    let settings = b.settings.clone();
    drop(b);
    let _ = app.emit("settings-changed", &settings);
    Ok(settings)
}

/// The Keep Awake "Let the screen turn off" checkbox (FR-18, D-081), saved
/// through the same persist-before-commit rule.
#[tauri::command]
pub fn set_keep_awake_options(
    app: AppHandle,
    state: State<'_, SharedState>,
    allow_display_off: bool,
) -> IpcResult<Settings> {
    let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
    let mut candidate = b.settings.clone();
    candidate.keep_awake.allow_display_off = allow_display_off;
    save_settings(&app, b.settings_read_only, &candidate)?;
    b.settings = candidate;
    let settings = b.settings.clone();
    drop(b);
    let _ = app.emit("settings-changed", &settings);
    Ok(settings)
}

/// Shared tail of the two setting-writing commands: refuse to overwrite a
/// settings file from a newer build (spec 7.12), then persist.
fn save_settings(
    app: &AppHandle,
    settings_read_only: bool,
    candidate: &Settings,
) -> Result<(), IpcError> {
    if settings_read_only {
        return Err(IpcError::NotAvailable(
            "the settings file was written by a newer button; not saving over it".into(),
        ));
    }
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| IpcError::OsError(e.to_string()))?;
    crate::settings::save(candidate, &dir).map_err(|e| IpcError::OsError(e.to_string()))
}
