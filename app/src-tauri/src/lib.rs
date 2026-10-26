mod automation;
mod backdrop;
pub mod com;
pub mod commands;
mod front;
mod hotkeys;
pub mod journal;
pub mod os;
pub mod position;
pub mod session;
pub mod settings;
pub mod switches;
mod tray;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use os::OsInfo;
use switches::{Availability, SwitchId};

/// The per-switch item the UI sees (spec 5.4).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchState {
    pub id: SwitchId,
    pub on: bool,
    pub availability: Availability,
    pub experimental: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub off_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedSwitch {
    pub id: SwitchId,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Idle,
    Countdown,
    Applying,
    Active,
    Restoring,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub status: SessionStatus,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub started_at: Option<OffsetDateTime>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub ends_at: Option<OffsetDateTime>,
    pub applied: Vec<SwitchId>,
    pub skipped: Vec<SkippedSwitch>,
    pub failed: Vec<SkippedSwitch>,
}

impl SessionState {
    pub fn idle() -> Self {
        Self {
            status: SessionStatus::Idle,
            started_at: None,
            ends_at: None,
            applied: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyState {
    pub binding: String,
    pub registered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub version: String,
}

/// The whole in-memory model shared with the UI (spec 5.4).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub session: SessionState,
    pub switches: Vec<SwitchState>,
    pub hotkey: HotkeyState,
    pub os: OsInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<UpdateState>,
}

/// Everything mutable the app shares, behind one `Mutex` (commands.rs owns
/// the wrapper; tray, hotkeys and the dispatcher read through it).
pub struct ButtonState {
    pub engine: session::SessionEngine,
    pub settings: settings::Settings,
    /// The on-disk settings schemaVersion is newer than this build knows:
    /// never save over it (spec 7.12).
    pub settings_read_only: bool,
    pub hotkey: HotkeyState,
    pub os: OsInfo,
}

pub type SharedState = std::sync::Mutex<ButtonState>;

pub fn build_app_state(b: &ButtonState) -> AppState {
    AppState {
        session: b.engine.state().clone(),
        switches: b.engine.switch_states(),
        hotkey: b.hotkey.clone(),
        os: b.os,
        update: None,
    }
}

/// Atomic write protocol (spec 7.4): write `<name>.tmp` in the same dir,
/// sync, then rename over the target. std::fs::rename replaces an existing
/// target on Windows (MoveFileExW; see spec 7.4 citation).
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let name = path.file_name().expect("path has a file name");
    let tmp = path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(format!("{}.tmp", name.to_string_lossy()));
    let file = std::fs::File::create(&tmp)?;
    (&file).write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be the first plugin (spec 6.3): a second process forwards its
        // args here and exits (FR-14).
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let cli_args: Vec<String> = args.iter().skip(1).cloned().collect();
            match automation::parse(&cli_args) {
                Ok(action) => {
                    if let Err(message) = automation::run(app, action) {
                        automation::notify(app, &message);
                    }
                }
                Err(message) => automation::notify(app, &message),
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            // ~5 rotated 1 MB files in the app log dir (spec 8).
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .max_file_size(1024 * 1024)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                ])
                .build(),
        )
        .setup(|app| {
            use tauri::Manager;
            let os = os::detect();
            let data_dir = app.path().app_local_data_dir().expect("app data dir");
            std::fs::create_dir_all(&data_dir).ok();
            let now = time::OffsetDateTime::now_utc();

            let settings_read = settings::load(&data_dir, now).expect("settings load");
            let settings_read_only = matches!(settings_read, settings::SettingsRead::Newer(_));
            let settings = match settings_read {
                settings::SettingsRead::Fresh(s)
                | settings::SettingsRead::Loaded(s)
                | settings::SettingsRead::Invalid(s)
                | settings::SettingsRead::Newer(s) => s,
            };
            let backdrop_color = settings.backdrop.color.clone();

            let mut engine = session::SessionEngine::new(
                switches::build_registry(&data_dir),
                os,
                data_dir,
                env!("CARGO_PKG_VERSION").to_string(),
            );
            // Crash recovery runs before the tray is ready (spec 7.3). A
            // failing recovery must not stop the app from booting: the
            // journal stays on disk for the next launch.
            let recovery = match engine.recover(now) {
                Ok(report) => report,
                Err(e) => {
                    log::error!("launch recovery failed: {e}");
                    session::RecoveryReport::default()
                }
            };
            let hotkey_binding = settings.hotkeys.recording_mode.clone();

            app.manage(std::sync::Mutex::new(crate::ButtonState {
                engine,
                settings,
                settings_read_only,
                hotkey: crate::HotkeyState {
                    binding: hotkey_binding.clone(),
                    registered: false,
                },
                os,
            }));
            // FR-6: notify what launch recovery restored, including what
            // failed to restore.
            if !recovery.restored.is_empty()
                || !recovery.timers_to_rearm.is_empty()
                || !recovery.failed.is_empty()
            {
                use tauri_plugin_notification::NotificationExt;
                let mut message = format!(
                    "Restored {} switch(es) after an unexpected exit",
                    recovery.restored.len()
                );
                if !recovery.failed.is_empty() {
                    message.push_str(&format!(
                        ", {} could not be restored",
                        recovery.failed.len()
                    ));
                }
                let _ = app
                    .notification()
                    .builder()
                    .title("button")
                    .body(&message)
                    .show();
                log::info!("{message}");
            }

            // Panics are logged; the journal survives for the next launch
            // (spec 8).
            std::panic::set_hook(Box::new(|info| {
                log::error!("panic: {info}");
            }));

            let tray_handles = tray::build(app.handle())?;
            app.manage(tray_handles);
            tray::update_tray(app.handle());
            tray::start_ticker(app.handle().clone());

            // The foreground hook must be installed on the main thread: its
            // events arrive on that thread's message pump (spec 2.3, 7.8).
            front::install();

            // The backdrop windows exist from launch, hidden (D-071):
            // WebviewWindowBuilder deadlocks inside sync commands, so they
            // can only be created here, one per monitor present at launch.
            backdrop::create(app.handle(), &backdrop_color);

            // The dropdown lives next to the tray icon: frameless, always on
            // top, hidden until a tray click (spec 5.4 intro, 7.13). It is
            // content-protected so captures don't show it (D-032).
            let dropdown = tauri::WebviewWindowBuilder::new(
                app,
                "dropdown",
                tauri::WebviewUrl::App("dropdown.html".into()),
            )
            .title("button")
            .inner_size(320.0, 440.0)
            .decorations(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .resizable(false)
            .visible(false)
            .content_protected(true)
            .build()?;
            tray::attach_blur_close(&dropdown);

            // Global hotkey (FR-9): failure leaves registered=false and the
            // dropdown warns (spec 7.11).
            let registered = hotkeys::register_toggle(app.handle(), &hotkey_binding);
            hotkeys::set_registered(app.handle(), &hotkey_binding, registered);

            // A CLI invocation runs its action and exits; `button.exe` with
            // no arguments behaves like the tray icon's left click later
            // (task 2.6). Task 2.8 adds forwarding to the running instance.
            // Exit code is always 0 (spec 6.3, KNOW verified in 2.8).
            let cli_args: Vec<String> = std::env::args().skip(1).collect();
            match automation::parse(&cli_args) {
                Ok(action) if !matches!(action, automation::Action::ShowDropdown { .. }) => {
                    // Spec 6.3: with no instance running, this process
                    // becomes the instance and runs the command. It stays
                    // resident so the session (or dropdown) survives.
                    if let Err(message) = automation::run(app.handle(), action) {
                        automation::notify(app.handle(), &message);
                    }
                }
                Err(error) => {
                    automation::notify(app.handle(), &error);
                    app.handle().exit(0);
                }
                _ => {}
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            crate::commands::get_state,
            crate::commands::get_settings,
            crate::commands::start_recording,
            crate::commands::stop_recording,
            crate::commands::toggle_recording,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // FR-8: an active session is restored before the process ends
            // (Quit, sign-out, shutdown when Windows allows it).
            if let tauri::RunEvent::Exit = event {
                automation::restore_on_exit(_app);
                front::unhook();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use switches::Availability;

    fn test_state() -> AppState {
        AppState {
            session: SessionState::idle(),
            switches: vec![
                SwitchState {
                    id: SwitchId::DesktopIcons,
                    on: false,
                    availability: Availability::Available,
                    experimental: false,
                    off_at: None,
                    detail: None,
                },
                SwitchState {
                    id: SwitchId::TrayIcons,
                    on: false,
                    availability: Availability::NeedsWindows11,
                    experimental: false,
                    off_at: None,
                    detail: Some("Needs Windows 11".into()),
                },
            ],
            hotkey: HotkeyState {
                binding: "Alt+R".into(),
                registered: true,
            },
            os: OsInfo {
                build: 26200,
                is_windows_11: true,
            },
            update: None,
        }
    }

    // spec 5.4: exact key names and casing in the serialized AppState.
    #[test]
    fn app_state_serializes_with_ui_key_names() {
        let json = serde_json::to_string(&test_state()).unwrap();
        assert!(json.contains("\"session\""), "{json}");
        assert!(json.contains("\"status\":\"idle\""), "{json}");
        assert!(json.contains("\"isWindows11\""), "{json}");
        assert!(json.contains("\"id\":\"desktop-icons\""), "{json}");
        assert!(
            json.contains("\"availability\":\"needs-windows-11\""),
            "{json}"
        );
        assert!(json.contains("\"registered\":true"), "{json}");
        assert!(!json.contains("update"), "{json}");
    }
}
