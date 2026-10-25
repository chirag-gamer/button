//! IPC commands (spec 6.1). Phase 2 subset: get_state, the three Recording
//! Mode commands and read-only get_settings. The rest arrive with their
//! phases. Typed wrappers live in `app/src/shared/ipc.ts`.

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::automation::{self, Action};
use crate::settings::Settings;
use crate::{AppState, SharedState, build_app_state};

/// Error shape for every failing command (spec 6.1).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "code", content = "message", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IpcError {
    SessionActive(String),
    NoSession(String),
    NotAvailable(String),
    #[allow(dead_code)] // produced by Phase 4+ commands
    InvalidArgument(String),
    #[allow(dead_code)] // produced by Phase 3+ switch commands
    OsError(String),
}

type IpcResult<T> = Result<T, IpcError>;

/// Maps a dispatcher failure to its IPC code (spec 6.1).
fn map_error(message: String) -> IpcError {
    match message.as_str() {
        "A session is already active" => IpcError::SessionActive(message),
        "No session is active" => IpcError::NoSession(message),
        _ => IpcError::NotAvailable(message),
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
