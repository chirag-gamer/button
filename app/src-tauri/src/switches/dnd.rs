//! Do Not Disturb (FR-36, spec 7.5, task 4.8). Experimental: the
//! `IQuietHoursSettings` COM interface (UNDOC) is reverse-engineered
//! (Rafael Rivera's gist; verified on build 26200 by versailles/yasb and
//! our probe, 2026-10-27). `put_UserSelectedProfile` switches between
//! "Microsoft.QuietHoursProfile.PriorityOnly" (on) and
//! "Microsoft.QuietHoursProfile.Unrestricted" (off); the object lives in
//! another process (CLSCTX_LOCAL_SERVER). A read-only startup probe gates
//! the switch (D-013, FR-35, U-2).

// The interface's vtable methods carry their Win32 names (the macro
// generates the vtable from the trait, so the casing is load-bearing).
#![allow(non_snake_case)]

use serde::{Deserialize, Serialize};
use windows::Win32::System::Com::{CLSCTX_LOCAL_SERVER, CoCreateInstance};
use windows::core::{GUID, HRESULT, PCWSTR, PWSTR};

use super::{Availability, Ctx, Switch, SwitchError, SwitchId, parse_state};
use crate::com::ComGuard;
use crate::os::OsInfo;

const QUIET_HOURS_SETTINGS: GUID = GUID::from_u128(0xf53321fa_34f8_4b7f_b9a3_361877cb94cf);

/// `6bff4732-81ec-4ffb-ae67-b6c1bc29631f` — vtable order from the probe
/// (get_UserSelectedProfile / put_UserSelectedProfile must come first).
#[windows_core::interface("6bff4732-81ec-4ffb-ae67-b6c1bc29631f")]
pub unsafe trait IQuietHoursSettings: windows::core::IUnknown {
    fn get_UserSelectedProfile(&self, profile: *mut PWSTR) -> HRESULT;
    fn put_UserSelectedProfile(&self, profile: PCWSTR) -> HRESULT;
    fn GetProfile(&self, id: PCWSTR, out: *mut *mut core::ffi::c_void) -> HRESULT;
    fn GetAllProfileData(&self, count: *mut u32, data: *mut *mut core::ffi::c_void) -> HRESULT;
    fn GetDisplayNameForProfile(&self, id: PCWSTR, out: *mut PWSTR) -> HRESULT;
    fn get_QuietMomentsManager(&self, out: *mut *mut core::ffi::c_void) -> HRESULT;
    fn get_OffProfileId(&self, out: *mut PWSTR) -> HRESULT;
    fn get_ActiveQuietMomentProfile(&self, out: *mut PWSTR) -> HRESULT;
    fn put_ActiveQuietMomentProfile(&self, id: PCWSTR) -> HRESULT;
    fn get_ActiveProfile(&self, out: *mut PWSTR) -> HRESULT;
}

const PRIORITY_ONLY: &str = "Microsoft.QuietHoursProfile.PriorityOnly";
const UNRESTRICTED: &str = "Microsoft.QuietHoursProfile.Unrestricted";
const ALARMS_ONLY: &str = "Microsoft.QuietHoursProfile.AlarmsOnly";

/// State shape: `{"profile": str}` (spec 5.3).
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub profile: String,
}

pub struct Dnd;

impl Switch for Dnd {
    fn id(&self) -> SwitchId {
        SwitchId::Dnd
    }

    fn experimental(&self) -> bool {
        true // undocumented COM server (D-013)
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available // the probe decides (spec 7.6)
    }

