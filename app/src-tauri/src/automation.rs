//! The one dispatcher every trigger goes through: tray menu, tray clicks,
//! hotkey, dropdown and the command line (spec 2.2, 6.3).

use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Emitter, Manager};
use time::OffsetDateTime;

use crate::settings::Settings;
use crate::switches::{Ctx, SwitchId};
use crate::tray;
use crate::{AppState, SessionStatus, SharedState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Start,
    /// The countdown thread's real start (FR-41): bypasses the countdown
    /// entry so it cannot loop. Never produced by the CLI parser.
    StartNow,
    Stop,
    Toggle,
    /// Toggle the dropdown window (`button.exe` with no args; the tray
    /// left click is handled directly in `tray.rs`). Carries the click
    /// rect in physical pixels when known, so the window can be placed
    /// next to the tray icon (spec 7.13).
    ShowDropdown {
        click: Option<crate::position::ClickRect>,
    },
    /// `button.exe switch <id> on|off|toggle`: one quick switch through the
    /// same engine path as the dropdown (FR-27, spec 6.3).
    Switch {
        id: SwitchId,
        on: SwitchOnOff,
    },
    /// `button.exe restore`: restore any leftover journal (spec 6.3, U-7).
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchOnOff {
    On,
    Off,
    Toggle,
}

/// Strict parser for `button.exe ...` (spec 6.3). The exe path must already
/// be stripped. Anything else than the exact grammar is an error.
///
/// ```text
/// button.exe []                    -> ShowDropdown
/// button.exe start|stop|toggle     -> Start | Stop | Toggle
/// button.exe switch <id> on|off|toggle
/// button.exe restore               -> Restore
/// ```
pub fn parse(args: &[String]) -> Result<Action, String> {
    let mut args = args.iter().map(String::as_str);
    let first = args.next();
    match first {
        None => Ok(Action::ShowDropdown { click: None }),
        Some("start") | Some("stop") | Some("toggle") | Some("restore") => {
            if args.next().is_some() {
                return Err(format!(
                    "unexpected extra argument after '{}'",
                    first.unwrap()
                ));
            }
            Ok(match first.unwrap() {
                "start" => Action::Start,
                "stop" => Action::Stop,
                "toggle" => Action::Toggle,
                _ => Action::Restore,
            })
        }
        Some("switch") => {
            let id = args
                .next()
                .ok_or_else(|| "switch needs an id: switch <id> on|off|toggle".to_string())?;
            let id = parse_switch_id(id)?;
            let on = args
                .next()
                .ok_or_else(|| "switch needs on|off|toggle".to_string())?;
            let on = match on {
                "on" => SwitchOnOff::On,
                "off" => SwitchOnOff::Off,
                "toggle" => SwitchOnOff::Toggle,
                other => return Err(format!("expected on|off|toggle, got '{other}'")),
            };
            if let Some(extra) = args.next() {
                return Err(format!("unexpected extra argument '{extra}'"));
            }
            Ok(Action::Switch { id, on })
        }
        Some(other) => Err(format!(
            "unknown command '{other}': use start|stop|toggle|switch <id> on|off|toggle|restore"
        )),
    }
}

/// Switch ids in the CLI use the kebab-case names from the dropdown (5.4).
pub fn parse_switch_id(id: &str) -> Result<SwitchId, String> {
    serde_json::from_value(serde_json::json!(id)).map_err(|_| format!("unknown switch '{id}'"))
}

/// The work after every state change, in one place: tray icon/tooltip/menu
/// text, then the state-changed event to the UI (spec 2.5 instruction).
pub fn after_change(app: &AppHandle) {
    tray::update_tray(app);
    let state = app.state::<SharedState>();
    let b = state.lock().unwrap_or_else(|p| p.into_inner());
    let app_state: AppState = crate::build_app_state(&b);
    drop(b);
    let _ = app.emit("state-changed", app_state);
}

/// D-066: what one start needs, from settings plus the front tracker.
/// Recording Mode always keeps the display awake (D-069).
fn build_ctx(settings: &Settings) -> Ctx {
    Ctx {
        front: crate::front::front(&settings.apps.stays_on_screen),
        stays_on_screen: settings.apps.stays_on_screen.clone(),
        tray_visible: settings.tray_icons.visible.clone(),
        backdrop_color: settings.backdrop.color.clone(),
        own_pid: std::process::id(),
        own_exe: std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_default(),
        keep_awake_display: true,
    }
}

/// What a manual `set_switch` needs: the standalone Keep Awake obeys the
/// "Let the screen turn off" option (D-081).
pub fn manual_ctx(settings: &Settings) -> Ctx {
    Ctx {
        keep_awake_display: !settings.keep_awake.allow_display_off,
        ..build_ctx(settings)
    }
}

/// Re-read every switch row and push the fresh state to the UI (dropdown
/// show, delayed Bluetooth refresh, keyboard unlock — task 4.2's refresh
/// points outside the engine).
#[cfg_attr(test, allow(dead_code))] // every caller is cfg(not(test)) (D-074)
pub fn refresh_switches(app: &AppHandle) {
    let state = app.state::<SharedState>();
    let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
    b.engine.refresh_rows();
    drop(b);
    after_change(app);
}

/// Dispatcher failures; commands.rs maps these onto the spec 6.1 codes
/// (replaces matching the error prose by string, which drifted silently
/// into NOT_AVAILABLE on any wording change).
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("A session is already active")]
    SessionActive,
    #[error("No session is active")]
    NoSession,
    #[error("{0}")]
    Failed(String),
}

