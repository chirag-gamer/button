//! Session engine: apply / restore / crash recovery (spec 7.1 to 7.3).
//! No Tauri types in here: it takes the switch list, OS info and the data
//! dir, which is what makes it unit-testable with fakes (D-033).

use std::path::PathBuf;

use serde::Serialize;
use time::OffsetDateTime;

use crate::journal::{self, Journal, JournalEntry, JournalOwner, JournalRead};
use crate::os::OsInfo;
use crate::switches::{Availability, Ctx, RECORDING_ORDER, RestoreOutcome, Switch, SwitchId};
use crate::{SessionState, SessionStatus, SkippedSwitch};

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("a session is already active")]
    AlreadyActive,
    #[error("no session is active")]
    NotActive,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What launch-time recovery did (spec 7.3); the app layer turns this into
/// the FR-6 notification and re-arms surviving timers.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReport {
    pub corrupted: bool,
    pub restored: Vec<SwitchId>,
    pub user_changed: Vec<SwitchId>,
    pub failed: Vec<SkippedSwitch>,
    /// Persistent timer entries whose offAt is still in the future.
    pub timers_to_rearm: Vec<JournalEntry>,
}

pub struct SessionEngine {
    switches: Vec<Box<dyn Switch>>,
    os: OsInfo,
    data_dir: PathBuf,
    app_version: String,
    session: SessionState,
    journal: Option<Journal>,
}

impl SessionEngine {
    pub fn new(
        switches: Vec<Box<dyn Switch>>,
        os: OsInfo,
        data_dir: PathBuf,
        app_version: String,
    ) -> Self {
        Self {
            switches,
            os,
            data_dir,
            app_version,
            session: SessionState::idle(),
            journal: None,
        }
    }

    pub fn state(&self) -> &SessionState {
        &self.session
    }

    /// Availability of one registered switch, if present.
    pub fn switch_availability(&self, id: SwitchId) -> Option<Availability> {
        self.switches
            .iter()
            .find(|s| s.id() == id)
            .map(|s| s.availability(&self.os))
    }

    /// Per-switch rows for the UI (spec 5.4). Off-timers come from the
    /// journal's timer entries; timers fire via the ticker (spec 7.7).
    pub fn switch_states(&self) -> Vec<crate::SwitchState> {
        self.switches
            .iter()
            .map(|s| {
                let off_at = self
                    .journal
                    .as_ref()
                    .and_then(|j| {
                        j.entries
                            .iter()
                            .rev()
                            .find(|e| e.owner == JournalOwner::Timer && e.switch == s.id())
                    })
                    .and_then(|e| e.off_at);
                crate::SwitchState {
                    id: s.id(),
                    on: s.read().map(|v| s.is_on(&v)).unwrap_or(false),
                    availability: s.availability(&self.os),
                    experimental: s.experimental(),
                    off_at,
                    detail: None,
                }
            })
            .collect()
    }

