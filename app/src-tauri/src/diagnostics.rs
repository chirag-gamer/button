//! The diagnostic report (FR-24, spec 8): app version, Windows build,
//! every switch's availability and the last 200 log lines, copied to the
//! clipboard from Settings > About or the tray menu.
//!
//! The builder only takes those inputs, so window titles can't get in
//! (the logs never hold them either, spec 8). Log lines go through
//! `redact` once more, so a path that slipped into a log line is cut out.

use std::borrow::Cow;

use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

use crate::os::OsInfo;
use crate::{SharedState, SwitchState};

/// How many log lines the report keeps (FR-24 "recent log lines").
pub const LOG_LINES: usize = 200;

/// What replaces a path in a log line.
const PATH_REMOVED: &str = "<path removed>";

/// Builds the report text. Pure, so the field list and the redaction are
/// unit-tested (FR-24 acceptance).
pub fn build_report(version: &str, os: OsInfo, switches: &[SwitchState], log_text: &str) -> String {
    let mut out = String::new();
    out.push_str("button diagnostic report\n");
    out.push_str(&format!("Version: {version}\n"));
    out.push_str(&format!(
        "Windows build: {} ({})\n",
        os.build,
        if os.is_windows_11 {
            "Windows 11"
        } else {
            "Windows 10"
        }
    ));
    out.push_str("\nSwitches:\n");
    for row in switches {
        let availability = serde_json::to_value(row.availability)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        out.push_str(&format!(
            "- {}: {}, {}{}",
            row.id,
            availability,
            if row.on { "on" } else { "off" },
            if row.experimental {
                ", experimental"
            } else {
                ""
            }
        ));
        if let Some(detail) = &row.detail {
            out.push_str(&format!(" ({})", redact(detail)));
        }
        out.push('\n');
    }
    let lines: Vec<&str> = log_text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(LOG_LINES)..];
    out.push_str(&format!("\nLast {} log lines:\n", tail.len()));
    for line in tail {
        out.push_str(&redact(line));
        out.push('\n');
    }
    out
}

/// Cuts a line at the first path-shaped text (FR-24: no user file paths):
/// a drive path (`C:\`, `C:/`) or a UNC or device path (`\\`). Everything
/// from there to the end of the line goes, because paths can hold spaces.
/// A drive letter must start a word, so `https://` is not a path.
pub fn redact(line: &str) -> Cow<'_, str> {
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        let unc = bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'\\');
        let drive = bytes[i].is_ascii_alphabetic()
            && bytes.get(i + 1) == Some(&b':')
            && matches!(bytes.get(i + 2), Some(b'\\') | Some(b'/'))
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric());
        if unc || drive {
            // i is a char boundary: both checks start at an ASCII byte.
            return Cow::Owned(format!("{}{PATH_REMOVED}", &line[..i]));
        }
    }
    Cow::Borrowed(line)
}

