//! UI-thread message hosting for modal native file gestures.

use super::{drag, native_error};
use crate::platform::{DragError, DragOutcome};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender},
};
use windows::Win32::{
    Foundation::{
        ERROR_CLASS_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT,
        SetLastError, WPARAM,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::VK_ESCAPE,
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
            HWND_MESSAGE, PostMessageW, PostThreadMessageW, RegisterClassW, SetWindowLongPtrW,
            WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_KEYDOWN, WNDCLASSW,
        },
    },
};

const RUN_DRAG: u32 = WM_APP + 1;
const HOST_CLASS: windows_core::PCWSTR = windows_core::w!("Sift.NativeFileDrag.v1");

type Outcome = Result<DragOutcome, DragError>;
type Wake = Arc<dyn Fn() + Send + Sync>;

struct NativeRequest {
    bytes: Vec<u8>,
    cancelled: Arc<AtomicBool>,
}

struct HostState {
    pending: RefCell<Option<NativeRequest>>,
    active: Cell<bool>,
    results: SyncSender<Outcome>,
    wake: Wake,
    #[cfg(test)]
    callback_thread: Cell<Option<std::thread::ThreadId>>,
}

/// A message-only window bound to the creator's UI thread. Rc deliberately
/// prevents moving this host or its owning coordinator to another thread.
pub(in crate::platform) struct DragHost {
    window: HWND,
    state: Rc<HostState>,
    results: Receiver<Outcome>,
}

impl DragHost {
    pub(in crate::platform) fn new(wake: Wake) -> windows_core::Result<Self> {
        // SAFETY: a static class name and callback. Register once per process;
        // subsequent hosts reuse the class, and each owns its private HWND.
        let instance = HINSTANCE(unsafe { GetModuleHandleW(None)? }.0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(drag_host_proc),
            hInstance: instance,
            lpszClassName: HOST_CLASS,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0
            && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
        {
            return Err(windows_core::Error::from_thread());
        }
        // SAFETY: this hidden message-only window belongs to this thread. It is
        // never shown, activated, or used to inject input into other apps.
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                HOST_CLASS,
                windows_core::w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            )?
        };
        let (sender, results) = mpsc::sync_channel(1);
        let state = Rc::new(HostState {
            pending: RefCell::new(None),
            active: Cell::new(false),
            results: sender,
            wake,
            #[cfg(test)]
            callback_thread: Cell::new(None),
        });
        let host = Self {
            window,
            state,
            results,
        };
        // SAFETY: Rc's heap allocation stays stable until this host clears the
        // user-data pointer/destroys its HWND. Callbacks retain an extra Rc.
        set_user_data(window, Rc::as_ptr(&host.state) as isize)?;
        Ok(host)
    }

    pub(in crate::platform) fn queue(
        &self,
        bytes: Vec<u8>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(), DragError> {
        if self.state.active.get() || self.state.pending.borrow().is_some() {
            return Err(DragError::Busy);
        }
        self.state
            .pending
            .replace(Some(NativeRequest { bytes, cancelled }));
        // SAFETY: PostMessage queues work; it does not invoke the callback while
        // egui or winit is borrowed. No pointers are transported in this message.
        if let Err(error) =
            unsafe { PostMessageW(Some(self.window), RUN_DRAG, WPARAM(0), LPARAM(0)) }
        {
            self.state.pending.take();
            return Err(native_error(error));
        }
        Ok(())
    }

    pub(in crate::platform) fn active(&self) -> bool {
        self.state.active.get()
    }

    pub(in crate::platform) fn poll(&self) -> Option<Outcome> {
        match self.results.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(DragError::Disconnected)),
        }
    }
}

fn set_user_data(window: HWND, pointer: isize) -> windows_core::Result<()> {
    // SAFETY: the HWND belongs to this thread. Zero is a successful previous
    // value, so GetLastError distinguishes it from a failed update.
    unsafe { SetLastError(ERROR_SUCCESS) };
    let previous = unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, pointer) };
    if previous == 0 && unsafe { GetLastError() } != ERROR_SUCCESS {
        return Err(windows_core::Error::from_thread());
    }
    Ok(())
}