    /// Recording Mode apply (spec 7.1). The countdown is the app layer's job.
    /// One failing apply never stops the others (FR-7). Only the actions the
    /// user selected (settings.recordingMode.actions) are applied, in the
    /// fixed order (spec 7.1).
    pub fn start(
        &mut self,
        now: OffsetDateTime,
        selected: &[SwitchId],
        ctx: &Ctx,
    ) -> Result<&SessionState, SessionError> {
        if self.session.status != SessionStatus::Idle {
            return Err(SessionError::AlreadyActive);
        }
        self.session = SessionState {
            status: SessionStatus::Applying,
            started_at: Some(now),
            ends_at: None,
            applied: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
        };
        let journal = self
            .journal
            .get_or_insert_with(|| Journal::new(&self.app_version, now));

        for id in RECORDING_ORDER {
            if !selected.contains(&id) {
                continue; // not selected by the user (spec 7.1)
            }
            let Some(switch) = self
                .switches
                .iter()
                .find(|s| s.id() == id)
                .map(|s| s.as_ref())
            else {
                self.session.skipped.push(SkippedSwitch {
                    id,
                    reason: "not built".into(),
                });
                continue;
            };
            let availability = switch.availability(&self.os);
            if availability != Availability::Available {
                self.session.skipped.push(SkippedSwitch {
                    id,
                    reason: availability_reason(availability),
                });
                continue;
            }
            let before = match switch.read() {
                Ok(v) => v,
                Err(e) => {
                    self.session.failed.push(SkippedSwitch {
                        id,
                        reason: e.to_string(),
                    });
                    continue;
                }
            };
            let after = switch.target_on(&before, ctx);
            if before == after {
                continue; // already in that state, nothing to undo (spec 7.1.2)
            }
            let entry = JournalEntry {
                seq: journal.next_seq(),
                owner: JournalOwner::Recording,
                switch: id,
                before: before.clone(),
                after: after.clone(),
                persistent: switch.persistent(),
                off_at: None,
                at: now,
            };
            journal.entries.push(entry);
            // Journal on disk BEFORE the change (spec 7.1.3).
            if let Err(e) = journal::write(journal, &self.data_dir) {
                journal.entries.pop();
                let _ = journal::write(journal, &self.data_dir);
                // Undo everything this start applied so no change is left
                // behind without a journal entry. restore_all always
                // returns the engine to Idle; whatever survives it (future
                // timers, or entries its own writes could not remove) stays
                // in the journal for the next recovery (spec 7.3), so only
                // an empty journal is deleted.
                let _ = self.restore_all(now);
                if self.journal.as_ref().is_some_and(|j| j.entries.is_empty()) {
                    self.journal = None;
                    let _ = journal::delete(&self.data_dir);
                }
                self.session.started_at = None;
                return Err(SessionError::Io(e));
            }
            if let Err(e) = switch.apply(&after) {
                // Remove the entry again, then keep going (spec 7.1.4, FR-7).
                // Best effort: if the pop-write fails, the on-disk entry
                // describes a change that never happened and restore treats
                // it as user-reverted, so nothing is lost.
                journal.entries.pop();
                let _ = journal::write(journal, &self.data_dir);
                self.session.failed.push(SkippedSwitch {
                    id,
                    reason: e.to_string(),
                });
                continue;
            }
            self.session.applied.push(id);
        }
        self.session.status = SessionStatus::Active;
        Ok(&self.session)
    }

    /// Restore everything the journal still owns (spec 7.2). Idempotent:
    /// an empty or missing journal is a no-op, a second stop errors NotActive.
    pub fn stop(&mut self, now: OffsetDateTime) -> Result<&SessionState, SessionError> {
        if self.session.status == SessionStatus::Idle {
            return Err(SessionError::NotActive);
        }
        self.restore_all(now)?;
        Ok(&self.session)
    }

    /// Launch-time recovery (spec 7.3): drop non-persistent entries, re-arm
    /// future timers, restore the rest, quarantine a corrupt journal.
    pub fn recover(&mut self, now: OffsetDateTime) -> Result<RecoveryReport, SessionError> {
        let mut report = RecoveryReport::default();
        match journal::read(&self.data_dir) {
            JournalRead::Missing => {}
            JournalRead::Corrupt => {
                report.corrupted = true;
                journal::quarantine_corrupt(&self.data_dir, now)?;
            }
            JournalRead::Ok(mut j) => {
                // Non-persistent entries died with the old process (spec 7.3).
                j.entries.retain(|e| e.persistent);
                // Future timers survive in the journal until they fire (spec 7.7);
                // restore_all skips them.
                report.timers_to_rearm = j
                    .entries
                    .iter()
                    .filter(|e| e.off_at.is_some_and(|t| t > now))
                    .cloned()
                    .collect();
                self.journal = Some(*j);
                if self.journal.as_ref().is_some_and(|j| !j.entries.is_empty()) {
                    journal::write(self.journal.as_ref().unwrap(), &self.data_dir)?;
                } else {
                    self.journal = None;
                    journal::delete(&self.data_dir)?;
                }
                self.restore_all(now)?;
            }
        }
        report.restored = self.session.applied.clone();
        report.user_changed = self
            .session
            .skipped
            .iter()
            .filter(|s| s.reason == "user-reverted")
            .map(|s| s.id)
            .collect();
        report.failed = self.session.failed.clone();
        Ok(report)
    }