/// Builds the report from the live state and the current log file, then
/// copies it. The error is the user-facing reason (OS_ERROR).
pub fn copy_report(app: &AppHandle) -> Result<(), String> {
    let (os, switches) = {
        let state = app.state::<SharedState>();
        let b = state.lock().unwrap_or_else(|p| p.into_inner());
        (b.os, b.engine.switch_states())
    };
    // tauri-plugin-log's LogDir target writes <productName>.log (spec 8).
    let log_text = app
        .path()
        .app_log_dir()
        .ok()
        .and_then(|dir| std::fs::read(dir.join("button.log")).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let report = build_report(
        app.package_info().version.to_string().as_str(),
        os,
        &switches,
        &log_text,
    );
    // OpenClipboard(NULL) makes SetClipboardData fail (OpenClipboard
    // docs), so the hidden dropdown window owns the clipboard data.
    let owner = app
        .get_webview_window("dropdown")
        .and_then(|w| w.hwnd().ok())
        .map(|h| HWND(h.0))
        .ok_or_else(|| "The diagnostic report could not be copied: no window".to_string())?;
    set_clipboard(owner, &report)
        .map_err(|e| format!("The diagnostic report could not be copied: {e}"))?;
    log::info!("diagnostic report copied ({} bytes)", report.len());
    Ok(())
}

/// CF_UNICODETEXT ("Standard Clipboard Formats", Win32 docs).
const CF_UNICODETEXT: u32 = 13;

/// Puts `text` on the clipboard as CF_UNICODETEXT. Another app may hold
/// the clipboard for a moment, so opening is tried a few times.
fn set_clipboard(owner: HWND, text: &str) -> windows::core::Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: plain Win32 clipboard calls. The global block is written
    // only between GlobalLock and GlobalUnlock and within its size; after
    // a successful SetClipboardData the system owns it, otherwise it is
    // freed here. The clipboard is always closed again.
    unsafe {
        let mut opened = OpenClipboard(Some(owner));
        for _ in 0..4 {
            if opened.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
            opened = OpenClipboard(Some(owner));
        }
        opened?;
        let result = (|| {
            EmptyClipboard()?;
            let block = GlobalAlloc(GMEM_MOVEABLE, wide.len() * std::mem::size_of::<u16>())?;
            let target = GlobalLock(block) as *mut u16;
            if target.is_null() {
                let error = windows::core::Error::from_thread();
                let _ = GlobalFree(Some(block));
                return Err(error);
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
            // Returns an "error" once the lock count reaches zero.
            let _ = GlobalUnlock(block);
            if let Err(error) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(block.0))) {
                let _ = GlobalFree(Some(block));
                return Err(error);
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::switches::{Availability, SwitchId};

    fn rows() -> Vec<SwitchState> {
        vec![
            SwitchState {
                id: SwitchId::DesktopIcons,
                on: true,
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
            SwitchState {
                id: SwitchId::Dnd,
                on: false,
                availability: Availability::ProbeFailed,
                experimental: true,
                off_at: None,
                detail: Some(r"probe read C:\Users\Someone\x.dat failed".into()),
            },
        ]
    }

    const OS: OsInfo = OsInfo {
        build: 19045,
        is_windows_11: false,
    };

    // FR-24 acceptance: the listed fields are all there.
    #[test]
    fn report_lists_version_os_and_every_switch() {
        let report = build_report("1.2.3", OS, &rows(), "first line\nsecond line\n");
        assert!(report.contains("Version: 1.2.3"), "{report}");
        assert!(
            report.contains("Windows build: 19045 (Windows 10)"),
            "{report}"
        );
        assert!(
            report.contains("- desktop-icons: available, on\n"),
            "{report}"
        );
        assert!(
            report.contains("- tray-icons: needs-windows-11, off (Needs Windows 11)\n"),
            "{report}"
        );
        assert!(
            report.contains("- dnd: probe-failed, off, experimental (probe read <path removed>)\n"),
            "{report}"
        );
        assert!(
            report.contains("Last 2 log lines:\nfirst line\nsecond line\n"),
            "{report}"
        );
    }

    #[test]
    fn report_keeps_only_the_last_200_log_lines() {
        let log: String = (1..=250).map(|n| format!("line {n}\n")).collect();
        let report = build_report("1.0.0", OS, &[], &log);
        assert!(
            report.contains("Last 200 log lines:\nline 51\n"),
            "{report}"
        );
        assert!(!report.contains("line 50\n"), "{report}");
        assert!(report.ends_with("line 250\n"), "{report}");
    }

    // FR-24: no user file paths and no window titles. The builder has no
    // title input; a log line with a path loses everything from the path on.
    #[test]
    fn paths_are_cut_from_log_lines() {
        let log = concat!(
            "[2026-10-30][10:00:00][button_lib][INFO] picture C:\\Users\\Jane Doe\\Pictures\\a b.png missing\n",
            "[2026-10-30][10:00:01][button_lib][WARN] share \\\\server\\share\\file.txt\n",
            "[2026-10-30][10:00:02][button_lib][INFO] forward d:/Videos/clip.mp4\n",
            "[2026-10-30][10:00:03][button_lib][INFO] device \\\\?\\C:\\x\n",
        );
        let report = build_report("1.0.0", OS, &[], log);
        assert!(!report.contains("Jane"), "{report}");
        assert!(!report.contains("Pictures"), "{report}");
        assert!(!report.contains("server"), "{report}");
        assert!(!report.contains("Videos"), "{report}");
        assert!(
            report.contains("[INFO] picture <path removed>\n"),
            "{report}"
        );
        assert!(report.contains("[WARN] share <path removed>\n"), "{report}");
        assert!(
            report.contains("[INFO] forward <path removed>\n"),
            "{report}"
        );
        assert!(
            report.contains("[INFO] device <path removed>\n"),
            "{report}"
        );
    }

    #[test]
    fn non_paths_are_kept() {
        let line = "[2026-10-30][10:00:00][tauri_plugin_updater][INFO] GET https://github.com/x 12:30:01 a:b";
        assert_eq!(redact(line), line);
        assert_eq!(redact("ratio 16:9"), "ratio 16:9");
        assert_eq!(redact("C:\\"), PATH_REMOVED);
    }
}
