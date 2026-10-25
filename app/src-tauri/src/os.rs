//! OS info: Windows build and Win11 detection (spec 7.6).

use serde::{Deserialize, Serialize};
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

/// Shared with the UI as `os: { build, isWindows11 }` (spec 5.4).
/// Also the input for `Switch::availability` in tests (faked freely).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsInfo {
    pub build: u32,
    pub is_windows_11: bool,
}

pub fn detect() -> OsInfo {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // RtlGetVersion always returns STATUS_SUCCESS on Windows; failing here is impossible.
    let status = unsafe { RtlGetVersion(&mut info) };
    assert!(status.is_ok(), "RtlGetVersion failed with {:?}", status);
    OsInfo {
        build: info.dwBuildNumber,
        is_windows_11: info.dwBuildNumber >= 22000,
    }
}