impl Drop for DragHost {
    fn drop(&mut self) {
        if let Some(request) = self.state.pending.take() {
            request.cancelled.store(true, Ordering::Release);
        }
        if self.state.active.get() {
            // The coordinator sets the active request's cancellation flag first.
            // Wake only this input thread in case disposal occurs reentrantly.
            if let Err(error) = unsafe {
                PostThreadMessageW(
                    GetCurrentThreadId(),
                    WM_KEYDOWN,
                    WPARAM(VK_ESCAPE.0 as usize),
                    LPARAM(0),
                )
            } {
                eprintln!("Native drag disposal wake failed: {error}");
            }
        }
        let cleared = set_user_data(self.window, 0);
        // SAFETY: this host owns this HWND on its creator's thread.
        let destroyed = unsafe { DestroyWindow(self.window) };
        if let Err(error) = &cleared {
            eprintln!("Native drag callback cleanup failed: {error}");
        }
        if let Err(error) = &destroyed {
            eprintln!("Native drag window cleanup failed: {error}");
        }
        if cleared.is_err() && destroyed.is_err() {
            // An unreleased window could still hold the callback pointer. Retain
            // its small state rather than permit a use-after-free on API failure.
            let _retained_state = Rc::into_raw(self.state.clone());
        }
    }
}

unsafe extern "system" fn drag_host_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Windows invokes this callback on the HWND's owner thread. User data
    // is either zero or the stable Rc allocation installed by DragHost::new.
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const HostState;
    if message != RUN_DRAG || pointer.is_null() {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    // Keep the allocation alive even if a nested event closes the application.
    unsafe { Rc::increment_strong_count(pointer) };
    let state = unsafe { Rc::from_raw(pointer) };
    let request = state.pending.take(); // no RefCell borrow across the OLE loop
    let Some(request) = request else {
        return LRESULT(0);
    };
    state.active.set(true);
    #[cfg(test)]
    state.callback_thread.set(Some(std::thread::current().id()));
    // No Rust panic may unwind through a Win32 callback. OLE pumps normal window
    // messages now that the application/event-handler borrow has ended.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drag(request.bytes, request.cancelled)
    }))
    .unwrap_or_else(|_| Err(DragError::Native("Native drag callback panicked".into())));
    state.active.set(false);
    match state.results.try_send(outcome) {
        Ok(()) => {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (state.wake)())).is_err() {
                eprintln!("Native drag completion notification panicked");
            }
        }
        Err(mpsc::TrySendError::Disconnected(_)) => {} // owner closed during the gesture
        Err(mpsc::TrySendError::Full(_)) => eprintln!("Native drag result queue unexpectedly full"),
    }
    LRESULT(0)
}

#[cfg(test)]
mod tests {
    use super::super::drop_bytes;
    use super::*;
    use crate::platform::DragWorker;
    use std::time::Duration;
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW};

    #[test]
    fn releases_over_owned_windows_or_no_window_cancel_before_a_drop() {
        let host = DragHost::new(Arc::new(|| {})).unwrap();
        assert_eq!(
            super::super::release_over_window(host.window),
            windows::Win32::Foundation::DRAGDROP_S_CANCEL
        );
        assert_eq!(
            super::super::release_over_window(HWND::default()),
            windows::Win32::Foundation::DRAGDROP_S_CANCEL
        );
    }

    #[test]
    fn native_start_is_deferred_to_the_owner_queue_and_cancelled_jobs_clear_busy_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.wav");
        std::fs::write(&path, b"unchanged source").unwrap();
        let owner = std::thread::current().id();
        let (wake, ready) = mpsc::sync_channel(2);
        let mut worker = DragWorker::start_with(
            move || wake.send(()).unwrap(),
            |sources, _| drop_bytes(sources),
        )
        .unwrap();
        worker.request(vec![path.clone()]).unwrap();
        ready.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(worker.poll().is_none());
        assert!(worker.busy());
        assert!(worker.native_started);
        let host = worker.native.as_ref().unwrap();
        assert_eq!(host.state.callback_thread.get(), None);
        let window = host.window;
        // No native call occurs while the UI/coordinator is borrowed. Cancelling
        // before the queued message runs never captures the user's mouse.
        worker.cancel();
        let mut message = MSG::default();
        assert!(
            unsafe { PeekMessageW(&mut message, Some(window), RUN_DRAG, RUN_DRAG, PM_REMOVE) }
                .as_bool()
        );
        unsafe { DispatchMessageW(&message) };
        assert_eq!(
            worker.native.as_ref().unwrap().state.callback_thread.get(),
            Some(owner)
        );
        ready.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(worker.poll().unwrap().unwrap(), DragOutcome::Cancelled);
        assert!(!worker.busy());
        assert!(!worker.native_active());
        assert_eq!(std::fs::read(path).unwrap(), b"unchanged source");
    }
}
