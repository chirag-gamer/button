//! Backdrop switch (FR-20, spec 7.9). The switch decides when the
//! backdrop shows; the window work lives in the Shell module
//! `crate::backdrop` (spec 2.1: switches change system settings and
//! windows, the Shell manages button's own windows).

use serde::{Deserialize, Serialize};

use super::{Availability, Ctx, Switch, SwitchError, SwitchId, parse_state};
use crate::os::OsInfo;

/// State shape: `{"shown": bool}` (spec 7.5). The colour never enters the
/// journal — it lives in settings and is applied to the windows at
/// creation.
#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub shown: bool,
}

pub struct Backdrop;

impl Switch for Backdrop {
    fn id(&self) -> SwitchId {
        SwitchId::Backdrop
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available
    }

    fn persistent(&self) -> bool {
        false // the windows die with the process; recovery drops the entry
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        Ok(serde_json::json!(State {
            shown: crate::backdrop::shown(),
        }))
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!(State { shown: true })
    }

    fn target_off(&self, _current: &serde_json::Value, _ctx: &Ctx) -> Option<serde_json::Value> {
        Some(serde_json::json!(State { shown: false }))
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "backdrop")?;
        if target.shown {
            crate::backdrop::show().map_err(SwitchError::NotAvailable)
        } else {
            crate::backdrop::hide();
            Ok(())
        }
    }

    // restore: trait default — still shown means hide it (restored); the
    // backdrop has no user-facing surface, so UserChanged is unreachable
    // in practice.

    /// "On" reads as "the backdrop is visible".
    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "backdrop")
            .map(|state| state.shown)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shape_round_trips() {
        let state = parse_state::<State>(&serde_json::json!({ "shown": true }), "t").unwrap();
        assert!(state.shown);
        assert!(parse_state::<State>(&serde_json::json!({}), "t").is_err());
    }

    #[test]
    fn is_on_follows_shown() {
        assert!(Backdrop.is_on(&serde_json::json!({ "shown": true })));
        assert!(!Backdrop.is_on(&serde_json::json!({ "shown": false })));
        assert!(!Backdrop.is_on(&serde_json::json!({})));
    }

    #[test]
    fn target_on_is_always_shown() {
        let ctx = Ctx::default();
        let target = Backdrop.target_on(&serde_json::json!({ "shown": false }), &ctx);
        assert_eq!(target, serde_json::json!({ "shown": true }));
    }

    #[test]
    fn apply_to_false_succeeds_without_windows() {
        // Hiding what does not exist is a no-op; showing without windows
        // is an error the session reports (G-4).
        assert!(
            Backdrop
                .apply(&serde_json::json!({ "shown": false }))
                .is_ok()
        );
        assert!(
            Backdrop
                .apply(&serde_json::json!({ "shown": true }))
                .is_err()
        );
    }
}
