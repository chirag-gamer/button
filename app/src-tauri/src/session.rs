//! Session engine: apply / restore / crash recovery (spec 7.1 to 7.3), the
//! startup probes (spec 7.6, FR-35), quick-switch turns (FR-27) and
//! off-timers (FR-28, spec 7.7). No Tauri types in here: it takes the switch
//! list, OS info and the data dir, which is what makes it unit-testable
//! with fakes (D-033).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

use crate::journal::{self, Journal, JournalEntry, JournalOwner, JournalRead};
use crate::os::OsInfo;
use crate::switches::{
    Availability, Ctx, RECORDING_ORDER, RestoreOutcome, Switch, SwitchError, SwitchId,
};
use crate::{SessionState, SessionStatus, SkippedSwitch, SwitchState};

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("a session is already active")]
    AlreadyActive,
    #[error("no session is active")]
    NotActive,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A journal write failure surfaced through a SwitchError path (the engine's
/// SwitchError has no Io variant).
fn journal_write_err(e: std::io::Error) -> SwitchError {
    SwitchError::Os {
        api: "journal::write",
        code: e.raw_os_error().unwrap_or(-1),
    }
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

/// The `offAfter` argument of `set_switch` (spec 6.1, D-051): a preset
/// string, a minute count, or an absolute RFC 3339 time.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum OffAfter {
    Preset(String),
    Minutes {
        minutes: u32,
    },
    At {
        #[serde(with = "time::serde::rfc3339")]
        at: OffsetDateTime,
    },
}

/// Parses `offAfter` into the absolute deadline (task 4.2). "tomorrow"
/// means the next 08:00 local: chosen before 08:00 it is today's 08:00,
/// at or after 08:00 it is the next day's (D-079). The local offset is
/// recomputed at the deadline once, so a DST boundary between now and
/// then still lands on 08:00 local.
pub fn parse_off_after(spec: &OffAfter, now: OffsetDateTime) -> Result<OffsetDateTime, String> {
    match spec {
        OffAfter::Preset(p) => match p.as_str() {
            "15m" => Ok(now + time::Duration::minutes(15)),
            "1h" => Ok(now + time::Duration::hours(1)),
            "4h" => Ok(now + time::Duration::hours(4)),
            "tomorrow" => tomorrow_0800(now),
            other => Err(format!("unknown offAfter preset '{other}'")),
        },
        OffAfter::Minutes { minutes } => {
            if (1..=1440).contains(minutes) {
                Ok(now + time::Duration::minutes(*minutes as i64))
            } else {
                Err(format!("minutes must be 1..=1440, got {minutes}"))
            }
        }
        OffAfter::At { at } if *at > now => Ok(*at),
        OffAfter::At { .. } => Err("offAfter.at must be in the future".into()),
    }
}

fn tomorrow_0800(now: OffsetDateTime) -> Result<OffsetDateTime, String> {
    let offset = UtcOffset::local_offset_at(now)
        .map_err(|e| format!("could not determine the local time zone: {e}"))?;
    let local = now.to_offset(offset);
    // D-079: before 08:00 the next 08:00 is later today, else tomorrow's.
    let date = next_0800_date(local.date(), local.time())?;
    let eight = time::Time::from_hms(8, 0, 0).expect("08:00 is a valid time");
    let naive = PrimitiveDateTime::new(date, eight).assume_offset(offset);
    // One DST recompute: 08:00 wall clock could sit in a different offset.
    let at_offset = UtcOffset::local_offset_at(naive).unwrap_or(offset);
    Ok(PrimitiveDateTime::new(date, eight).assume_offset(at_offset))
}

/// The calendar date of the next 08:00 local (D-079): today while the local
/// time is before 08:00, else the next day.
fn next_0800_date(date: time::Date, local_time: time::Time) -> Result<time::Date, String> {
    let eight = time::Time::from_hms(8, 0, 0).expect("08:00 is a valid time");
    if local_time < eight {
        Ok(date)
    } else {
        date.next_day()
            .ok_or_else(|| "no tomorrow at the end of the calendar".to_string())
    }
}

pub struct SessionEngine {
    switches: Vec<Box<dyn Switch>>,
    os: OsInfo,
    data_dir: PathBuf,
    app_version: String,
    session: SessionState,
    journal: Option<Journal>,
    /// Probe failures by switch id (FR-35): the switch is unavailable with
    /// this reason until the app restarts.
    probe_failures: HashMap<SwitchId, String>,
    /// Manual on-turns without a timer, in memory only (D-030): (before,
    /// after) so a later off, or a timer added while already on, knows what
    /// to go back to.
    manual: HashMap<SwitchId, (serde_json::Value, serde_json::Value)>,
    /// Row cache (task 4.1): refreshed at the few points where switch state
    /// changes — never by the 1 s ticker.
    rows: Vec<SwitchState>,
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
            probe_failures: HashMap::new(),
            manual: HashMap::new(),
            rows: Vec::new(),
        }
    }

    pub fn state(&self) -> &SessionState {
        &self.session
    }

    fn switch(&self, id: SwitchId) -> Option<&dyn Switch> {
        self.switches
            .iter()
            .find(|s| s.id() == id)
            .map(|s| s.as_ref())
    }

    /// Availability including the probe outcome (FR-35): a failed probe
    /// disables the switch for Recording Mode and the dropdown alike.
    fn effective_availability(&self, id: SwitchId, os_availability: Availability) -> Availability {
        if self.probe_failures.contains_key(&id) {
            Availability::ProbeFailed
        } else {
            os_availability
        }
    }

    /// The user-facing reason an unavailable switch carries (row detail and
    /// skip reason): the probe message, else the availability name.
    fn unavailability_reason(&self, id: SwitchId, availability: Availability) -> String {
        match self.probe_failures.get(&id) {
            Some(probe_reason) => probe_reason.clone(),
            None => availability_reason(availability),
        }
    }

    /// Availability of one registered switch, if present.
    pub fn switch_availability(&self, id: SwitchId) -> Option<Availability> {
        self.switch(id)
            .map(|s| self.effective_availability(id, s.availability(&self.os)))
    }

    /// Runs the read-only startup probes of every experimental switch, once
    /// (spec 7.6). Failures disable the switch with the reason.
    pub fn run_probes(&mut self) {
        let begun = std::time::Instant::now();
        let failures: Vec<(SwitchId, String)> = self
            .switches
            .iter()
            .filter(|s| s.experimental())
            .filter_map(|s| s.probe().err().map(|reason| (s.id(), reason)))
            .collect();
        let probed = self.switches.iter().filter(|s| s.experimental()).count();
        for (id, reason) in &failures {
            log::warn!("probe failed for {id}: {reason}");
        }
        log::info!(
            "probes: {} experimental, {} failed in {} ms",
            probed,
            failures.len(),
            begun.elapsed().as_millis()
        );
        self.probe_failures = failures.into_iter().collect();
        self.refresh_rows();
    }

    /// Reads every switch once and rebuilds the row cache (task 4.1).
    /// Called after every state-changing point — set_switch, start, stop,
    /// a fired timer, the dropdown opening, the delayed Bluetooth refresh —
    /// and never by the ticker.
    pub fn refresh_rows(&mut self) {
        let mut rows = Vec::with_capacity(self.switches.len());
        for s in &self.switches {
            let id = s.id();
            let availability = self.effective_availability(id, s.availability(&self.os));
            let on = s.read().map(|v| s.is_on(&v)).unwrap_or(false);
            let off_at = self
                .journal
                .as_ref()
                .and_then(|j| {
                    j.entries
                        .iter()
                        .rev()
                        .find(|e| e.owner == JournalOwner::Timer && e.switch == id)
                })
                .and_then(|e| e.off_at);
            rows.push(SwitchState {
                id,
                on,
                availability,
                experimental: s.experimental(),
                off_at,
                detail: match availability {
                    Availability::ProbeFailed => self.probe_failures.get(&id).cloned(),
                    Availability::NeedsWindows11 => Some("Needs Windows 11".into()),
                    _ => None,
                },
            });
        }
        self.rows = rows;
    }

    /// Per-switch rows for the UI (spec 5.4) — the cache built by
    /// `refresh_rows`, so the ticker's once-a-second emit does no reads.
    pub fn switch_states(&self) -> Vec<SwitchState> {
        self.rows.clone()
    }

    /// The cached on flag of one switch (automation's `switch toggle`).
    pub fn is_switch_on(&self, id: SwitchId) -> bool {
        self.rows.iter().find(|r| r.id == id).is_some_and(|r| r.on)
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
        let begun = std::time::Instant::now();
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
            let availability = if self.probe_failures.contains_key(&id) {
                Availability::ProbeFailed
            } else {
                switch.availability(&self.os)
            };
            if availability != Availability::Available {
                // Inline field access: the journal borrow is live for the
                // rest of the loop, so no &self methods up here.
                let reason = match self.probe_failures.get(&id) {
                    Some(probe_reason) => probe_reason.clone(),
                    None => availability_reason(availability),
                };
                self.session.skipped.push(SkippedSwitch { id, reason });
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
            let after = switch.recording_target(&before, ctx);
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
        self.refresh_rows();
        // G-1 evidence (prd.md §6): apply time from Start to recording-ready,
        // excluding the countdown, which runs before this in the app layer.
        log::info!(
            "start: {} applied, {} skipped, {} failed in {} ms",
            self.session.applied.len(),
            self.session.skipped.len(),
            self.session.failed.len(),
            begun.elapsed().as_millis()
        );
        Ok(&self.session)
    }

    /// Turn a single switch on or off, optionally with an off-timer
    /// (FR-27, FR-28, task 4.2). Recording Mode ownership of the active
    /// session wins; manual on-turns without a timer are not journaled
    /// (D-030), timers are.
    pub fn set_switch(
        &mut self,
        id: SwitchId,
        on: bool,
        off_after: Option<OffAfter>,
        now: OffsetDateTime,
        ctx: &Ctx,
    ) -> Result<(), SwitchError> {
        let deadline = match &off_after {
            Some(spec) => Some(parse_off_after(spec, now).map_err(SwitchError::Invalid)?),
            None => None,
        };

        let Some(switch) = self.switch(id) else {
            return Err(SwitchError::Invalid(format!("{id} is not registered")));
        };
        let availability = self.effective_availability(id, switch.availability(&self.os));
        if availability != Availability::Available {
            return Err(SwitchError::NotAvailable(
                self.unavailability_reason(id, availability),
            ));
        }

        // Actions (screen-saver): fire on "on", never journaled, never timed.
        if switch.is_action() {
            if deadline.is_some() {
                return Err(SwitchError::Invalid(format!(
                    "{id} starts instantly and cannot have a timer"
                )));
            }
            if on {
                let target = switch.target_on(&switch.read()?, ctx);
                switch.apply(&target)?;
            }
            self.refresh_rows();
            return Ok(());
        }

        // A Recording entry of the active session owns this switch.
        if self.session.status == SessionStatus::Active
            && self.journal.as_ref().is_some_and(|j| {
                j.entries
                    .iter()
                    .any(|e| e.owner == JournalOwner::Recording && e.switch == id)
            })
        {
            return match (on, deadline) {
                (false, _) => {
                    self.restore_recording_entry(id);
                    self.refresh_rows();
                    Ok(())
                }
                (true, None) => Ok(()), // already on and owned by the session
                (true, Some(_)) => Err(SwitchError::Invalid(
                    "Recording Mode controls this switch while the session is active".into(),
                )),
            };
        }

        if on {
            match deadline {
                Some(deadline) => self.set_timer(id, deadline, now, ctx)?,
                None => self.turn_on(id, ctx)?,
            }
        } else {
            self.turn_off(id, now, ctx)?;
        }
        self.refresh_rows();
        Ok(())
    }

    /// set_switch step 4: on with an off-timer (FR-28).
    fn set_timer(
        &mut self,
        id: SwitchId,
        deadline: OffsetDateTime,
        now: OffsetDateTime,
        ctx: &Ctx,
    ) -> Result<(), SwitchError> {
        {
            let journal = self
                .journal
                .get_or_insert_with(|| Journal::new(&self.app_version, now));
            // An existing Timer entry just moves its deadline. Roll the
            // in-memory entry back when the rewrite fails, so memory and
            // disk cannot disagree about when the switch turns off. (Index
            // access, not an entry borrow: the entry is written back to
            // after the file write.)
            if let Some(idx) = journal
                .entries
                .iter()
                .rposition(|e| e.owner == JournalOwner::Timer && e.switch == id)
            {
                let previous = journal.entries[idx].off_at;
                journal.entries[idx].off_at = Some(deadline);
                if let Err(e) = journal::write(journal, &self.data_dir) {
                    journal.entries[idx].off_at = previous;
                    return Err(journal_write_err(e));
                }
                return Ok(());
            }
        }

        let switch = self
            .switches
            .iter()
            .find(|s| s.id() == id)
            .map(|s| s.as_ref())
            .expect("availability checked in set_switch");
        let current = switch.read()?;
        let (before, after, apply_value) = if switch.is_on(&current) {
            // Already on: the timer turns it off later. What it goes back
            // to is the recorded manual `before` when we have one, else the
            // computed target_off.
            let before = match self.manual.get(&id) {
                Some((manual_before, _)) => Some(manual_before.clone()),
                None => switch.target_off(&current, ctx),
            };
            let Some(before) = before else {
                return Err(SwitchError::Invalid(format!(
                    "{id} is already on and has no recorded off state to return to"
                )));
            };
            (before, current.clone(), None)
        } else {
            let after = switch.target_on(&current, ctx);
            (current, after.clone(), Some(after))
        };

        let journal = self
            .journal
            .get_or_insert_with(|| Journal::new(&self.app_version, now));
        journal.entries.push(JournalEntry {
            seq: journal.next_seq(),
            owner: JournalOwner::Timer,
            switch: id,
            before,
            after,
            persistent: switch.persistent(),
            off_at: Some(deadline),
            at: now,
        });
        if let Err(e) = journal::write(journal, &self.data_dir) {
            let journal = self.journal.as_mut().expect("inserted above");
            journal.entries.pop();
            let _ = journal::write(journal, &self.data_dir);
            return Err(journal_write_err(e));
        }
        // The Timer entry now owns restoring; the manual record is superseded.
        self.manual.remove(&id);
        if let Some(value) = apply_value
            && let Err(e) = switch.apply(&value)
        {
            let journal = self.journal.as_mut().expect("inserted above");
            journal.entries.pop();
            let _ = journal::write(journal, &self.data_dir);
            return Err(e);
        }
        Ok(())
    }

    /// set_switch step 5: on with no timer. A running timer is dropped (no
    /// end); an on-switch is a no-op; else apply target_on and remember the
    /// manual record (D-030: not journaled). When the user (or Windows)
    /// turned the switch off under the timer, the plain on still applies —
    /// dropping the entry must not swallow the turn-on.
    fn turn_on(&mut self, id: SwitchId, ctx: &Ctx) -> Result<(), SwitchError> {
        self.drop_timer_entry(id)?;
        let switch = self
            .switches
            .iter()
            .find(|s| s.id() == id)
            .map(|s| s.as_ref())
            .expect("availability checked in set_switch");
        let before = switch.read()?;
        if switch.is_on(&before) {
            return Ok(()); // already on
        }
        let after = switch.target_on(&before, ctx);
        switch.apply(&after)?;
        self.manual.insert(id, (before, after));
        Ok(())
    }

    /// set_switch step 6: off.
    fn turn_off(
        &mut self,
        id: SwitchId,
        now: OffsetDateTime,
        ctx: &Ctx,
    ) -> Result<(), SwitchError> {
        // A timer means button owns the current on state: restore it now.
        if self.remove_timer_entry(id)? {
            return Ok(());
        }
        // A manual record knows what the switch came from. It stays in the
        // map when the restore errors, so a retry walks the same path.
        let manual = self.manual.remove(&id);
        let switch = self
            .switches
            .iter()
            .find(|s| s.id() == id)
            .map(|s| s.as_ref())
            .expect("availability checked in set_switch");

        if let Some((before, after)) = manual {
            let entry = JournalEntry {
                seq: 0,
                owner: JournalOwner::Timer,
                switch: id,
                before: before.clone(),
                after: after.clone(),
                persistent: switch.persistent(),
                off_at: None,
                at: now,
            };
            match switch.restore(&entry) {
                Ok(RestoreOutcome::Restored) => return Ok(()),
                Ok(RestoreOutcome::UserChanged | RestoreOutcome::Gone) => {
                    let still_on = match switch.read() {
                        Ok(current) => switch.is_on(&current),
                        Err(e) => {
                            // The record survives a failed read too, so a
                            // retry walks the same path.
                            self.manual.insert(id, (before, after));
                            return Err(e);
                        }
                    };
                    if !still_on {
                        return Ok(()); // user turned it off already
                    }
                    // Still on but not in our recorded state: fall through
                    // to target_off below.
                }
                Err(e) => {
                    self.manual.insert(id, (before, after));
                    return Err(e);
                }
            }
        }

        let current = switch.read()?;
        if !switch.is_on(&current) {
            return Ok(()); // already off
        }
        match switch.target_off(&current, ctx) {
            Some(off) => switch.apply(&off),
            None => Err(SwitchError::Invalid(format!(
                "{id} is on, but button has nothing recorded to undo"
            ))),
        }
    }

    /// Removes the switch's Timer entry, restoring it first. The entry
    /// survives a failed restore (the recovery record must outlive a
    /// transient OS error). Returns whether an entry existed.
    fn remove_timer_entry(&mut self, id: SwitchId) -> Result<bool, SwitchError> {
        let Some(idx) = self.journal.as_ref().and_then(|j| {
            j.entries
                .iter()
                .rposition(|e| e.owner == JournalOwner::Timer && e.switch == id)
        }) else {
            return Ok(false);
        };
        let entry = self.journal.as_ref().unwrap().entries[idx].clone();
        self.restore_entry_logged(&entry)?;
        self.remove_entry_and_write(idx)?;
        Ok(true)
    }

    /// Removes the switch's Timer entry without restoring (the switch stays
    /// on, no end). Returns whether an entry existed.
    fn drop_timer_entry(&mut self, id: SwitchId) -> Result<bool, SwitchError> {
        let Some(idx) = self.journal.as_ref().and_then(|j| {
            j.entries
                .iter()
                .rposition(|e| e.owner == JournalOwner::Timer && e.switch == id)
        }) else {
            return Ok(false);
        };
        self.remove_entry_and_write(idx)?;
        Ok(true)
    }

    /// set_switch step 3-off: restore one Recording entry of the active
    /// session now and remove it (the session no longer owns the switch).
    /// A failed restore keeps the entry — the session still owns it.
    fn restore_recording_entry(&mut self, id: SwitchId) {
        let Some(idx) = self.journal.as_ref().and_then(|j| {
            j.entries
                .iter()
                .rposition(|e| e.owner == JournalOwner::Recording && e.switch == id)
        }) else {
            return;
        };
        let entry = self.journal.as_ref().unwrap().entries[idx].clone();
        if let Err(e) = self.restore_entry_logged(&entry) {
            log::error!("set_switch off: {id} stays session-owned after a failed restore: {e}");
            return;
        }
        if let Err(e) = self.remove_entry_and_write(idx) {
            log::error!("set_switch off: journal rewrite failed: {e}");
        }
    }

    /// Restores one entry and logs the outcome, without touching the
    /// session summary lists (used by the quick-switch paths). The error is
    /// returned so callers can keep the journal entry for a retry.
    fn restore_entry_logged(&self, entry: &JournalEntry) -> Result<(), SwitchError> {
        match self.switch(entry.switch) {
            Some(switch) => {
                let outcome = switch.restore(entry).inspect_err(|e| {
                    log::error!("restore {}: {e}", entry.switch);
                })?;
                log::info!("restore {}: {outcome:?}", entry.switch);
                Ok(())
            }
            None => {
                // Ownership of a switch this build no longer registers:
                // nothing to restore, the entry is stale.
                log::error!("restore {}: switch not registered", entry.switch);
                Ok(())
            }
        }
    }

    /// Removes journal entry `idx` and rewrites (or deletes) the file;
    /// drops the in-memory journal once it is empty.
    fn remove_entry_and_write(&mut self, idx: usize) -> Result<(), SwitchError> {
        let journal = self.journal.as_mut().expect("caller checked");
        journal.entries.remove(idx);
        if journal.entries.is_empty() {
            journal::delete(&self.data_dir).map_err(journal_write_err)?;
            self.journal = None;
        } else {
            journal::write(journal, &self.data_dir).map_err(journal_write_err)?;
        }
        Ok(())
    }

    /// Whether any Timer entry is pending (the ticker's work condition).
    pub fn has_timers(&self) -> bool {
        self.journal
            .as_ref()
            .is_some_and(|j| j.entries.iter().any(|e| e.owner == JournalOwner::Timer))
    }

    /// Fires every due Timer entry (spec 7.7). Wall-clock deadlines, so a
    /// deadline missed during sleep fires on the first tick after resume
    /// (FR-28). Never touches the session status or the applied/skipped/
    /// failed summary (that is restore_all's job). A restore that fails
    /// keeps its entry — the next tick retries, and the OTHER due timers
    /// still fire (no head-of-line blocking). Returns whether anything
    /// fired.
    pub fn fire_due_timers(&mut self, now: OffsetDateTime) -> bool {
        // Snapshot the due entries' seqs (newest first, restore order); a
        // failed one is skipped for this pass instead of stopping the loop.
        let due: Vec<u64> = self
            .journal
            .as_ref()
            .map(|j| {
                j.entries
                    .iter()
                    .filter(|e| {
                        e.owner == JournalOwner::Timer && e.off_at.is_some_and(|t| t <= now)
                    })
                    .map(|e| e.seq)
                    .rev()
                    .collect()
            })
            .unwrap_or_default();
        let mut fired = false;
        for seq in due {
            let Some(idx) = self
                .journal
                .as_ref()
                .and_then(|j| j.entries.iter().position(|e| e.seq == seq))
            else {
                continue; // removed by an earlier iteration of this pass
            };
            let entry = self.journal.as_ref().unwrap().entries[idx].clone();
            match self.restore_entry_logged(&entry) {
                Ok(()) => {
                    if let Err(e) = self.remove_entry_and_write(idx) {
                        log::error!("timer {}: journal write failed: {e}", entry.switch);
                    }
                    fired = true;
                }
                Err(e) => {
                    log::error!(
                        "timer {}: restore failed, retrying next tick: {e}",
                        entry.switch
                    );
                    // the entry stays; keep draining the other due timers
                }
            }
        }
        if fired {
            self.refresh_rows();
        }
        fired
    }

    /// CLI `restore` (spec 6.3): stop the active session if any, then
    /// restore EVERY remaining entry — including Timer entries that are not
    /// due yet. Never runs launch recovery while the app runs: that would
    /// drop non-persistent entries like keep-awake (spec 7.3 vs 7.2).
    /// Returns how many restores failed; their entries stay in the journal
    /// for the next recovery.
    pub fn restore_all_now(&mut self, now: OffsetDateTime) -> Result<usize, SessionError> {
        let mut failed = 0usize;
        if self.session.status == SessionStatus::Active {
            self.restore_all(now)?;
        }
        let remaining: Vec<JournalEntry> = self
            .journal
            .as_ref()
            .map(|j| j.entries.clone())
            .unwrap_or_default();
        let mut kept: Vec<JournalEntry> = Vec::new();
        for entry in remaining.iter().rev() {
            match self.restore_entry_logged(entry) {
                Ok(()) => {}
                Err(_) => {
                    failed += 1;
                    kept.push(entry.clone());
                }
            }
        }
        kept.reverse(); // back to journal order
        if kept.is_empty() {
            journal::delete(&self.data_dir)?;
            self.journal = None;
        } else {
            let journal = self
                .journal
                .get_or_insert_with(|| Journal::new(&self.app_version, now));
            journal.entries = kept;
            journal::write(journal, &self.data_dir)?;
        }
        self.refresh_rows();
        Ok(failed)
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
            JournalRead::Io(e) => {
                // Locked/unreadable, not corrupt (spec 7.3): skip recovery,
                // leave the file for the next launch — the app layer's
                // existing catch logs it and keeps booting.
                return Err(SessionError::Io(e));
            }
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
            self.refresh_rows();
            return Ok(());
        }
        self.session.status = SessionStatus::Restoring;
        let result = self.restore_entries(now);
        self.session.status = SessionStatus::Idle;
        self.refresh_rows();
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
        let Some(switch) = self.switch(entry.switch) else {
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
            panic_at,
            ..manual_opts()
        }
    }

    /// Quick-switch-capable options: target_off works like the real
    /// desktop-icons/taskbar switches.
    fn manual_opts() -> FakeOptions {
        FakeOptions {
            has_target_off: true,
            ..FakeOptions::default()
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
            JournalRead::Io(_) => panic!("journal unreadable"),
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

    /// The rig's index of a switch id.
    fn idx(id: SwitchId) -> usize {
        IDS.iter().position(|s| *s == id).unwrap()
    }

    /// The Timer entry of one switch in the on-disk journal, if any.
    fn timer_entry(
        dir: &std::path::Path,
        id: SwitchId,
    ) -> Option<(serde_json::Value, serde_json::Value, Option<OffsetDateTime>)> {
        match journal::read(dir) {
            JournalRead::Ok(j) => j
                .entries
                .iter()
                .find(|e| e.owner == JournalOwner::Timer && e.switch == id)
                .map(|e| (e.before.clone(), e.after.clone(), e.off_at)),
            _ => None,
        }
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

        // Starting Recording Mode turns every available fake on (FR-4) and
        // the row cache follows.
        rig.engine.start(now(), &IDS, &ctx()).unwrap();
        let states = rig.engine.switch_states();
        assert_eq!(states.len(), 3);
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

    // ---- Quick switches (task 4.2) ----

    #[test]
    fn manual_on_and_off_without_a_timer_is_not_journaled() {
        let dir = test_dir("manual-on-off");
        let mut rig = build(&dir, |_| manual_opts());
        rig.engine
            .set_switch(SwitchId::DesktopIcons, true, None, now(), &ctx())
            .unwrap();
        assert!(rig.handles[idx(SwitchId::DesktopIcons)].is_on());
        assert_eq!(journal_len(&dir), 0, "D-030: no journal for manual on");
        assert!(rig.engine.is_switch_on(SwitchId::DesktopIcons));

        rig.engine
            .set_switch(SwitchId::DesktopIcons, false, None, later(), &ctx())
            .unwrap();
        assert!(!rig.handles[idx(SwitchId::DesktopIcons)].is_on());
        assert_eq!(journal_len(&dir), 0);
    }

    #[test]
    fn off_of_an_unrecorded_on_state_uses_target_off() {
        let dir = test_dir("target-off");
        let mut rig = build(&dir, |_| manual_opts());
        rig.handles[idx(SwitchId::Taskbar)].set(true); // on, but not by button
        rig.engine
            .set_switch(SwitchId::Taskbar, false, None, now(), &ctx())
            .unwrap();
        assert!(!rig.handles[idx(SwitchId::Taskbar)].is_on());
    }

    #[test]
    fn off_without_a_record_or_target_off_is_invalid() {
        let dir = test_dir("nothing-to-undo");
        let mut rig = build(&dir, |i| {
            let mut o = manual_opts();
            if i == 0 {
                o.has_target_off = false; // like other-apps
            }
            o
        });
        rig.handles[idx(SwitchId::DesktopIcons)].set(true);
        let err = rig
            .engine
            .set_switch(SwitchId::DesktopIcons, false, None, now(), &ctx())
            .unwrap_err();
        assert!(matches!(err, SwitchError::Invalid(_)));
        assert!(rig.handles[idx(SwitchId::DesktopIcons)].is_on());
    }

    #[test]
    fn timer_is_journaled_before_apply_and_fires() {
        let dir = test_dir("timer");
        let mut rig = build(&dir, |_| manual_opts());
        let deadline = datetime!(2026-10-26 12:15:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: deadline }),
                now(),
                &ctx(),
            )
            .unwrap();
        assert!(rig.handles[idx(SwitchId::Taskbar)].is_on());
        let (before, after, off_at) = timer_entry(&dir, SwitchId::Taskbar).unwrap();
        assert_eq!(before, serde_json::json!(false));
        assert_eq!(after, serde_json::json!(true));
        assert_eq!(off_at, Some(deadline));
        assert_eq!(
            rig.engine.switch_states()[idx(SwitchId::Taskbar)].off_at,
            Some(deadline)
        );

        // Not due yet: nothing fires and no reads are done by the check.
        let reads_before = rig.handles[idx(SwitchId::Taskbar)].read_count();
        assert!(!rig.engine.fire_due_timers(now()));
        assert!(rig.handles[idx(SwitchId::Taskbar)].is_on());
        assert_eq!(
            rig.handles[idx(SwitchId::Taskbar)].read_count(),
            reads_before,
            "a non-firing check does no reads"
        );

        // Due: the entry restores and the journal empties. The session
        // stays Idle throughout (spec 7.7: timers never touch it).
        assert!(rig.engine.fire_due_timers(deadline));
        assert!(!rig.handles[idx(SwitchId::Taskbar)].is_on());
        assert_eq!(journal_len(&dir), 0);
        assert_eq!(rig.engine.state().status, SessionStatus::Idle);
        assert!(rig.engine.state().applied.is_empty());
        assert!(!rig.engine.has_timers());
    }

    #[test]
    fn timer_entry_updates_its_deadline_when_set_again() {
        let dir = test_dir("timer-update");
        let mut rig = build(&dir, |_| manual_opts());
        let first = datetime!(2026-10-26 13:00:00 UTC);
        let second = datetime!(2026-10-26 14:00:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: first }),
                now(),
                &ctx(),
            )
            .unwrap();
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: second }),
                now(),
                &ctx(),
            )
            .unwrap();
        assert_eq!(journal_len(&dir), 1, "the same timer moved, not doubled");
        assert_eq!(
            timer_entry(&dir, SwitchId::Taskbar).unwrap().2,
            Some(second)
        );
    }

    #[test]
    fn already_on_switch_gets_a_timer_with_target_off_as_before() {
        // Turned on outside button (a user change), then a timer is added:
        // before must come from target_off and no apply happens.
        let dir = test_dir("timer-on-on");
        let mut rig = build(&dir, |_| manual_opts());
        rig.handles[idx(SwitchId::Taskbar)].set(true);
        let deadline = datetime!(2026-10-26 12:30:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: deadline }),
                now(),
                &ctx(),
            )
            .unwrap();
        let (before, after, _) = timer_entry(&dir, SwitchId::Taskbar).unwrap();
        assert_eq!(
            before,
            serde_json::json!(false),
            "target_off becomes before"
        );
        assert_eq!(after, serde_json::json!(true), "after is the current state");
        assert!(rig.engine.fire_due_timers(deadline));
        assert!(!rig.handles[idx(SwitchId::Taskbar)].is_on());
    }

    #[test]
    fn manual_record_feeds_the_timer_before_when_one_is_added() {
        let dir = test_dir("timer-manual-record");
        let mut rig = build(&dir, |_| manual_opts());
        rig.engine
            .set_switch(SwitchId::Taskbar, true, None, now(), &ctx())
            .unwrap(); // manual record: (false, true)
        rig.handles[idx(SwitchId::Taskbar)].set(false); // user turned it off
        rig.handles[idx(SwitchId::Taskbar)].set(true); // and on again by hand
        let deadline = datetime!(2026-10-26 12:45:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: deadline }),
                now(),
                &ctx(),
            )
            .unwrap();
        let (before, _, _) = timer_entry(&dir, SwitchId::Taskbar).unwrap();
        assert_eq!(before, serde_json::json!(false), "manual.before wins");
    }

    #[test]
    fn turning_on_without_a_timer_drops_an_existing_timer() {
        let dir = test_dir("timer-drop");
        let mut rig = build(&dir, |_| manual_opts());
        let deadline = datetime!(2026-10-26 13:30:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: deadline }),
                now(),
                &ctx(),
            )
            .unwrap();
        rig.engine
            .set_switch(SwitchId::Taskbar, true, None, now(), &ctx())
            .unwrap();
        assert!(rig.handles[idx(SwitchId::Taskbar)].is_on(), "stays on");
        assert_eq!(journal_len(&dir), 0, "the timer entry is gone");
        assert!(!rig.engine.has_timers());
    }

    #[test]
    fn turning_off_a_timed_switch_restores_it_immediately() {
        let dir = test_dir("timer-off-now");
        let mut rig = build(&dir, |_| manual_opts());
        let deadline = datetime!(2026-10-26 15:00:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: deadline }),
                now(),
                &ctx(),
            )
            .unwrap();
        rig.engine
            .set_switch(SwitchId::Taskbar, false, None, now(), &ctx())
            .unwrap();
        assert!(!rig.handles[idx(SwitchId::Taskbar)].is_on());
        assert_eq!(journal_len(&dir), 0);
    }

    #[test]
    fn recording_owned_switches_follow_the_session_rules() {
        let dir = test_dir("recording-owned");
        let mut rig = build(&dir, |_| manual_opts());
        rig.engine.start(now(), &IDS, &ctx()).unwrap();

        // on -> no-op (already on, owned by the session)
        rig.engine
            .set_switch(SwitchId::Taskbar, true, None, now(), &ctx())
            .unwrap();
        assert!(rig.handles[idx(SwitchId::Taskbar)].is_on());

        // on + timer -> rejected
        let err = rig
            .engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::Preset("15m".into())),
                now(),
                &ctx(),
            )
            .unwrap_err();
        assert!(matches!(err, SwitchError::Invalid(_)));

        // off -> restores just that entry now
        rig.engine
            .set_switch(SwitchId::Taskbar, false, None, now(), &ctx())
            .unwrap();
        assert!(!rig.handles[idx(SwitchId::Taskbar)].is_on());
        assert_eq!(rig.engine.state().status, SessionStatus::Active);
        assert_eq!(journal_len(&dir), 2, "the other session entries remain");
        rig.engine.stop(later()).unwrap();
        assert!(all_off(&rig));
    }

    #[test]
    fn actions_never_journal_and_reject_timers() {
        let dir = test_dir("action");
        let mut rig = build(&dir, |i| {
            let mut o = manual_opts();
            if i == 0 {
                o.is_action = true; // like screen-saver
            }
            o
        });
        rig.engine
            .set_switch(SwitchId::DesktopIcons, true, None, now(), &ctx())
            .unwrap();
        assert_eq!(journal_len(&dir), 0, "actions are never journaled");
        // off is a no-op
        rig.engine
            .set_switch(SwitchId::DesktopIcons, false, None, now(), &ctx())
            .unwrap();
        // a timer is rejected
        let err = rig
            .engine
            .set_switch(
                SwitchId::DesktopIcons,
                true,
                Some(OffAfter::Preset("1h".into())),
                now(),
                &ctx(),
            )
            .unwrap_err();
        assert!(matches!(err, SwitchError::Invalid(_)));
    }

    #[test]
    fn failed_probe_disables_the_switch_for_everything() {
        let dir = test_dir("probe-failed");
        let mut rig = build(&dir, |i| {
            let mut o = manual_opts();
            if i == 1 {
                o.fail_probe = true;
                o.experimental = true; // probes run for experimental switches
            }
            o
        });
        rig.engine.run_probes();
        // Row detail carries the reason (FR-35).
        let states = rig.engine.switch_states();
        let taskbar = states.iter().find(|s| s.id == SwitchId::Taskbar).unwrap();
        assert_eq!(taskbar.availability, Availability::ProbeFailed);
        assert_eq!(taskbar.detail.as_deref(), Some("fake probe failure"));

        // set_switch refuses it.
        let err = rig
            .engine
            .set_switch(SwitchId::Taskbar, true, None, now(), &ctx())
            .unwrap_err();
        assert!(matches!(err, SwitchError::NotAvailable(_)));

        // Recording Mode skips it with the reason.
        let selected = [SwitchId::DesktopIcons, SwitchId::Taskbar];
        rig.engine.start(now(), &selected, &ctx()).unwrap();
        assert_eq!(rig.engine.state().applied, vec![SwitchId::DesktopIcons]);
        assert_eq!(rig.engine.state().skipped.len(), 1);
        assert_eq!(rig.engine.state().skipped[0].reason, "fake probe failure");
    }

    #[test]
    fn unavailable_switch_cannot_be_set() {
        let dir = test_dir("set-unavailable");
        let mut rig = build(&dir, |i| {
            let mut o = manual_opts();
            if i == 1 {
                o.availability = Availability::NeedsWindows11;
            }
            o
        });
        let err = rig
            .engine
            .set_switch(SwitchId::Taskbar, true, None, now(), &ctx())
            .unwrap_err();
        assert!(matches!(err, SwitchError::NotAvailable(_)));
    }

    #[test]
    fn restore_all_now_also_restores_future_timers() {
        let dir = test_dir("restore-all-now");
        let mut rig = build(&dir, |_| manual_opts());
        let deadline = datetime!(2027-01-01 08:00:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: deadline }),
                now(),
                &ctx(),
            )
            .unwrap();
        assert!(rig.engine.has_timers());
        rig.engine.restore_all_now(now()).unwrap();
        assert!(
            !rig.handles[idx(SwitchId::Taskbar)].is_on(),
            "future timer restored"
        );
        assert_eq!(journal_len(&dir), 0);

        // With an active session, it stops the session too.
        rig.engine.start(now(), &IDS, &ctx()).unwrap();
        assert!(
            rig.engine
                .set_switch(
                    SwitchId::KeepAwake,
                    true,
                    Some(OffAfter::At { at: deadline }),
                    later(),
                    &ctx(),
                )
                .is_err(),
            "keep-awake is owned by the active session"
        );
        rig.engine.restore_all_now(later()).unwrap();
        assert_eq!(rig.engine.state().status, SessionStatus::Idle);
        assert!(all_off(&rig));
        assert_eq!(journal_len(&dir), 0);
    }

    #[test]
    fn a_rearmed_timer_fires_after_recovery() {
        // A timer entry on disk with a future deadline survives recovery and
        // is fired by the ticker path (spec 7.7).
        let dir = test_dir("timer-after-recovery");
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
        let mut rig = build(&dir, |_| manual_opts());
        rig.engine.recover(now()).unwrap();
        rig.handles[idx(SwitchId::Taskbar)].set(true); // the OS still has it on
        assert!(rig.engine.has_timers());
        assert!(rig.engine.fire_due_timers(later()));
        assert!(!rig.handles[idx(SwitchId::Taskbar)].is_on());
        assert_eq!(journal_len(&dir), 0);
    }

    #[test]
    fn the_row_cache_keeps_the_ticker_read_free() {
        let dir = test_dir("ticker-zero-reads");
        let mut rig = build(&dir, |_| manual_opts());
        rig.engine.refresh_rows();
        let deadline = datetime!(2026-10-26 16:00:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::DesktopIcons,
                true,
                Some(OffAfter::At { at: deadline }),
                now(),
                &ctx(),
            )
            .unwrap();
        let reads: Vec<usize> = rig.handles.iter().map(|h| h.read_count()).collect();
        // What the ticker does every second: has_timers + switch_states.
        let _timers = rig.engine.has_timers();
        let _states = rig.engine.switch_states();
        let _states = rig.engine.switch_states();
        let after: Vec<usize> = rig.handles.iter().map(|h| h.read_count()).collect();
        assert_eq!(reads, after, "the ticker path does zero switch reads");
    }

    #[test]
    fn a_failed_restore_keeps_the_timer_entry() {
        let dir = test_dir("restore-failed-keeps-entry");
        let mut rig = build(&dir, |i| {
            let mut o = manual_opts();
            if i == 1 {
                o.fail_restore = true;
            }
            o
        });
        let deadline = datetime!(2026-10-26 12:30:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: deadline }),
                now(),
                &ctx(),
            )
            .unwrap();
        // Turning it off must fail and keep the recovery record on disk.
        assert!(
            rig.engine
                .set_switch(SwitchId::Taskbar, false, None, now(), &ctx())
                .is_err()
        );
        assert!(rig.handles[idx(SwitchId::Taskbar)].is_on());
        assert_eq!(journal_len(&dir), 1, "the timer entry survives the failure");
        // The fired timer retries instead of deleting the record.
        assert!(!rig.engine.fire_due_timers(deadline));
        assert_eq!(journal_len(&dir), 1);
        assert!(rig.engine.has_timers());
    }

    #[test]
    fn restore_all_now_counts_and_keeps_failed_entries() {
        let dir = test_dir("restore-all-now-failures");
        let mut rig = build(&dir, |i| {
            let mut o = manual_opts();
            if i == 1 {
                o.fail_restore = true;
            }
            o
        });
        let deadline = datetime!(2027-01-01 08:00:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: deadline }),
                now(),
                &ctx(),
            )
            .unwrap();
        let failed = rig.engine.restore_all_now(now()).unwrap();
        assert_eq!(failed, 1);
        assert!(rig.handles[idx(SwitchId::Taskbar)].is_on());
        assert_eq!(
            journal_len(&dir),
            1,
            "the failed entry stays for the next recovery"
        );
    }

    #[test]
    fn turning_on_after_an_out_of_band_off_still_applies() {
        let dir = test_dir("turn-on-after-off");
        let mut rig = build(&dir, |_| manual_opts());
        let deadline = datetime!(2026-10-26 13:00:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: deadline }),
                now(),
                &ctx(),
            )
            .unwrap();
        rig.handles[idx(SwitchId::Taskbar)].set(false); // the user undid it
        rig.engine
            .set_switch(SwitchId::Taskbar, true, None, now(), &ctx())
            .unwrap();
        assert!(
            rig.handles[idx(SwitchId::Taskbar)].is_on(),
            "the plain on still applies after the timer is dropped"
        );
        assert_eq!(journal_len(&dir), 0);
    }

    #[test]
    fn a_timer_fires_while_a_session_is_active() {
        let dir = test_dir("timer-during-session");
        let mut rig = build(&dir, |_| manual_opts());
        let selected = [SwitchId::DesktopIcons];
        rig.engine.start(now(), &selected, &ctx()).unwrap();
        let deadline = datetime!(2026-10-26 12:20:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: deadline }),
                now(),
                &ctx(),
            )
            .unwrap();
        assert!(rig.engine.fire_due_timers(deadline));
        assert!(!rig.handles[idx(SwitchId::Taskbar)].is_on());
        // Spec 7.7: the session is untouched — still active, same summary.
        assert_eq!(rig.engine.state().status, SessionStatus::Active);
        assert_eq!(rig.engine.state().applied, vec![SwitchId::DesktopIcons]);
        assert!(rig.engine.state().skipped.is_empty());
        assert!(rig.engine.state().failed.is_empty());
    }

    #[test]
    fn set_timer_apply_failure_pops_the_entry() {
        let dir = test_dir("timer-apply-failure");
        let mut rig = build(&dir, |i| {
            let mut o = manual_opts();
            if i == 0 {
                o.fail_apply = true;
            }
            o
        });
        let deadline = datetime!(2026-10-26 12:40:00 UTC);
        assert!(
            rig.engine
                .set_switch(
                    SwitchId::DesktopIcons,
                    true,
                    Some(OffAfter::At { at: deadline }),
                    now(),
                    &ctx(),
                )
                .is_err()
        );
        assert_eq!(journal_len(&dir), 0, "the never-applied entry is popped");
        assert!(!rig.engine.has_timers());
    }

    #[test]
    fn timer_deadline_move_rolls_back_when_the_journal_write_fails() {
        let dir = test_dir("deadline-move-rollback");
        let mut rig = build(&dir, |_| manual_opts());
        let first = datetime!(2026-10-26 13:00:00 UTC);
        rig.engine
            .set_switch(
                SwitchId::Taskbar,
                true,
                Some(OffAfter::At { at: first }),
                now(),
                &ctx(),
            )
            .unwrap();
        // A directory where write_atomic wants its tmp file makes the
        // rewrite fail (File::create cannot open a directory).
        std::fs::create_dir_all(dir.join("journal.json.tmp")).unwrap();
        let second = datetime!(2026-10-26 14:00:00 UTC);
        assert!(
            rig.engine
                .set_switch(
                    SwitchId::Taskbar,
                    true,
                    Some(OffAfter::At { at: second }),
                    now(),
                    &ctx(),
                )
                .is_err()
        );
        assert_eq!(journal_len(&dir), 1);
        assert_eq!(
            timer_entry(&dir, SwitchId::Taskbar).unwrap().2,
            Some(first),
            "disk keeps the old deadline"
        );
    }

    #[test]
    fn manual_record_survives_a_failed_restore() {
        let dir = test_dir("manual-record-survives");
        let mut rig = build(&dir, |i| {
            let mut o = manual_opts();
            if i == 1 {
                o.fail_restore = true;
            }
            o
        });
        rig.engine
            .set_switch(SwitchId::Taskbar, true, None, now(), &ctx())
            .unwrap();
        assert!(
            rig.engine
                .set_switch(SwitchId::Taskbar, false, None, now(), &ctx())
                .is_err()
        );
        // The record was put back: the retry fails the same way instead of
        // succeeding through target_off.
        assert!(
            rig.engine
                .set_switch(SwitchId::Taskbar, false, None, later(), &ctx())
                .is_err()
        );
        assert!(rig.handles[idx(SwitchId::Taskbar)].is_on());
    }

    #[test]
    fn set_switch_rejects_unregistered_ids() {
        let dir = test_dir("unregistered-id");
        let mut engine = SessionEngine::new(
            Vec::new(),
            OsInfo {
                build: 26200,
                is_windows_11: true,
            },
            dir,
            "0.1.0".into(),
        );
        let err = engine
            .set_switch(SwitchId::Dnd, true, None, now(), &ctx())
            .unwrap_err();
        assert!(matches!(err, SwitchError::Invalid(_)));
    }

    // ---- offAfter parsing (task 4.2) ----

    #[test]
    fn off_after_presets_parse() {
        assert_eq!(
            parse_off_after(&OffAfter::Preset("15m".into()), now()).unwrap(),
            datetime!(2026-10-26 12:15:00 UTC)
        );
        assert_eq!(
            parse_off_after(&OffAfter::Preset("1h".into()), now()).unwrap(),
            datetime!(2026-10-26 13:00:00 UTC)
        );
        assert_eq!(
            parse_off_after(&OffAfter::Preset("4h".into()), now()).unwrap(),
            datetime!(2026-10-26 16:00:00 UTC)
        );
        assert!(parse_off_after(&OffAfter::Preset("5m".into()), now()).is_err());
        assert!(parse_off_after(&OffAfter::Minutes { minutes: 0 }, now()).is_err());
        assert!(parse_off_after(&OffAfter::Minutes { minutes: 1441 }, now()).is_err());
        assert_eq!(
            parse_off_after(&OffAfter::Minutes { minutes: 90 }, now()).unwrap(),
            datetime!(2026-10-26 13:30:00 UTC)
        );
        let past = OffAfter::At {
            at: datetime!(2026-10-26 11:00:00 UTC),
        };
        assert!(parse_off_after(&past, now()).is_err(), "at must be future");
        let future = OffAfter::At {
            at: datetime!(2026-10-26 18:00:00 UTC),
        };
        assert_eq!(
            parse_off_after(&future, now()).unwrap(),
            datetime!(2026-10-26 18:00:00 UTC)
        );
    }

    #[test]
    fn tomorrow_0800_is_the_next_local_eight_oclock() {
        // Zone-independent check (CI runs UTC, the reference PC +05:30):
        // the result lands on 08:00 local, on the date the D-079 rule
        // picks from the chooser's local time.
        let eight = time::Time::from_hms(8, 0, 0).unwrap();
        for chooser in [
            datetime!(2026-10-26 2:00:00 UTC),  // early morning UTC
            datetime!(2026-10-26 12:00:00 UTC), // afternoon
            datetime!(2026-10-26 23:30:00 UTC), // late night
        ] {
            let at = parse_off_after(&OffAfter::Preset("tomorrow".into()), chooser).unwrap();
            assert!(at > chooser, "tomorrow is always in the future");
            let at_local = at.to_offset(UtcOffset::local_offset_at(at).unwrap());
            assert_eq!(at_local.time(), eight, "lands on 08:00 local");
            let chooser_local = chooser.to_offset(UtcOffset::local_offset_at(chooser).unwrap());
            let expected_date = next_0800_date(chooser_local.date(), chooser_local.time()).unwrap();
            assert_eq!(at_local.date(), expected_date);
        }
    }

    #[test]
    fn next_0800_date_follows_the_d079_rule() {
        use time::macros::date;
        let d = date!(2026 - 10 - 26);
        let before_eight = time::Time::from_hms(7, 59, 0).unwrap();
        let at_eight = time::Time::from_hms(8, 0, 0).unwrap();
        let evening = time::Time::from_hms(21, 15, 0).unwrap();
        assert_eq!(next_0800_date(d, before_eight).unwrap(), d);
        assert_eq!(
            next_0800_date(d, at_eight).unwrap(),
            date!(2026 - 10 - 27),
            "exactly 08:00 counts as at-or-after"
        );
        assert_eq!(next_0800_date(d, evening).unwrap(), date!(2026 - 10 - 27));
    }

    #[test]
    fn serde_off_after_shapes() {
        // The IPC shapes (spec 6.1): preset strings, {minutes}, {at}.
        assert_eq!(
            serde_json::from_value::<OffAfter>(serde_json::json!("15m")).unwrap(),
            OffAfter::Preset("15m".into())
        );
        assert_eq!(
            serde_json::from_value::<OffAfter>(serde_json::json!({ "minutes": 30 })).unwrap(),
            OffAfter::Minutes { minutes: 30 }
        );
        assert_eq!(
            serde_json::from_value::<OffAfter>(serde_json::json!({
                "at": "2026-10-26T18:00:00Z"
            }))
            .unwrap(),
            OffAfter::At {
                at: datetime!(2026-10-26 18:00:00 UTC)
            }
        );
        assert!(serde_json::from_value::<OffAfter>(serde_json::json!(5)).is_err());
    }
}
