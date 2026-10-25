//! FakeSwitch: the only "switch" registered in debug builds (D-063, D-033).
//! It never touches Windows. State shape: `{"on": bool}`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::{Availability, Ctx, RestoreOutcome, Switch, SwitchError, SwitchId};
use crate::journal::JournalEntry;
use crate::os::OsInfo;

/// Test-mode options (spec 11): fail apply, fail restore, or panic at call N
/// — either before the call changes anything, or after it changed and saved
/// its value (the dangerous crash window: entry journaled, state applied).
#[derive(Debug, Clone)]
pub struct FakeOptions {
    pub availability: Availability,
    pub persistent: bool,
    pub fail_apply: bool,
    pub fail_restore: bool,
    pub panic_at: Option<PanicAt>,
}

/// Where a fake should panic: at call N, before or after its mutation.
#[derive(Debug, Clone, Copy)]
pub struct PanicAt {
    pub call: usize,
    pub after_mutation: bool,
}

impl Default for FakeOptions {
    fn default() -> Self {
        Self {
            availability: Availability::Available,
            persistent: false,
            fail_apply: false,
            fail_restore: false,
            panic_at: None,
        }
    }
}

struct Inner {
    on: bool,
    calls: usize,
}

pub struct FakeSwitch {
    id: SwitchId,
    options: FakeOptions,
    state_file: Option<PathBuf>,
    inner: Arc<Mutex<Inner>>,
}

/// Observable view of a fake's state, usable even after the owning engine
/// was dropped (kill-point tests read and set the value through it).
#[derive(Clone)]
pub struct FakeHandle(Arc<Mutex<Inner>>);

impl FakeHandle {
    /// Read the current value; survives a poisoned mutex (the panicking call).
    pub fn is_on(&self) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).on
    }

    /// Simulate the user changing the value behind button's back.
    pub fn set(&self, on: bool) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).on = on;
    }
}

impl FakeSwitch {
    /// In-memory fake for tests (spec 11).
    pub fn new(id: SwitchId, options: FakeOptions) -> Self {
        Self {
            id,
            options,
            state_file: None,
            inner: Arc::new(Mutex::new(Inner {
                on: false,
                calls: 0,
            })),
        }
    }

    /// Debug-build fake whose persistent state survives restarts in
    /// `%LOCALAPPDATA%\io.github.chirag-gamer.button\fake-switches.json`.
    pub fn persistent(id: SwitchId, data_dir: PathBuf) -> Self {
        let state_file = data_dir.join("fake-switches.json");
        let on = load_state(&state_file, id).is_some_and(|v| v == json!(true));
        Self {
            id,
            options: FakeOptions::default(),
            state_file: Some(state_file),
            inner: Arc::new(Mutex::new(Inner { on, calls: 0 })),
        }
    }

    pub fn handle(&self) -> FakeHandle {
        FakeHandle(Arc::clone(&self.inner))
    }

    /// Panic check for the start of a call: fires when this call number
    /// matches and the timing is "before mutation".
    fn should_panic(&self, inner: &Inner) -> bool {
        matches!(&self.options.panic_at, Some(p) if p.call == inner.calls && !p.after_mutation)
    }

    /// Panic check for the end of a mutating call (after the value changed
    /// and any persistent state was saved).
    fn panic_after_mutation(&self, inner: &Inner) -> bool {
        matches!(&self.options.panic_at, Some(p) if p.after_mutation && p.call == inner.calls)
    }

    pub fn is_on(&self) -> bool {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).on
    }
}

fn load_state(path: &PathBuf, id: SwitchId) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&text).ok()?;
    map.get(id.to_string().as_str()).cloned()
}

fn save_state(path: &PathBuf, id: SwitchId, on: bool) {
    let mut map: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    map.insert(id.to_string(), json!(on));
    // Best effort: losing fake state only means a fake switches back to off.
    let _ = std::fs::write(path, serde_json::to_string(&map).unwrap_or_default());
}

impl Switch for FakeSwitch {
    fn id(&self) -> SwitchId {
        self.id
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        self.options.availability
    }

    fn persistent(&self) -> bool {
        self.options.persistent
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.calls += 1;
        if self.should_panic(&inner) {
            panic!("FakeSwitch {} panicked at call {}", self.id, inner.calls);
        }
        Ok(json!(inner.on))
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        json!(true)
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.calls += 1;
        if self.should_panic(&inner) {
            panic!("FakeSwitch {} panicked at call {}", self.id, inner.calls);
        }
        if self.options.fail_apply {
            return Err(SwitchError::NotAvailable("fake apply failure".into()));
        }
        inner.on = value.as_bool().unwrap_or(false);
        if let Some(path) = &self.state_file {
            save_state(path, self.id, inner.on);
        }
        if self.panic_after_mutation(&inner) {
            panic!(
                "FakeSwitch {} panicked after apply {}",
                self.id, inner.calls
            );
        }
        Ok(())
    }

    fn restore(&self, e: &JournalEntry) -> Result<RestoreOutcome, SwitchError> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.calls += 1;
        if self.should_panic(&inner) {
            panic!("FakeSwitch {} panicked at call {}", self.id, inner.calls);
        }
        if self.options.fail_restore {
            return Err(SwitchError::NotAvailable("fake restore failure".into()));
        }
        if inner.on == e.after.as_bool().unwrap_or(false) {
            inner.on = e.before.as_bool().unwrap_or(false);
            if let Some(path) = &self.state_file {
                save_state(path, self.id, inner.on);
            }
            if self.panic_after_mutation(&inner) {
                panic!(
                    "FakeSwitch {} panicked after restore {}",
                    self.id, inner.calls
                );
            }
            Ok(RestoreOutcome::Restored)
        } else {
            Ok(RestoreOutcome::UserChanged)
        }
    }
}
