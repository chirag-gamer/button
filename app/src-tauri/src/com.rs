//! COM apartment guard for switches that call COM APIs (task 3.0).

use std::marker::PhantomData;
use std::rc::Rc;

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

use crate::switches::SwitchError;

/// Initializes COM for the calling thread (apartment-threaded) and balances
/// it on drop. `RPC_E_CHANGED_MODE` keeps going without the uninitialize,
/// because that apartment belongs to someone else (task 3.0). The guard is
/// thread-affine: it must be created and dropped on the same thread.
pub struct ComGuard {
    uninitialize: bool,
    // CoInitializeEx/CoUninitialize are thread-affine: this marker makes the
    // guard !Send, so it cannot drift to another thread between init and drop.
    _affine: PhantomData<Rc<()>>,
}

impl ComGuard {
    pub fn new() -> Result<Self, SwitchError> {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            Ok(Self {
                uninitialize: false,
                _affine: PhantomData,
            })
        } else if hr.is_ok() {
            // S_OK (first init) and S_FALSE (already initialized) both own a
            // matching CoUninitialize.
            Ok(Self {
                uninitialize: true,
                _affine: PhantomData,
            })
        } else {
            Err(SwitchError::Os {
                api: "CoInitializeEx",
                code: hr.0,
            })
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guard_initializes_and_balances_on_this_thread() {
        let first = ComGuard::new().unwrap();
        let second = ComGuard::new().unwrap(); // S_FALSE on an initialized thread
        drop(second);
        drop(first);
    }
}
