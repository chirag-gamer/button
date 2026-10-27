//! Night Light (FR-37, spec 7.5, task 4.9). Experimental: the state lives
//! in an undocumented CloudStore registry blob (U-5), decoded by
//! win-nightlight-lib (D-077). `set_active` writes only the on/off State
//! value and leaves the Settings value (schedule, temperature) alone
//! (docs.rs, verified 2026-10-27). Recording Mode turns Night Light OFF for
//! true colours (D-080); the standalone switch turns it on.

use serde::{Deserialize, Serialize};

use super::{Availability, Ctx, Switch, SwitchError, SwitchId, parse_state};
use crate::os::OsInfo;

/// State shape: `{"on": bool}` (spec 5.3).
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub on: bool,
}

/// What the user must do once before the switch can work (probe finding,
/// 2026-10-27: a never-toggled Night Light holds a 13-byte stub blob that
/// does not decode).
const NEVER_USED_HINT: &str = "Turn Night light on once in Settings \u{203a} System \u{203a} Display \u{203a} Night light, then restart button.";

pub struct NightLight;

impl Switch for NightLight {
    fn id(&self) -> SwitchId {
        SwitchId::NightLight
    }

    fn experimental(&self) -> bool {
        true // undocumented CloudStore blob (D-013)
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available // the probe decides (spec 7.6)
    }

    fn persistent(&self) -> bool {
        true // the registry blob outlives the process
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        let state = read_state()?;
        Ok(serde_json::json!(State {
            on: state.is_active(),
        }))
    }

    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "night light")
            .map(|s| s.on)
            .unwrap_or(false)
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!(State { on: true })
    }

    fn target_off(&self, _current: &serde_json::Value, _ctx: &Ctx) -> Option<serde_json::Value> {
        Some(serde_json::json!(State { on: false }))
    }

    /// Recording Mode wants true colours: Night Light OFF (D-080).
    fn recording_target(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!(State { on: false })
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "night light")?;
        win_nightlight_lib::Nightlight::new()
            .set_active(target.on)
            .map_err(|e| SwitchError::NotAvailable(e.to_string()))?;
        Ok(())
    }

    // restore keeps the trait default: set `before` back only when the
    // current state still equals `after`.

    fn probe(&self) -> Result<(), String> {
        let state = read_state().map_err(|_| NEVER_USED_HINT.to_string())?;
        if state.is_usable() {
            Ok(())
        } else {
            Err(NEVER_USED_HINT.to_string())
        }
    }
}

fn read_state() -> Result<win_nightlight_lib::State, SwitchError> {
    win_nightlight_lib::Nightlight::new()
        .read_state()
        .map_err(|e| SwitchError::NotAvailable(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shape_round_trips() {
        assert!(
            parse_state::<State>(&serde_json::json!({ "on": true }), "t")
                .unwrap()
                .on
        );
        assert!(parse_state::<State>(&serde_json::json!({}), "t").is_err());
    }

    #[test]
    fn is_on_reads_the_state_object() {
        assert!(NightLight.is_on(&serde_json::json!({ "on": true })));
        assert!(!NightLight.is_on(&serde_json::json!({ "on": false })));
        assert!(
            !NightLight.is_on(&serde_json::json!(true)),
            "the state shape is an object, not a bool"
        );
    }

    #[test]
    fn night_light_is_experimental_and_recording_turns_it_off() {
        assert!(NightLight.experimental());
        assert_eq!(
            NightLight.recording_target(&serde_json::json!({ "on": true }), &Ctx::default()),
            serde_json::json!({ "on": false }),
            "D-080: recording wants true colours"
        );
        assert_eq!(
            NightLight.target_on(&serde_json::json!({ "on": false }), &Ctx::default()),
            serde_json::json!({ "on": true }),
            "the standalone switch turns it on"
        );
        assert_eq!(
            NightLight
                .target_off(&serde_json::json!({ "on": true }), &Ctx::default())
                .unwrap(),
            serde_json::json!({ "on": false })
        );
    }

    #[test]
    fn the_probe_fails_kindly_when_never_used() {
        // On CI the registry key does not exist: the probe must fail with
        // the actionable hint, never panic.
        if let Err(reason) = NightLight.probe() {
            assert_eq!(reason, NEVER_USED_HINT);
        }
    }
}
