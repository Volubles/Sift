//! Bounded native file transfers. Destination applications own the actual copy.

use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
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
    requests: Option<SyncSender<Sources>>,
    results: Option<Receiver<Result<DragOutcome, DragError>>>,
    shutdown: Arc<AtomicBool>,
    native_thread: Arc<AtomicU32>,
    thread: Option<JoinHandle<()>>,
    busy: bool,
}

impl DragWorker {
    pub const fn supported() -> bool {
        cfg!(windows)
    }

    pub fn start(wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        Self::start_with(wake, |sources, shutdown| {
            #[cfg(windows)]
            {
                windows_drag::drag(sources, shutdown)
            }
            #[cfg(not(windows))]
            {
                let _unused = (sources, shutdown);
                Err(DragError::Unsupported)
            }
        })
    }

    fn start_with(
        wake: impl Fn() + Send + 'static,
        transfer: impl Fn(&Sources, Arc<AtomicBool>) -> Result<DragOutcome, DragError> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (requests, jobs) = mpsc::sync_channel::<Sources>(1);
        let (sender, results) = mpsc::sync_channel(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let stop = shutdown.clone();
        let native_thread = Arc::new(AtomicU32::new(0));
        let worker_id = native_thread.clone();
        let thread = thread::Builder::new()
            .name("sift-file-drag".into())
            .spawn(move || {
                #[cfg(windows)]
                worker_id.store(windows_drag::prepare_thread(), Ordering::Release);
                while let Ok(sources) = jobs.recv() {
                    let result = sources.validate(&stop).and_then(|valid| {
                        if valid {
                            transfer(&sources, stop.clone())
                        } else {
                            Ok(DragOutcome::Cancelled)
                        }
                    });
                    if sender.send(result).is_err() {
                        break;
                    }
                    wake();
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                }
                worker_id.store(0, Ordering::Release);
            })?;
        Ok(Self {
            requests: Some(requests),
            results: Some(results),
            shutdown,
            native_thread,
            thread: Some(thread),
            busy: false,
        })
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn request(&mut self, paths: Vec<PathBuf>) -> Result<(), DragError> {
        if self.busy {
            return Err(DragError::Busy);
        }
        let sources = Sources::new(paths)?;
        #[cfg(windows)]
        windows_drag::release_pointer_capture()?;
        match self
            .requests
            .as_ref()
            .ok_or(DragError::Disconnected)?
            .try_send(sources)
        {
            Ok(()) => {
                self.busy = true;
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(DragError::Busy),
            Err(TrySendError::Disconnected(_)) => Err(DragError::Disconnected),
        }
    }

    pub fn poll(&mut self) -> Option<Result<DragOutcome, DragError>> {
        let result = match self.results.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) if self.busy => Err(DragError::Disconnected),
            Err(mpsc::TryRecvError::Disconnected) => return None,
        };
        self.busy = false;
        Some(result)
    }
}

impl Drop for DragWorker {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        #[cfg(windows)]
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            windows_drag::wake_cancellation(self.native_thread.load(Ordering::Acquire));
        }
        #[cfg(not(windows))]
        let _unused = &self.native_thread;
        self.requests.take();
        self.results.take();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("File drag worker panicked during shutdown");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn paths_and_selection_count_are_bounded() {
        assert!(matches!(Sources::new(vec![]), Err(DragError::InvalidCount)));
        assert!(matches!(
            Sources::new(vec![PathBuf::from("relative.wav")]),
            Err(DragError::RelativePath)
        ));
        let directory = tempfile::tempdir().unwrap();
        assert!(matches!(
            Sources::new(vec![directory.path().join("sound.wav"); MAX_FILES + 1]),
            Err(DragError::InvalidCount)
        ));
    }

    #[test]
    fn missing_files_and_directories_are_rejected_without_writes() {
        let directory = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let sources = Sources::new(vec![directory.path().join("missing.wav")]).unwrap();
        assert!(matches!(sources.validate(&stop), Err(DragError::Source(_))));
        let sources = Sources::new(vec![directory.path().to_owned()]).unwrap();
        assert!(matches!(
            sources.validate(&stop),
            Err(DragError::InvalidSource)
        ));
        stop.store(true, Ordering::Release);
        assert!(!sources.validate(&stop).unwrap());
    }

    #[test]
    fn worker_allows_one_transfer_and_joins_when_result_is_unread() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.wav");
        std::fs::write(&path, b"unchanged fixture").unwrap();
        let (entered, entry) = mpsc::sync_channel(1);
        let mut worker = DragWorker::start_with(
            || {},
            move |_, stop| {
                entered.send(()).unwrap();
                while !stop.load(Ordering::Acquire) {
                    thread::park_timeout(Duration::from_millis(1));
                }
                Ok(DragOutcome::Cancelled)
            },
        )
        .unwrap();
        worker.request(vec![path.clone()]).unwrap();
        entry.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(worker.busy());
        assert!(matches!(
            worker.request(vec![path.clone()]),
            Err(DragError::Busy)
        ));
        drop(worker);
        assert_eq!(std::fs::read(path).unwrap(), b"unchanged fixture");
    }
}
