//! journal.json: the file exists only while there is something to undo (spec 5.2).

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::switches::SwitchId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JournalOwner {
    Recording,
    Timer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub seq: u64,
    pub owner: JournalOwner,
    pub switch: SwitchId,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
    pub persistent: bool,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub off_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub journal_version: u32,
    pub app_version: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    pub entries: Vec<JournalEntry>,
}

impl Journal {
    pub fn new(app_version: &str, started_at: OffsetDateTime) -> Self {
        Self {
            journal_version: 1,
            app_version: app_version.to_string(),
            started_at,
            entries: Vec::new(),
        }
    }

    /// The next sequence number; empty journal starts at 1.
    pub fn next_seq(&self) -> u64 {
        self.entries.last().map_or(1, |e| e.seq + 1)
    }
}

/// What happened when reading `journal.json` (spec 7.3).
#[derive(Debug)]
pub enum JournalRead {
    /// No journal file exists.
    Missing,
    /// The file parsed. Old journalVersions stay readable (spec 7.12).
    Ok(Box<Journal>),
    /// The file exists but doesn't parse.
    Corrupt,
}

const JOURNAL_FILE: &str = "journal.json";

pub fn journal_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join(JOURNAL_FILE)
}

/// Parse the journal file if it exists (spec 5.2, 7.3).
pub fn read(data_dir: &std::path::Path) -> JournalRead {
    let path = journal_path(data_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return JournalRead::Missing,
        Err(e) => panic!("reading {}: {e}", path.display()),
    };
    match serde_json::from_str::<Journal>(&text) {
        Ok(journal) => JournalRead::Ok(Box::new(journal)),
        Err(_) => JournalRead::Corrupt,
    }
}

/// Rename a corrupt journal out of the way (spec 7.3): journal.corrupt-<timestamp>.json
pub fn quarantine_corrupt(data_dir: &std::path::Path, now: OffsetDateTime) -> std::io::Result<()> {
    let from = journal_path(data_dir);
    let stamp = now.unix_timestamp();
    let to = data_dir.join(format!("journal.corrupt-{stamp}.json"));
    std::fs::rename(from, to)
}

/// Atomic write (spec 7.4): serialize, then the shared tmp + sync + rename
/// protocol (lib::write_atomic).
pub fn write(journal: &Journal, data_dir: &std::path::Path) -> std::io::Result<()> {
    let text = serde_json::to_string(journal).expect("journal serializes");
    crate::write_atomic(&data_dir.join(JOURNAL_FILE), text.as_bytes())
}

/// Delete the journal once everything is restored (spec 7.2).
pub fn delete(data_dir: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(journal_path(data_dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use time::macros::datetime;

    fn entry(seq: u64, persistent: bool) -> JournalEntry {
        JournalEntry {
            seq,
            owner: JournalOwner::Recording,
            switch: SwitchId::Taskbar,
            before: json!({ "autoHide": false }),
            after: json!({ "autoHide": true }),
            persistent,
            off_at: None,
            at: datetime!(2026-10-26 10:00:00 UTC),
        }
    }

    // Unique dir under the system temp dir (D-063 instruction: no tempfile crate).
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("button-journal-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_journal() -> Journal {
        let mut journal = Journal::new("0.1.0", datetime!(2026-10-26 10:00:00 UTC));
        journal.entries.push(entry(1, true));
        journal
    }

    #[test]
    fn roundtrip_preserves_entries_and_times() {
        let dir = test_dir("roundtrip");
        write(&sample_journal(), &dir).unwrap();
        // Write again: rename must replace the existing target on Windows.
        write(&sample_journal(), &dir).unwrap();

        match read(&dir) {
            JournalRead::Ok(j) => {
                assert_eq!(j.journal_version, 1);
                assert_eq!(j.app_version, "0.1.0");
                assert_eq!(j.entries.len(), 1);
                assert_eq!(j.entries[0].switch, SwitchId::Taskbar);
                assert_eq!(j.entries[0].at, datetime!(2026-10-26 10:00:00 UTC));
                assert_eq!(j.next_seq(), 2);
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn missing_file_reads_as_missing() {
        let dir = test_dir("missing");
        assert!(matches!(read(&dir), JournalRead::Missing));
        // delete() on a missing journal is a no-op.
        delete(&dir).unwrap();
    }

    #[test]
    fn corrupt_file_reads_as_corrupt_and_can_be_quarantined() {
        let dir = test_dir("corrupt");
        fs::write(dir.join(JOURNAL_FILE), "{ not json").unwrap();
        assert!(matches!(read(&dir), JournalRead::Corrupt));
        quarantine_corrupt(&dir, datetime!(2026-10-26 10:00:00 UTC)).unwrap();
        assert!(!dir.join(JOURNAL_FILE).exists());
        let stamp = datetime!(2026-10-26 10:00:00 UTC).unix_timestamp();
        assert!(dir.join(format!("journal.corrupt-{stamp}.json")).exists());
    }

    #[test]
    fn leftover_tmp_file_is_ignored() {
        let dir = test_dir("tmp");
        fs::write(dir.join("journal.json.tmp"), "half written").unwrap();
        assert!(matches!(read(&dir), JournalRead::Missing));
        // A real write removes the stale tmp file by renaming over it.
        write(&sample_journal(), &dir).unwrap();
        assert!(!dir.join("journal.json.tmp").exists());
        assert!(matches!(read(&dir), JournalRead::Ok(_)));
    }

    #[test]
    fn old_journal_version_stays_readable() {
        let dir = test_dir("old-version");
        let old = r#"{"journalVersion":0,"appVersion":"0.0.9","startedAt":"2026-01-01T00:00:00Z","entries":[]}"#;
        fs::write(dir.join(JOURNAL_FILE), old).unwrap();
        match read(&dir) {
            JournalRead::Ok(j) => assert_eq!(j.journal_version, 0),
            other => panic!("expected Ok, got {other:?}"),
        }
    }
}