    fn persistent(&self) -> bool {
        true // the profile lives in Windows' own state, not our process
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        let _guard = ComGuard::new()?;
        let profile = read_profile()?;
        Ok(serde_json::json!(State { profile }))
    }

    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "dnd")
            .map(|s| s.profile != UNRESTRICTED)
            .unwrap_or(false)
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!(State {
            profile: PRIORITY_ONLY.into(),
        })
    }

    fn target_off(&self, _current: &serde_json::Value, _ctx: &Ctx) -> Option<serde_json::Value> {
        Some(serde_json::json!(State {
            profile: UNRESTRICTED.into(),
        }))
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "dnd")?;
        let _guard = ComGuard::new()?;
        let settings = create()?;
        let wide: Vec<u16> = target.profile.encode_utf16().chain([0]).collect();
        unsafe {
            settings
                .put_UserSelectedProfile(PCWSTR(wide.as_ptr()))
                .ok()
                .map_err(os("IQuietHoursSettings::put_UserSelectedProfile"))?;
        }
        Ok(())
    }

    // restore keeps the trait default: put the recorded profile back only
    // when the current one is still what we set (a user change wins).

    fn probe(&self) -> Result<(), String> {
        let guard = ComGuard::new().map_err(|e| e.to_string())?;
        let settings = create().map_err(|e| e.to_string())?;
        let profile = read_profile_of(&settings).map_err(|e| e.to_string())?;
        drop(guard);
        match profile.as_str() {
            UNRESTRICTED | PRIORITY_ONLY | ALARMS_ONLY => Ok(()),
            other => Err(format!("unknown DND profile '{other}'")),
        }
    }
}

fn os(api: &'static str) -> impl Fn(windows::core::Error) -> SwitchError {
    move |e| SwitchError::Os {
        api,
        code: e.code().0,
    }
}

fn create() -> Result<IQuietHoursSettings, SwitchError> {
    unsafe { CoCreateInstance(&QUIET_HOURS_SETTINGS, None, CLSCTX_LOCAL_SERVER) }
        .map_err(os("CoCreateInstance(IQuietHoursSettings)"))
}

/// The currently selected DND profile (CoTaskMemFree'd PWSTR).
fn read_profile() -> Result<String, SwitchError> {
    let settings = create()?;
    read_profile_of(&settings)
}

fn read_profile_of(settings: &IQuietHoursSettings) -> Result<String, SwitchError> {
    unsafe {
        let mut out = PWSTR::null();
        settings
            .get_UserSelectedProfile(&mut out)
            .ok()
            .map_err(os("IQuietHoursSettings::get_UserSelectedProfile"))?;
        // Profile ids are ASCII GUID strings; a broken surrogate pair can
        // only mean a corrupted value, which the probe's known-list check
        // then reports.
        let text = out.to_string().unwrap_or_default();
        windows::Win32::System::Com::CoTaskMemFree(Some(out.0 as *const _));
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shape_round_trips() {
        let state =
            parse_state::<State>(&serde_json::json!({ "profile": PRIORITY_ONLY }), "t").unwrap();
        assert_eq!(state.profile, PRIORITY_ONLY);
        assert!(parse_state::<State>(&serde_json::json!(true), "t").is_err());
    }

    #[test]
    fn is_on_means_anything_but_unrestricted() {
        assert!(Dnd.is_on(&serde_json::json!({ "profile": PRIORITY_ONLY })));
        assert!(Dnd.is_on(&serde_json::json!({ "profile": ALARMS_ONLY })));
        assert!(!Dnd.is_on(&serde_json::json!({ "profile": UNRESTRICTED })));
    }

    #[test]
    fn targets_are_priority_only_and_unrestricted() {
        assert_eq!(
            Dnd.target_on(&serde_json::json!({}), &Ctx::default()),
            serde_json::json!({ "profile": PRIORITY_ONLY })
        );
        assert_eq!(
            Dnd.target_off(&serde_json::json!({}), &Ctx::default())
                .unwrap(),
            serde_json::json!({ "profile": UNRESTRICTED })
        );
    }

    #[test]
    fn dnd_is_experimental_and_the_probe_is_read_only() {
        assert!(Dnd.experimental());
        // On CI there is no notification server: the probe may fail, but it
        // must never change state — its only calls are create + get.
        let _ = Dnd.probe();
    }

    /// Read-only live probe (the banner check with the owner is U-2's
    /// manual part, docs/testing.md). CI machines surface as an Os error.
    #[test]
    fn read_reports_the_live_profile_or_an_os_error() {
        match Dnd.read() {
            Ok(value) => {
                parse_state::<State>(&value, "t").unwrap();
            }
            Err(SwitchError::Os { .. }) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
