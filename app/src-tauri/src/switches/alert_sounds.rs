//! Mute Alert Sounds (FR-19, spec 7.5). Walks the sessions of the default
//! render endpoint, mutes the ones that report
//! `IAudioSessionControl2::IsSystemSoundsSession() == S_OK` via
//! `ISimpleAudioVolume::SetMute` — other audio stays untouched. Needs COM:
//! each call wraps itself in the thread-affine `ComGuard`.

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::{
    IAudioSessionControl2, IAudioSessionEnumerator, IAudioSessionManager2, IMMDeviceEnumerator,
    ISimpleAudioVolume, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::Interface;

use super::{Availability, Ctx, Switch, SwitchError, SwitchId, parse_state};
use crate::com::ComGuard;
use crate::os::OsInfo;

/// State shape: `{"muted": bool}` — the aggregate over the system-sound
/// sessions found right now (there can be several, research/02; they are
/// not stable enough to journal individually).
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub muted: bool,
}

pub struct AlertSounds;

impl Switch for AlertSounds {
    fn id(&self) -> SwitchId {
        SwitchId::AlertSounds
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available
    }

    fn persistent(&self) -> bool {
        false // a live per-session mute; gone with the process (spec 7.3)
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        let _guard = ComGuard::new()?;
        let volumes = system_sound_volumes()?;
        let mut muted = !volumes.is_empty();
        for volume in &volumes {
            unsafe {
                if !volume
                    .GetMute()
                    .map_err(os("ISimpleAudioVolume::GetMute"))?
                    .as_bool()
                {
                    muted = false;
                }
            }
        }
        Ok(serde_json::json!(State { muted }))
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!(State { muted: true })
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "alert sounds")?;
        let _guard = ComGuard::new()?;
        for volume in system_sound_volumes()? {
            unsafe {
                volume
                    .SetMute(target.muted, std::ptr::null())
                    .map_err(os("ISimpleAudioVolume::SetMute"))?;
            }
        }
        Ok(())
    }

    // restore keeps the trait default: unmute only when everything is
    // still muted by us.

    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "alert sounds")
            .map(|state| state.muted)
            .unwrap_or(false)
    }
}

fn os(api: &'static str) -> impl Fn(windows::core::Error) -> SwitchError {
    move |e| SwitchError::Os {
        api,
        code: e.code().0,
    }
}

/// `ISimpleAudioVolume` for every system-sounds session on the default
/// render endpoint.
fn system_sound_volumes() -> Result<Vec<ISimpleAudioVolume>, SwitchError> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(os("CoCreateInstance"))?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(os("GetDefaultAudioEndpoint"))?;
        let manager: IAudioSessionManager2 = device
            .Activate(CLSCTX_ALL, None)
            .map_err(os("IMMDevice::Activate"))?;
        let sessions: IAudioSessionEnumerator = manager
            .GetSessionEnumerator()
            .map_err(os("GetSessionEnumerator"))?;
        let count = sessions
            .GetCount()
            .map_err(os("IAudioSessionEnumerator::GetCount"))?;
        let mut volumes = Vec::new();
        for i in 0..count {
            let control = sessions.GetSession(i).map_err(os("GetSession"))?;
            let control2: IAudioSessionControl2 =
                control.cast().map_err(os("IAudioSessionControl2"))?;
            if control2.IsSystemSoundsSession() == S_OK {
                let volume: ISimpleAudioVolume =
                    control.cast().map_err(os("ISimpleAudioVolume"))?;
                volumes.push(volume);
            }
        }
        Ok(volumes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shape_is_muted_only() {
        let state = parse_state::<State>(&serde_json::json!({ "muted": true }), "t").unwrap();
        assert!(state.muted);
        assert!(parse_state::<State>(&serde_json::json!(true), "t").is_err());
        assert!(parse_state::<State>(&serde_json::json!({}), "t").is_err());
    }

    #[test]
    fn is_on_means_system_sounds_muted() {
        assert!(AlertSounds.is_on(&serde_json::json!(State { muted: true })));
        assert!(!AlertSounds.is_on(&serde_json::json!(State { muted: false })));
    }

    #[test]
    fn target_on_is_muted() {
        let after =
            AlertSounds.target_on(&serde_json::json!(State { muted: false }), &Ctx::default());
        assert_eq!(after, serde_json::json!(State { muted: true }));
    }

    /// Read-only live probe: GetMute changes nothing. A machine without a
    /// default render device (CI) surfaces as an Os error and is fine.
    #[test]
    fn read_reports_the_live_mute_or_an_audio_error() {
        match AlertSounds.read() {
            Ok(value) => {
                parse_state::<State>(&value, "t").unwrap();
            }
            Err(SwitchError::Os { .. }) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
