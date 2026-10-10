//! Bounded native file transfers. Destination applications own the actual copy.

use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
use std::thread::{self, JoinHandle};

#[cfg(windows)]
mod windows_drag;

const MAX_FILES: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum DragError {
    #[error("Choose between 1 and {MAX_FILES} source files")]
    InvalidCount,
    #[error("A source must have an absolute path")]
    RelativePath,
    #[error("A source is missing or inaccessible: {0}")]
    Source(#[from] std::io::Error),
    #[error("A source is not a regular file, or is a link/reparse point")]
    InvalidSource,
    #[error("A source path contains a null character or exceeds the transfer limit")]
    InvalidPath,
    #[error(
        "Windows Shell cannot safely represent this source path; use a normal drive or UNC path without reserved names or trailing spaces/dots"
    )]
    ShellPath,
    #[error("Another file drag is still active")]
    Busy,
    #[error("The file drag worker is unavailable; reopen Sift and try again")]
    Disconnected,
    #[error("The native file drag must run on the thread that owns the Sift window")]
    WrongThread,
    #[error("External file dragging is currently supported only on Windows")]
    Unsupported,
    #[error("Windows file transfer failed: {0}")]
    Native(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragOutcome {
    /// A target accepted the copy offer; it may still be copying asynchronously.
    Accepted,
    Cancelled,
    Rejected,
}

struct Sources(Vec<PathBuf>);

struct Request {
    sources: Sources,
    cancelled: Arc<AtomicBool>,
}

enum Prepared {
    Files(Vec<u8>),
    Cancelled,
}

enum Presentation {
    #[cfg(windows)]
    Deferred,
    Complete(DragOutcome),
}

impl Sources {
    fn new(paths: Vec<PathBuf>) -> Result<Self, DragError> {
        if paths.is_empty() || paths.len() > MAX_FILES {
            return Err(DragError::InvalidCount);
        }
        if paths.iter().any(|path| !path.is_absolute()) {
            return Err(DragError::RelativePath);
        }
        Ok(Self(paths))
    }

    fn validate(&self, shutdown: &AtomicBool) -> Result<bool, DragError> {
        for path in &self.0 {
            if shutdown.load(Ordering::Acquire) {
                return Ok(false);
            }
            let metadata = path.symlink_metadata()?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(DragError::InvalidSource);
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                // Match the scanner: do not unexpectedly export a replaced link.
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(DragError::InvalidSource);
                }
            }
        }
        Ok(true)
    }
}

pub struct DragWorker {
    requests: Option<SyncSender<Request>>,
    results: Option<Receiver<Result<Prepared, DragError>>>,
    pending: Option<Arc<AtomicBool>>,
    owner: thread::ThreadId,
    thread: Option<JoinHandle<()>>,
    #[cfg(windows)]
    native_started: bool,
    #[cfg(windows)]
    native: Option<windows_drag::DragHost>,
}

impl DragWorker {
    pub const fn supported() -> bool {
        cfg!(windows)
    }

    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
        Self::start_with(wake, |sources, _cancelled| {
            #[cfg(windows)]
            {
                windows_drag::drop_bytes(sources)
            }
            #[cfg(not(windows))]
            {
                let _unused = sources;
                Err(DragError::Unsupported)
            }
        })
    }

    fn start_with(
        wake: impl Fn() + Send + Sync + 'static,
        prepare: impl Fn(&Sources, Arc<AtomicBool>) -> Result<Vec<u8>, DragError> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (requests, jobs) = mpsc::sync_channel::<Request>(1);
        let (sender, results) = mpsc::sync_channel(1);
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        #[cfg(windows)]
        let native = windows_drag::DragHost::new(wake.clone())
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let thread = thread::Builder::new()
            .name("sift-drag-prepare".into())
            .spawn(move || {
                while let Ok(Request { sources, cancelled }) = jobs.recv() {
                    let result = sources.validate(&cancelled).and_then(|valid| {
                        if valid {
                            prepare(&sources, cancelled).map(Prepared::Files)
                        } else {
                            Ok(Prepared::Cancelled)
                        }
                    });
                    if sender.send(result).is_err() {
                        break;
                    }
                    wake();
                }
            })?;
        Ok(Self {
            requests: Some(requests),
            results: Some(results),
            pending: None,
            owner: thread::current().id(),
            thread: Some(thread),
            #[cfg(windows)]
            native_started: false,
            #[cfg(windows)]
            native: Some(native),
        })
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn native_active(&self) -> bool {
        #[cfg(windows)]
        {
            self.native.as_ref().is_some_and(|host| host.active())
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn request(&mut self, paths: Vec<PathBuf>) -> Result<(), DragError> {
        if self.busy() {
            return Err(DragError::Busy);
        }
        let sources = Sources::new(paths)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        match self
            .requests
            .as_ref()
            .ok_or(DragError::Disconnected)?
            .try_send(Request {
                sources,
                cancelled: cancelled.clone(),
            }) {
            Ok(()) => {
                self.pending = Some(cancelled);
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(DragError::Busy),
            Err(TrySendError::Disconnected(_)) => Err(DragError::Disconnected),
        }
    }

    pub fn poll(&mut self) -> Option<Result<DragOutcome, DragError>> {
        #[cfg(windows)]
        {
            if self.native_started {
                let result = self.native.as_ref()?.poll()?;
                self.pending = None;
                self.native_started = false;
                return Some(result);
            }
            let host = self.native.take()?;
            let result = self.poll_with(|bytes, cancelled| {
                host.queue(bytes, cancelled)?;
                Ok(Presentation::Deferred)
            });
            self.native = Some(host);
            result
        }
        #[cfg(not(windows))]
        {
            self.poll_with(|_, _| Err(DragError::Unsupported))
        }
    }

    pub fn cancel(&self) {
        if let Some(cancelled) = &self.pending {
            cancelled.store(true, Ordering::Release);
        }
    }

    fn poll_with(
        &mut self,
        present: impl FnOnce(Vec<u8>, Arc<AtomicBool>) -> Result<Presentation, DragError>,
    ) -> Option<Result<DragOutcome, DragError>> {
        let cancelled = self.pending.as_ref()?.clone();
        let prepared = match self.results.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err(DragError::Disconnected),
        };
        let result = if cancelled.load(Ordering::Acquire) {
            Ok(Presentation::Complete(DragOutcome::Cancelled))
        } else if thread::current().id() != self.owner {
            Err(DragError::WrongThread)
        } else {
            prepared.and_then(|prepared| match prepared {
                // The modal native gesture must use the window's input queue.
                // No filesystem work or COM objects cross back from the worker.
                Prepared::Files(bytes) => present(bytes, cancelled),
                Prepared::Cancelled => Ok(Presentation::Complete(DragOutcome::Cancelled)),
            })
        };
        match result {
            #[cfg(windows)]
            Ok(Presentation::Deferred) => {
                self.native_started = true;
                None
            }
            Ok(Presentation::Complete(outcome)) => {
                self.pending = None;
                Some(Ok(outcome))
            }
            Err(error) => {
                self.pending = None;
                Some(Err(error))
            }
        }
    }
}

impl Drop for DragWorker {
    fn drop(&mut self) {
        self.cancel();
        self.requests.take();
        self.results.take();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("File drag worker panicked during shutdown");
        }
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/platform/mod.rs"
    ));
}