fn dispatch_err(e: crate::session::SessionError) -> DispatchError {
    match e {
        crate::session::SessionError::AlreadyActive => DispatchError::SessionActive,
        crate::session::SessionError::NotActive => DispatchError::NoSession,
        crate::session::SessionError::Io(e) => DispatchError::Failed(e.to_string()),
        crate::session::SessionError::JournalBlocked => {
            DispatchError::Failed(crate::session::SessionError::JournalBlocked.to_string())
        }
    }
}

pub fn run(app: &AppHandle, action: Action) -> Result<(), DispatchError> {
    // The single-instance callback can arrive during setup, before
    // ButtonState is managed (registry build + recovery + probes take a
    // moment; every other caller exists only after setup). Refuse instead
    // of panicking on the missing state.
    let Some(state) = app.try_state::<SharedState>() else {
        let message = "button is still starting; try again in a moment";
        log::warn!("dispatcher dropped {action:?}: {message}");
        return Err(DispatchError::Failed(message.into()));
    };
    let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
    let now = OffsetDateTime::now_utc();

    let status = b.engine.state().status;
    let active = status == SessionStatus::Active;

    // FR-41: with the countdown on, Start/Toggle from Idle opens the 3·2·1
    // countdown instead of starting. The countdown thread does the real
    // start through StartNow; a second press (the hotkey) cancels.
    if status == SessionStatus::Idle
        && b.settings.recording_mode.countdown
        && matches!(action, Action::Start | Action::Toggle)
    {
        let generation = COUNTDOWN_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        b.engine.set_status(SessionStatus::Countdown);
        drop(b);
        spawn_countdown(app.clone(), generation);
        after_change(app);
        return Ok(());
    }
    if status == SessionStatus::Countdown {
        match action {
            // Start while counting: already in progress.
            Action::Start => return Ok(()),
            // Stop and Toggle cancel the countdown (FR-41 acceptance).
            Action::Stop | Action::Toggle => {
                let counted = COUNTDOWN_GENERATION.load(Ordering::Relaxed);
                COUNTDOWN_GENERATION.fetch_add(1, Ordering::Relaxed);
                b.engine.set_status(SessionStatus::Idle);
                drop(b);
                crate::countdown::close(app, counted);
                after_change(app);
                return Ok(());
            }
            // Restore cancels too, then does its real work below (the
            // journal is empty during a countdown, so this is the
            // "nothing to restore" notification).
            Action::Restore => {
                let counted = COUNTDOWN_GENERATION.load(Ordering::Relaxed);
                COUNTDOWN_GENERATION.fetch_add(1, Ordering::Relaxed);
                b.engine.set_status(SessionStatus::Idle);
                drop(b);
                crate::countdown::close(app, counted);
                after_change(app);
                b = state.lock().unwrap_or_else(|p| p.into_inner());
            }
            _ => {}
        }
    }

    let selected = b.settings.recording_mode.actions.clone();
    let mut started = false;
    let result: Result<(), DispatchError> = match action {
        Action::Start | Action::StartNow if !active => {
            let ctx = build_ctx(&b.settings);
            b.engine
                .start(now, &selected, &ctx)
                .map(|_| started = true)
                .map_err(dispatch_err)
        }
        Action::Start => Err(DispatchError::SessionActive),
        Action::StartNow => Err(DispatchError::SessionActive),
        Action::Stop if active => b.engine.stop(now).map(|_| ()).map_err(dispatch_err),
        Action::Stop => Err(DispatchError::NoSession),
        Action::Toggle if active => b.engine.stop(now).map(|_| ()).map_err(dispatch_err),
        Action::Toggle => {
            let ctx = build_ctx(&b.settings);
            b.engine
                .start(now, &selected, &ctx)
                .map(|_| started = true)
                .map_err(dispatch_err)
        }
        Action::ShowDropdown { click } => {
            drop(b);
            tray::show_dropdown(app, click).map_err(DispatchError::Failed)?;
            return Ok(());
        }
        Action::Switch { id, on } => {
            let ctx = manual_ctx(&b.settings);
            let resolved = match on {
                SwitchOnOff::On => true,
                SwitchOnOff::Off => false,
                SwitchOnOff::Toggle => !b.engine.is_switch_on(id),
            };
            b.engine
                .set_switch(id, resolved, None, now, &ctx)
                .map_err(|e| DispatchError::Failed(e.to_string()))
        }
        Action::Restore => {
            // Stop the session if one is active, then restore every
            // remaining journal entry, timers included. Never run launch
            // recovery while the app runs: it would drop non-persistent
            // entries like keep-awake (spec 7.3 vs 7.2, task 4.2).
            let summary = match b.engine.restore_all_now(now) {
                Ok(0) if active => "Recording Mode ended".to_string(),
                Ok(0) => "Everything button still owned was restored".to_string(),
                Ok(failed) => format!(
                    "Restored everything else, but {failed} switch(es) could not be restored"
                ),
                Err(e) => format!("Restore failed: {e}"),
            };
            notify(app, &summary);
            drop(b);
            after_change(app);
            return Ok(());
        }
    };
    // FR-42: the time limit deadline is set once, when the session starts.
    if started && let Some(minutes) = b.settings.recording_mode.time_limit_minutes {
        b.engine
            .set_time_limit(now + time::Duration::minutes(i64::from(minutes)));
    }
    drop(b);
    // Sync tray + UI on success and on error alike: the engine may have
    // changed state even when the overall action failed.
    after_change(app);
    result?;
    // A started session closes the dropdown (spec 7.1.4).
    if started {
        tray::hide_dropdown(app);
    }
    // Summary when the last run left failures behind (apply or restore,
    // spec 7.1.4 / 7.2). Only the session actions own this summary; a quick
    // switch must not re-announce an old session's failures.
    if matches!(
        action,
        Action::Start | Action::Stop | Action::Toggle | Action::Restore
    ) {
        let state = app.state::<SharedState>();
        let b = state.lock().unwrap_or_else(|p| p.into_inner());
        let failed = b.engine.state().failed.len();
        let idle = b.engine.state().status == SessionStatus::Idle;
        drop(b);
        if idle && failed > 0 {
            notify(
                app,
                &format!("{failed} switch(es) failed during the last run"),
            );
        }
    }
    Ok(())
}

