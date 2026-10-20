//! COM apartment for the Shell calls in `autostart` and `deeplink` (fork).

use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

/// Enters a single-threaded COM apartment on this thread for the guard's
/// lifetime. Only a successful call (S_OK, or S_FALSE when the thread already
/// is in an STA, like the main thread) is balanced by `CoUninitialize`. If
/// the thread is already in the MTA, it stays there; the Shell objects used
/// here work in both.
///
/// Declare it before any COM object so it is dropped after them.
pub struct ComApartment {
    initialized: bool,
}

impl ComApartment {
    pub fn enter() -> Self {
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        Self { initialized }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}