    /// Restore entries oldest-to-newest reversed (last seq first). Only
    /// entries with no deadline or a due deadline are touched; future
    /// timers stay (spec 7.7). After each entry the journal is written again, so a
    /// crash during restore resumes correctly (spec 7.2). A panic or a
    /// failing journal write mid-restore leaves the surviving entries on
    /// disk for the next recovery.
    ///
    /// Always returns the engine to Idle: a failed restore keeps the
    /// remaining entries in memory and on disk for a later recovery, and
    /// start/stop keep working.
    fn restore_all(&mut self, now: OffsetDateTime) -> Result<(), SessionError> {
        if self.journal.is_none() {
            self.session.status = SessionStatus::Idle;
            return Ok(());
        }
        self.session.status = SessionStatus::Restoring;
        let result = self.restore_entries(now);
        self.session.status = SessionStatus::Idle;
        result
    }

    fn restore_entries(&mut self, now: OffsetDateTime) -> Result<(), SessionError> {
        while let Some(idx) = self.journal.as_ref().and_then(|j| {
            j.entries
                .iter()
                .rposition(|e| e.off_at.is_none_or(|t| t <= now))
        }) {
            let entry = self.journal.as_ref().unwrap().entries[idx].clone();
            self.restore_one(&entry);
            let journal = self.journal.as_mut().unwrap();
            journal.entries.remove(idx);
            if journal.entries.is_empty() {
                journal::delete(&self.data_dir)?;
                self.journal = None;
            } else {
                journal::write(journal, &self.data_dir)?;
            }
        }
        Ok(())
    }

    fn restore_one(&mut self, entry: &JournalEntry) {
        let Some(switch) = self
            .switches
            .iter()
            .find(|s| s.id() == entry.switch)
            .map(|s| s.as_ref())
        else {
            self.session.failed.push(SkippedSwitch {
                id: entry.switch,
                reason: "switch not registered".into(),
            });
            return;
        };
        match switch.restore(entry) {
            Ok(RestoreOutcome::Restored) => self.session.applied.push(entry.switch),
            Ok(RestoreOutcome::UserChanged) => self.session.skipped.push(SkippedSwitch {
                id: entry.switch,
                reason: "user-reverted".into(),
            }),
            Ok(RestoreOutcome::Gone) => self.session.skipped.push(SkippedSwitch {
                id: entry.switch,
                reason: "gone".into(),
            }),
            Err(e) => self.session.failed.push(SkippedSwitch {
                id: entry.switch,
                reason: e.to_string(),
            }),
        }
    }
}