/// One countdown at a time (FR-41): the generation counter cancels a
/// running countdown when any trigger bumps it.
static COUNTDOWN_GENERATION: AtomicU64 = AtomicU64::new(0);

/// The current countdown generation, for the countdown window's
/// self-check after its build finishes.
pub(crate) fn countdown_generation() -> u64 {
    COUNTDOWN_GENERATION.load(Ordering::Relaxed)
}

/// The countdown thread (FR-41): open the window, emit 3·2·1 one second
/// apart, then start for real unless any trigger bumped the generation in
/// the meantime (cancel). The state lock is never held while sleeping.
fn spawn_countdown(app: AppHandle, generation: u64) {
    std::thread::spawn(move || {
        crate::countdown::open(&app, generation);
        for seconds_left in [3u8, 2, 1] {
            if COUNTDOWN_GENERATION.load(Ordering::Relaxed) != generation {
                // Cancelled: the cancelling trigger already closed this
                // generation's window (or the build thread's self-check
                // will); never close another generation's window.
                return;
            }
            let _ = app.emit(
                "countdown",
                serde_json::json!({ "secondsLeft": seconds_left }),
            );
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        if COUNTDOWN_GENERATION.load(Ordering::Relaxed) != generation {
            return;
        }
        crate::countdown::close(&app, generation);
        if COUNTDOWN_GENERATION.load(Ordering::Relaxed) != generation {
            return;
        }
        // Back to Idle so the engine's start (which requires Idle) runs;
        // StartNow bypasses the countdown entry, so this cannot loop.
        {
            let state = app.state::<SharedState>();
            let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
            if b.engine.state().status != SessionStatus::Countdown {
                return; // something else changed the state meanwhile
            }
            b.engine.set_status(SessionStatus::Idle);
        }
        if let Err(message) = run(&app, Action::StartNow) {
            notify(&app, &message.to_string());
        }
    });
}

/// Failures and CLI results go to a notification (spec 6.3: button has no
/// console output) and to the log.
pub(crate) fn notify(app: &AppHandle, message: &str) {
    log::warn!("{message}");
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title("button")
        .body(message)
        .show();
}

/// FR-8: restore an active session before the process ends. Runs on the
/// main thread from RunEvent::Exit.
pub fn restore_on_exit(app: &AppHandle) {
    let state = app.state::<SharedState>();
    let mut b = state.lock().unwrap_or_else(|p| p.into_inner());
    if b.engine.state().status == SessionStatus::Active {
        let now = OffsetDateTime::now_utc();
        match b.engine.stop(now) {
            Ok(_) => log::info!("session restored before exit"),
            Err(e) => log::error!("exit restore failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, SwitchOnOff, parse, parse_switch_id};

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_grammar_form_parses() {
        assert_eq!(parse(&args(&[])), Ok(Action::ShowDropdown { click: None }));
        assert_eq!(parse(&args(&["start"])), Ok(Action::Start));
        assert_eq!(parse(&args(&["stop"])), Ok(Action::Stop));
        assert_eq!(parse(&args(&["toggle"])), Ok(Action::Toggle));
        assert_eq!(parse(&args(&["restore"])), Ok(Action::Restore));
        assert_eq!(
            parse(&args(&["switch", "desktop-icons", "on"])),
            Ok(Action::Switch {
                id: crate::SwitchId::DesktopIcons,
                on: SwitchOnOff::On
            })
        );
        assert_eq!(
            parse(&args(&["switch", "keep-awake", "off"])),
            Ok(Action::Switch {
                id: crate::SwitchId::KeepAwake,
                on: SwitchOnOff::Off
            })
        );
        assert_eq!(
            parse(&args(&["switch", "taskbar", "toggle"])),
            Ok(Action::Switch {
                id: crate::SwitchId::Taskbar,
                on: SwitchOnOff::Toggle
            })
        );
    }

    #[test]
    fn junk_is_rejected_with_a_message() {
        assert!(parse(&args(&["start", "x"])).is_err());
        assert!(parse(&args(&["frobnicate"])).is_err());
        assert!(parse(&args(&["switch"])).is_err());
        assert!(parse(&args(&["switch", "no-such-switch", "on"])).is_err());
        assert!(parse(&args(&["switch", "taskbar", "maybe"])).is_err());
        assert!(parse(&args(&["switch", "taskbar", "on", "extra"])).is_err());
        assert!(parse(&args(&["SWITCH"])).is_err(), "case sensitive");
        // Junk that parses as a kebab id name fails id validation.
        assert!(parse_switch_id("desktop icons").is_err());
    }
}
