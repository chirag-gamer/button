//! Update check and install (FR-22, spec 7.12, 9). The updater plugin is
//! used from Rust only, so the webviews get no updater permissions.
//!
//! - One check at launch when `general.checkForUpdates` is on. Settings >
//!   About and the tray menu can check any time.
//! - A found update lives in `ButtonState.update`, so the tray and every
//!   window see it through AppState.
//! - Install is refused while a session runs or the journal holds
//!   anything (SESSION_ACTIVE). The plugin then runs the passive NSIS
//!   installer and exits the app; the installer starts it again.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::SharedState;
use crate::automation;

/// Why an install did not start.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("Stop Recording Mode and any switch timers first, then install the update")]
    SessionActive,
    #[error("No update is waiting. Check for updates first")]
    NoUpdate,
    #[error("The update is already downloading")]
    Busy,
    #[error("The update failed: {0}")]
    Failed(String),
}

/// Checks GitHub for a newer release and remembers the result (None =
/// up to date). The error is a user-facing message.
pub async fn check(app: &AppHandle) -> Result<Option<String>, String> {
    let updater = app
        .updater_builder()
        // A stalled check must not leave "Checking…" up forever.
        .timeout(Duration::from_secs(30))
        // The plugin would pass this process's command line to the new
        // version (`/ARGS`), so a `button.exe start` launch would start
        // Recording Mode after the update. Relaunch with no arguments.
        .restart_after_install(false)
        .installer_arg("/R")
        .build()
        .map_err(|e| format!("The update check failed: {e}"))?;
    let found = updater
        .check()
        .await
        .map_err(|e| format!("The update check failed: {e}"))?;
    let version = found.as_ref().map(|u| u.version.clone());
    {
        let state = app.state::<SharedState>();
        let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
        b.update = found;
    }
    match &version {
        Some(v) => log::info!("update check: {v} is available"),
        None => log::info!("update check: up to date"),
    }
    automation::after_change(app);
    Ok(version)
}

/// Set while an install runs, so a second click (tray or Settings) can't
/// start a second download.
static INSTALLING: AtomicBool = AtomicBool::new(false);

/// Clears INSTALLING on every early return.
struct InstallGuard;

impl Drop for InstallGuard {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::SeqCst);
    }
}

/// Downloads the waiting update, checks its signature, and runs the
/// installer. On success the process exits inside this call.
pub async fn install(app: &AppHandle) -> Result<(), InstallError> {
    if INSTALLING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(InstallError::Busy);
    }
    let _guard = InstallGuard;
    let mut update = {
        let state = app.state::<SharedState>();
        let b = state.lock().unwrap_or_else(|p| p.into_inner());
        if !b.engine.can_install_update() {
            return Err(InstallError::SessionActive);
        }
        b.update.clone().ok_or(InstallError::NoUpdate)?
    };
    // The check's 30 s limit would cut a slow download short.
    update.timeout = Some(Duration::from_secs(600));
    log::info!("update: downloading {}", update.version);
    // download() verifies the minisign signature against the embedded
    // public key and fails on a mismatch (spec 9).
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| InstallError::Failed(e.to_string()))?;
    // A session may have started during the download. The lock is held
    // from this check until the process exits, so none can start after it.
    let state = app.state::<SharedState>();
    let b = state.lock().unwrap_or_else(|p| p.into_inner());
    if !b.engine.can_install_update() {
        return Err(InstallError::SessionActive);
    }
    log::info!("update: installing {}", update.version);
    log::logger().flush();
    update
        .install(bytes)
        .map_err(|e| InstallError::Failed(e.to_string()))
}

/// The launch check (FR-22): runs in the background, offers a found
/// update with a notification, and only logs failures (an offline PC
/// should not get a warning at every start).
pub fn check_at_launch(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match check(&app).await {
            Ok(Some(version)) => automation::notify(
                &app,
                &format!(
                    "button {version} is available. Choose Install update in the tray menu or in Settings > About."
                ),
            ),
            Ok(None) => {}
            Err(message) => log::warn!("{message}"),
        }
    });
}

/// The tray's update item: installs a found update, otherwise checks.
/// Results go to a notification.
pub fn tray_action(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let waiting = {
            let state = app.state::<SharedState>();
            let b = state.lock().unwrap_or_else(|p| p.into_inner());
            b.update.is_some()
        };
        if waiting {
            if let Err(e) = install(&app).await {
                automation::notify(&app, &e.to_string());
            }
            return;
        }
        let message = match check(&app).await {
            Ok(Some(version)) => format!(
                "button {version} is available. Choose Install update in the tray menu to install it."
            ),
            Ok(None) => "button is up to date".to_string(),
            Err(message) => message,
        };
        automation::notify(&app, &message);
    });
}
