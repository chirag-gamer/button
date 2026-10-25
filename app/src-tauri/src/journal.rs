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