fn availability_reason(availability: Availability) -> String {
    match availability {
        Availability::Available => "available".into(),
        Availability::NeedsWindows11 => "needs-windows-11".into(),
        Availability::ProbeFailed => "probe-failed".into(),
        Availability::NotBuilt => "not-built".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::switches::fake::{FakeHandle, FakeOptions, FakeSwitch, PanicAt};
    use std::fs;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::path::PathBuf;
    use time::macros::datetime;

    const IDS: [SwitchId; 3] = [
        SwitchId::DesktopIcons,
        SwitchId::Taskbar,
        SwitchId::KeepAwake,
    ];

    fn now() -> OffsetDateTime {
        datetime!(2026-10-26 12:00:00 UTC)
    }

    fn later() -> OffsetDateTime {
        datetime!(2026-10-26 12:00:05 UTC)
    }

    fn ctx() -> Ctx {
        Ctx::default()
    }

    // Unique dir under the system temp dir (no tempfile crate, D-063 instruction).
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("button-session-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn opts(panic_at: Option<PanicAt>, availability: Availability) -> FakeOptions {
        FakeOptions {
            availability,
            persistent: false,
            fail_apply: false,
            fail_restore: false,
            panic_at,
        }
    }

    struct Rig {
        engine: SessionEngine,
        handles: Vec<FakeHandle>,
    }

    /// desktop-icons and taskbar persistent, keep-awake not (D-063).
    fn build(dir: &std::path::Path, per_fake: impl Fn(usize) -> FakeOptions) -> Rig {
        let mut switches: Vec<Box<dyn Switch>> = Vec::new();
        let mut handles = Vec::new();
        for (i, id) in IDS.iter().enumerate() {
            let mut options = per_fake(i);
            options.persistent = i < 2;
            let fake = FakeSwitch::new(*id, options);
            handles.push(fake.handle());
            switches.push(Box::new(fake));
        }
        let engine = SessionEngine::new(
            switches,
            OsInfo {
                build: 26200,
                is_windows_11: true,
            },
            dir.to_path_buf(),
            "0.1.0".into(),
        );
        Rig { engine, handles }
    }

    fn journal_len(dir: &std::path::Path) -> usize {
        match journal::read(dir) {
            JournalRead::Ok(j) => j.entries.len(),
            JournalRead::Missing => 0,
            JournalRead::Corrupt => panic!("journal corrupt"),
        }
    }

    /// Switch ids the journal still owns (the world still has them applied).
    fn surviving_entries(dir: &std::path::Path) -> Vec<SwitchId> {
        match journal::read(dir) {
            JournalRead::Ok(j) => j.entries.iter().map(|e| e.switch).collect(),
            _ => Vec::new(),
        }
    }

    fn all_off(rig: &Rig) -> bool {
        rig.handles.iter().all(|h| !h.is_on())
    }

    #[test]
    fn start_applies_in_fixed_order_and_journals() {
        let dir = test_dir("start");
        let mut rig = build(&dir, |_| opts(None, Availability::Available));
        rig.engine.start(now(), &IDS, &ctx()).unwrap();
        assert_eq!(rig.engine.state().status, SessionStatus::Active);
        assert_eq!(
            rig.engine.state().applied,
            vec![
                SwitchId::KeepAwake,
                SwitchId::DesktopIcons,
                SwitchId::Taskbar
            ]
        );
        assert!(rig.handles.iter().all(|h| h.is_on()));
        assert_eq!(journal_len(&dir), 3);
    }

    #[test]
    fn start_twice_errors_already_active() {
        let dir = test_dir("twice");
        let mut rig = build(&dir, |_| opts(None, Availability::Available));
        rig.engine.start(now(), &IDS, &ctx()).unwrap();
        assert!(matches!(
            rig.engine.start(now(), &IDS, &ctx()),
            Err(SessionError::AlreadyActive)
        ));
    }

    #[test]
    fn switch_states_reflect_the_registry_and_journal_timers() {
        let dir = test_dir("switch-states");
        let mut rig = build(&dir, |_| opts(None, Availability::Available));
        let states = rig.engine.switch_states();
        assert!(states.iter().all(|s| !s.on && s.off_at.is_none()));

        // Starting Recording Mode turns every available fake on (FR-4).
        rig.engine.start(now(), &IDS, &ctx()).unwrap();
        let states = rig.engine.switch_states();
        assert!(states.iter().all(|s| s.on));

        // A timer entry in the journal surfaces as off_at for that switch:
        // seed one on disk for a persistent switch and recover. (A timer for
        // a non-persistent switch is dropped at recovery: that switch died
        // with the old process, spec 7.3.)
        let mut journal = journal::Journal::new("0.1.0", now());
        journal.entries.push(JournalEntry {
            seq: journal.next_seq(),
            owner: JournalOwner::Timer,
            switch: SwitchId::Taskbar,
            before: serde_json::json!(false),
            after: serde_json::json!(true),
            persistent: true,
            off_at: Some(later()),
            at: now(),
        });
        journal::write(&journal, &dir).unwrap();
        let mut rig2 = build(&dir, |_| opts(None, Availability::Available));
        rig2.engine.recover(now()).unwrap();
        let states = rig2.engine.switch_states();
        let taskbar = states.iter().find(|s| s.id == SwitchId::Taskbar).unwrap();
        assert_eq!(taskbar.off_at, Some(later()));
        let desktop = states
            .iter()
            .find(|s| s.id == SwitchId::DesktopIcons)
            .unwrap();
        assert_eq!(desktop.off_at, None);
    }

    #[test]
    fn recovery_restores_a_due_timer_and_removes_its_entry() {
        // A deadline that passed while the app was down: the switch must be
        // restored (off_at <= now is due, spec 7.7) and the entry removed,
        // not re-armed.
        let dir = test_dir("due-timer");
        let mut journal = journal::Journal::new("0.1.0", now());
        journal.entries.push(JournalEntry {
            seq: journal.next_seq(),
            owner: JournalOwner::Timer,
            switch: SwitchId::Taskbar,
            before: serde_json::json!(false),
            after: serde_json::json!(true),
            persistent: true,
            off_at: Some(now()),
            at: now(),
        });
        journal::write(&journal, &dir).unwrap();

        let mut rig = build(&dir, |_| opts(None, Availability::Available));
        // The OS still has taskbar changed (the entry we expect restored);
        // registry order is desktop-icons, taskbar, keep-awake.
        rig.handles[1].set(true);
        let report = rig.engine.recover(now()).unwrap();
        assert_eq!(report.restored, vec![SwitchId::Taskbar]);
        assert!(report.timers_to_rearm.is_empty());
        assert!(all_off(&rig));
        assert_eq!(journal_len(&dir), 0);
    }

    #[test]
    fn failed_first_journal_write_rolls_back_and_stays_idle() {
        // The data dir does not exist, so the pre-apply journal write
        // fails: start must apply nothing, leave no journal behind, and
        // return the engine to Idle - never stuck in Applying/Restoring.
        let dir = test_dir("rollback");
        let missing = dir.join("does-not-exist");
        let mut rig = build(&missing, |_| opts(None, Availability::Available));
        assert!(matches!(
            rig.engine.start(now(), &IDS, &ctx()),
            Err(SessionError::Io(_))
        ));
        assert_eq!(rig.engine.state().status, SessionStatus::Idle);
        assert!(rig.handles.iter().all(|h| !h.is_on()));
        assert!(matches!(journal::read(&missing), JournalRead::Missing));
        // The engine stays usable: a start into a working directory works.
        std::fs::create_dir_all(&missing).unwrap();
        rig.engine.start(now(), &IDS, &ctx()).unwrap();
        assert_eq!(rig.engine.state().status, SessionStatus::Active);
    }

    #[test]
    fn stop_restores_and_a_second_stop_is_rejected() {
        let dir = test_dir("stop");
        let mut rig = build(&dir, |_| opts(None, Availability::Available));
        rig.engine.start(now(), &IDS, &ctx()).unwrap();
        rig.engine.stop(later()).unwrap();
        assert_eq!(rig.engine.state().status, SessionStatus::Idle);
        assert!(all_off(&rig));
        assert!(matches!(journal::read(&dir), JournalRead::Missing));
        assert!(matches!(
            rig.engine.stop(later()),
            Err(SessionError::NotActive)
        ));
    }

    #[test]
    fn one_failing_apply_does_not_stop_the_others() {
        let dir = test_dir("fail-apply");
        let mut rig = build(&dir, |i| {
            let mut o = opts(None, Availability::Available);
            o.fail_apply = i == 0;
            o
        });
        rig.engine.start(now(), &IDS, &ctx()).unwrap();
        assert_eq!(rig.engine.state().applied.len(), 2);
        assert_eq!(rig.engine.state().failed.len(), 1);
        assert_eq!(rig.engine.state().failed[0].id, SwitchId::DesktopIcons);
        assert!(!rig.handles[0].is_on());
        assert!(rig.handles[1].is_on() && rig.handles[2].is_on());
        // The failed switch is not in the journal: only 2 entries.
        assert_eq!(journal_len(&dir), 2);
        rig.engine.stop(later()).unwrap();
        assert!(all_off(&rig));
        assert!(matches!(journal::read(&dir), JournalRead::Missing));
    }

    #[test]
    fn unavailable_and_already_on_switches_are_skipped() {
        let dir = test_dir("skip");
        let mut rig = build(&dir, |i| {
            let mut o = opts(None, Availability::Available);
            if i == 1 {
                o.availability = Availability::NeedsWindows11;
            }
            o
        });
        rig.handles[0].set(true); // already on: nothing to undo
        rig.engine.start(now(), &IDS, &ctx()).unwrap();
        assert_eq!(rig.engine.state().applied, vec![SwitchId::KeepAwake]);
        assert_eq!(rig.engine.state().skipped.len(), 1);
        assert_eq!(rig.engine.state().skipped[0].reason, "needs-windows-11");
        // Only the keep-awake entry is journaled; the no-op is not.
        assert_eq!(journal_len(&dir), 1);
        rig.engine.stop(later()).unwrap();
        assert!(matches!(journal::read(&dir), JournalRead::Missing));
    }

    #[test]
    fn user_reverted_values_are_left_alone() {
        let dir = test_dir("user-revert");
        let mut rig = build(&dir, |_| opts(None, Availability::Available));
        rig.engine.start(now(), &IDS, &ctx()).unwrap();
        rig.handles[1].set(false); // the user undid taskbar behind our back
        rig.engine.stop(later()).unwrap();
        assert!(!rig.handles[1].is_on());
        let skipped = &rig.engine.state().skipped;
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].id, SwitchId::Taskbar);
        assert_eq!(skipped[0].reason, "user-reverted");
        assert!(matches!(journal::read(&dir), JournalRead::Missing));
    }

    #[test]
    fn recovery_drops_non_persistent_and_restores_persistent() {
        let dir = test_dir("recovery");
        let mut rig = build(&dir, |_| opts(None, Availability::Available));
        rig.engine.start(now(), &IDS, &ctx()).unwrap();
        drop(rig); // "crash": no restore ran

        let mut rig2 = build(&dir, |_| opts(None, Availability::Available));
        // The real world still has the changes applied; the new fake objects
        // start off, so set them to what the OS would still report.
        rig2.handles[0].set(true);
        rig2.handles[1].set(true);
        let report = rig2.engine.recover(now()).unwrap();
        assert!(!report.corrupted);
        // Keep-awake state died with the process; the persistent ones restored.
        // Restore runs last seq first, so taskbar before desktop-icons.
        assert_eq!(
            report.restored,
            vec![SwitchId::Taskbar, SwitchId::DesktopIcons]
        );
        assert!(all_off(&rig2));
        assert!(matches!(journal::read(&dir), JournalRead::Missing));
    }

    #[test]
    fn recovery_rearms_future_timers_and_restores_the_rest() {
        let dir = test_dir("timers");
        fs::write(
            dir.join("journal.json"),
            r#"{"journalVersion":1,"appVersion":"0.1.0","startedAt":"2026-10-26T11:00:00Z","entries":[
                {"seq":1,"owner":"recording","switch":"desktop-icons","before":false,"after":true,"persistent":true,"offAt":null,"at":"2026-10-26T11:00:00Z"},
                {"seq":2,"owner":"timer","switch":"taskbar","before":false,"after":true,"persistent":true,"offAt":"2027-01-01T08:00:00Z","at":"2026-10-26T11:30:00Z"}
            ]}"#,
        )
        .unwrap();
        let mut rig = build(&dir, |_| opts(None, Availability::Available));
        // The OS still has desktop-icons changed (the entry we expect restored).
        rig.handles[0].set(true);
        let report = rig.engine.recover(now()).unwrap();
        assert_eq!(
            report.restored,
            vec![SwitchId::DesktopIcons],
            "report={report:?}"
        );
        assert_eq!(report.timers_to_rearm.len(), 1);
        assert_eq!(report.timers_to_rearm[0].switch, SwitchId::Taskbar);
        // The timer entry stays in the journal until its deadline fires.
        assert_eq!(journal_len(&dir), 1);
    }

    #[test]
    fn kill_at_every_start_call_still_recovers_cleanly() {
        for after_mutation in [false, true] {
            for fake_idx in 0..3 {
                for n in 1..=4 {
                    let dir = test_dir(&format!("kill-start-{after_mutation}-{fake_idx}-{n}"));
                    let mut rig = build(&dir, |i| {
                        if i == fake_idx {
                            opts(
                                Some(PanicAt {
                                    call: n,
                                    after_mutation,
                                }),
                                Availability::Available,
                            )
                        } else {
                            opts(None, Availability::Available)
                        }
                    });
                    // n beyond the fake's call count means no panic; every
                    // branch must end in the same clean state.
                    let _ = catch_unwind(AssertUnwindSafe(|| {
                        rig.engine.start(now(), &IDS, &ctx()).unwrap();
                    }));
                    drop(rig);

                    let mut rig2 = build(&dir, |_| opts(None, Availability::Available));
                    rig2.engine.recover(now()).unwrap();
                    let states: Vec<bool> = rig2.handles.iter().map(|h| h.is_on()).collect();
                    assert!(
                        all_off(&rig2),
                        "fake_idx={fake_idx} n={n} after={after_mutation} states={states:?}"
                    );
                    assert!(
                        matches!(journal::read(&dir), JournalRead::Missing),
                        "fake_idx={fake_idx} n={n} after={after_mutation}"
                    );
                }
            }
        }
    }

    #[test]
    fn kill_during_restore_still_ends_clean() {
        for after_mutation in [false, true] {
            for fake_idx in 0..3 {
                for n in 1..=4 {
                    let dir = test_dir(&format!("kill-restore-{after_mutation}-{fake_idx}-{n}"));
                    let mut rig = build(&dir, |_| opts(None, Availability::Available));
                    rig.engine.start(now(), &IDS, &ctx()).unwrap();
                    drop(rig);

                    // Recovery itself runs the restore; kill it mid-way.
                    let mut rig2 = build(&dir, |i| {
                        if i == fake_idx {
                            opts(
                                Some(PanicAt {
                                    call: n,
                                    after_mutation,
                                }),
                                Availability::Available,
                            )
                        } else {
                            opts(None, Availability::Available)
                        }
                    });
                    // The OS still has everything the journal owns applied.
                    for (i, id) in IDS.iter().enumerate() {
                        if surviving_entries(&dir).contains(id) {
                            rig2.handles[i].set(true);
                        }
                    }
                    let _ = catch_unwind(AssertUnwindSafe(|| {
                        rig2.engine.recover(now()).unwrap();
                    }));
                    drop(rig2);

                    let mut rig3 = build(&dir, |_| opts(None, Availability::Available));
                    // Only the entries the killed restore left behind are
                    // still applied in the world.
                    for (i, id) in IDS.iter().enumerate() {
                        if surviving_entries(&dir).contains(id) {
                            rig3.handles[i].set(true);
                        }
                    }
                    rig3.engine.recover(later()).unwrap();
                    let states: Vec<bool> = rig3.handles.iter().map(|h| h.is_on()).collect();
                    assert!(
                        all_off(&rig3),
                        "fake_idx={fake_idx} n={n} after={after_mutation} states={states:?}"
                    );
                    assert!(
                        matches!(journal::read(&dir), JournalRead::Missing),
                        "fake_idx={fake_idx} n={n} after={after_mutation}"
                    );
                }
            }
        }
    }
}
