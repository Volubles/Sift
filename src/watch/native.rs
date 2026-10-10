//! Blocking queue/shutdown wait. Both event handles stay owned across the wait;
//! the callback thread never owns a UI or audio object.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects,
};

#[derive(Clone)]
pub(super) struct StopEvent {
    handle: Arc<OwnedHandle>,
    requested: Arc<AtomicBool>,
}

pub(super) enum Wake {
    Changes,
    Stop,
}

impl StopEvent {
    pub(super) fn new() -> io::Result<Self> {
        // SAFETY: null optional attributes/name; this creates a private event.
        let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful CreateEventW transfers this unique handle to us.
        Ok(Self {
            handle: Arc::new(unsafe { OwnedHandle::from_raw_handle(handle) }),
            requested: Arc::new(AtomicBool::new(false)),
        })
    }
    pub(super) fn signal(&self) -> io::Result<()> {
        self.requested.store(true, Ordering::Relaxed);
        // SAFETY: Arc ownership keeps the valid, modifiable event alive.
        if unsafe { SetEvent(self.handle.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub(super) fn wait(&self, changes: &OwnedHandle) -> io::Result<Wake> {
        if self.is_requested() {
            return Ok(Wake::Stop);
        }
        let handles = [self.handle.as_raw_handle(), changes.as_raw_handle()];
        // SAFETY: both owned event handles and this two-element array outlive the
        // blocking call. Shutdown wins when both events are signalled.
        match unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) } {
            WAIT_OBJECT_0 => Ok(Wake::Stop),
            value if value == WAIT_OBJECT_0 + 1 => Ok(Wake::Changes),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            _ => Err(io::Error::other("Unexpected file-watch wait result")),
        }
    }
    pub(super) fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Relaxed)
    }
}
