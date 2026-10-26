//! Keep Awake (FR-18, spec 7.5, D-068, D-069). One dedicated long-lived
//! thread owns `SetThreadExecutionState`: the switch sends commands over a
//! channel and waits for the reply, so the flag lives on a thread that
//! never runs COM/message-loop code. Clearing is `ES_CONTINUOUS` alone;
//! the request self-clears when the process exits (crash-safe, research/02).

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use serde::{Deserialize, Serialize};
use windows::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
};

use super::{Availability, Ctx, Switch, SwitchError, SwitchId, parse_state};
use crate::os::OsInfo;

/// State shape: `{"on": bool, "display": bool}` (D-069).
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub on: bool,
    pub display: bool,
}

enum Command {
    Apply {
        on: bool,
        display: bool,
        reply: Sender<Result<(), SwitchError>>,
    },
    Read {
        reply: Sender<State>,
    },
}

pub struct KeepAwake {
    tx: Mutex<Sender<Command>>,
}

impl KeepAwake {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("keep-awake".into())
            .spawn(move || worker(rx))
            .expect("keep-awake thread");
        Self { tx: Mutex::new(tx) }
    }

    fn ask(&self, command: Command) -> Result<(), SwitchError> {
        self.tx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .send(command)
            .map_err(|_| SwitchError::NotAvailable("keep-awake thread is gone".into()))
    }
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self::new()
    }
}

impl Switch for KeepAwake {
    fn id(&self) -> SwitchId {
        SwitchId::KeepAwake
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available
    }

    fn persistent(&self) -> bool {
        false // dies with the process (spec 7.3)
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        let (tx, rx) = channel();
        self.ask(Command::Read { reply: tx })?;
        let state = rx
            .recv()
            .map_err(|_| SwitchError::NotAvailable("keep-awake thread is gone".into()))?;
        Ok(serde_json::json!(state))
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!(State {
            on: true,
            display: true
        })
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let state = parse_state::<State>(value, "keep-awake")?;
        let (tx, rx) = channel();
        self.ask(Command::Apply {
            on: state.on,
            display: state.display,
            reply: tx,
        })?;
        rx.recv()
            .map_err(|_| SwitchError::NotAvailable("keep-awake thread is gone".into()))?
    }

    // restore keeps the trait default: clearing only when still ours.

    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "keep-awake")
            .map(|state| state.on)
            .unwrap_or(false)
    }
}

fn worker(rx: Receiver<Command>) {
    // The applied state, as this thread last set it.
    let mut on = false;
    let mut display = false;
    while let Ok(command) = rx.recv() {
        match command {
            Command::Apply {
                on: new_on,
                display: new_display,
                reply,
            } => {
                let flags = if new_on {
                    let mut flags = ES_CONTINUOUS | ES_SYSTEM_REQUIRED;
                    if new_display {
                        flags |= ES_DISPLAY_REQUIRED;
                    }
                    flags
                } else {
                    ES_CONTINUOUS
                };
                // 0 means the call failed (KNOW, probe line 344).
                let prev = unsafe { SetThreadExecutionState(flags) };
                if prev.0 == 0 {
                    let _ = reply.send(Err(SwitchError::Os {
                        api: "SetThreadExecutionState",
                        code: 0,
                    }));
                } else {
                    on = new_on;
                    display = new_display;
                    let _ = reply.send(Ok(()));
                }
            }
            Command::Read { reply } => {
                let _ = reply.send(State { on, display });
            }
        }
    }
    // Channel closed: the switch was dropped. The OS clears the request
    // when this thread (or the process) exits.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shape_is_on_and_display() {
        let state =
            parse_state::<State>(&serde_json::json!({ "on": true, "display": true }), "t").unwrap();
        assert!(state.on && state.display);
        assert!(parse_state::<State>(&serde_json::json!(true), "t").is_err());
        assert!(
            parse_state::<State>(&serde_json::json!({ "on": true }), "t").is_err(),
            "display is part of the shape (D-069)"
        );
    }

    #[test]
    fn is_on_means_the_awake_request_is_set() {
        assert!(KeepAwake::new().is_on(&serde_json::json!(State {
            on: true,
            display: false
        })));
        assert!(!KeepAwake::new().is_on(&serde_json::json!(State {
            on: false,
            display: true
        })));
    }

    #[test]
    fn target_on_is_system_and_display() {
        let after = KeepAwake::new().target_on(
            &serde_json::json!(State {
                on: false,
                display: false
            }),
            &Ctx::default(),
        );
        assert_eq!(
            after,
            serde_json::json!(State {
                on: true,
                display: true
            })
        );
    }

    /// Applies and clears the real request around the assertions. The
    /// effect lasts microseconds and clears itself on drop — no undo
    /// needed; `powercfg /requests` lists nothing after the test.
    #[test]
    fn apply_round_trips_through_the_worker_thread() {
        let switch = KeepAwake::new();
        switch
            .apply(&serde_json::json!(State {
                on: true,
                display: true
            }))
            .unwrap();
        assert!(switch.read().unwrap().to_string().contains("\"on\":true"));
        switch
            .apply(&serde_json::json!(State {
                on: false,
                display: false
            }))
            .unwrap();
        assert!(switch.read().unwrap().to_string().contains("\"on\":false"));
    }
}
