//! Mute Microphone (FR-33, spec 7.5, task 4.5). Mutes the default capture
//! endpoint via `IAudioEndpointVolume::SetMute`. The journal records the
//! concrete endpoint id, and apply/restore address that endpoint through
//! `GetDevice` — not "whatever is default now" — so switching the default
//! mic mid-session never mutes the wrong device. Needs COM: each call
//! wraps itself in the thread-affine `ComGuard`.

use serde::{Deserialize, Serialize};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eCapture, eConsole};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::PCWSTR;

use super::{Availability, Ctx, RestoreOutcome, Switch, SwitchError, SwitchId, parse_state};
use crate::com::ComGuard;
use crate::journal::JournalEntry;
use crate::os::OsInfo;

/// State shape: `{"endpointId": str, "muted": bool}` (spec 5.3).
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub endpoint_id: String,
    pub muted: bool,
}

pub struct Microphone;

impl Switch for Microphone {
    fn id(&self) -> SwitchId {
        SwitchId::Microphone
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available // read() reports "no microphone" when empty
    }

    fn persistent(&self) -> bool {
        true // the mute lives in the audio service, not button's process
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        let _guard = ComGuard::new()?;
        let (endpoint_id, muted) = default_mic()?;
        Ok(serde_json::json!(State { endpoint_id, muted }))
    }

    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "microphone")
            .map(|s| s.muted)
            .unwrap_or(false)
    }

    fn target_on(&self, before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        // Keep the endpoint the read saw, so the journal entry is
        // self-consistent even if the default changes before apply.
        let endpoint_id = before
            .get("endpointId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        serde_json::json!(State {
            endpoint_id,
            muted: true,
        })
    }

    fn target_off(&self, current: &serde_json::Value, _ctx: &Ctx) -> Option<serde_json::Value> {
        let endpoint_id = current
            .get("endpointId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        Some(serde_json::json!(State {
            endpoint_id,
            muted: false,
        }))
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "microphone")?;
        let _guard = ComGuard::new()?;
        let volume = endpoint_volume(&target.endpoint_id)?;
        unsafe {
            volume
                .SetMute(target.muted, std::ptr::null())
                .map_err(os("IAudioEndpointVolume::SetMute"))?;
        }
        Ok(())
    }

    fn restore(&self, e: &JournalEntry) -> Result<RestoreOutcome, SwitchError> {
        let before = parse_state::<State>(&e.before, "microphone")?;
        let after = parse_state::<State>(&e.after, "microphone")?;
        let _guard = ComGuard::new()?;
        let Ok(volume) = endpoint_volume(&before.endpoint_id) else {
            return Ok(RestoreOutcome::Gone); // device unplugged/removed
        };
        let muted = unsafe {
            volume
                .GetMute()
                .map_err(os("IAudioEndpointVolume::GetMute"))?
        };
        if muted.as_bool() != after.muted {
            return Ok(RestoreOutcome::UserChanged);
        }
        unsafe {
            volume
                .SetMute(before.muted, std::ptr::null())
                .map_err(os("IAudioEndpointVolume::SetMute"))?;
        }
        Ok(RestoreOutcome::Restored)
    }
}

fn os(api: &'static str) -> impl Fn(windows::core::Error) -> SwitchError {
    move |e| SwitchError::Os {
        api,
        code: e.code().0,
    }
}

/// The default capture endpoint's id and mute flag. Console role: on this
/// machine console == communications (probe 2026-10-27).
fn default_mic() -> Result<(String, bool), SwitchError> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(os("CoCreateInstance"))?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eCapture, eConsole)
            .map_err(|_| SwitchError::NotAvailable("No microphone".into()))?;
        let id = device.GetId().map_err(os("IMMDevice::GetId"))?;
        // Endpoint ids are ASCII braces-GUID strings (probe, 2026-10-27).
        let text = id.to_string().unwrap_or_default();
        windows::Win32::System::Com::CoTaskMemFree(Some(id.0 as *const _));
        let volume: IAudioEndpointVolume = device
            .Activate(CLSCTX_ALL, None)
            .map_err(os("IMMDevice::Activate"))?;
        let muted = volume
            .GetMute()
            .map_err(os("IAudioEndpointVolume::GetMute"))?
            .as_bool();
        Ok((text, muted))
    }
}

/// `IAudioEndpointVolume` of a specific endpoint id (GetDevice round trip
/// proven by the probe).
fn endpoint_volume(endpoint_id: &str) -> Result<IAudioEndpointVolume, SwitchError> {
    unsafe {
        let wide: Vec<u16> = endpoint_id.encode_utf16().chain([0]).collect();
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(os("CoCreateInstance"))?;
        let device = enumerator
            .GetDevice(PCWSTR(wide.as_ptr()))
            .map_err(os("IMMDeviceEnumerator::GetDevice"))?;
        device
            .Activate(CLSCTX_ALL, None)
            .map_err(os("IMMDevice::Activate"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shape_round_trips() {
        let state = parse_state::<State>(
            &serde_json::json!({ "endpointId": "{0.0.0.0}ep", "muted": true }),
            "t",
        )
        .unwrap();
        assert!(state.muted);
        assert!(parse_state::<State>(&serde_json::json!(true), "t").is_err());
        assert!(parse_state::<State>(&serde_json::json!({ "muted": true }), "t").is_err());
    }

    #[test]
    fn is_on_means_muted() {
        assert!(Microphone.is_on(&serde_json::json!({"endpointId": "x", "muted": true})));
        assert!(!Microphone.is_on(&serde_json::json!({"endpointId": "x", "muted": false})));
    }

    #[test]
    fn targets_keep_the_endpoint_and_flip_mute() {
        let before = serde_json::json!({ "endpointId": "{ep}", "muted": false });
        assert_eq!(
            Microphone.target_on(&before, &Ctx::default()),
            serde_json::json!({ "endpointId": "{ep}", "muted": true })
        );
        let current = serde_json::json!({ "endpointId": "{ep}", "muted": true });
        assert_eq!(
            Microphone.target_off(&current, &Ctx::default()).unwrap(),
            serde_json::json!({ "endpointId": "{ep}", "muted": false })
        );
    }

    /// Read-only live probe (the mute itself is M-1). A machine with no
    /// microphone surfaces as NotAvailable.
    #[test]
    fn read_reports_the_live_mic_or_reports_none() {
        match Microphone.read() {
            Ok(value) => {
                parse_state::<State>(&value, "t").unwrap();
            }
            Err(SwitchError::NotAvailable(_)) => {}
            Err(SwitchError::Os { .. }) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
