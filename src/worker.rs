use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::catalog::{Catalog, FolderPage, Sample, Scope, SearchPage};
use crate::collections::{Availability, Reference};
use crate::libraries::{self, Library, LibraryId, SourceId};
use crate::library::FileStamp;
use crate::library::{self, Cancellation, ScanReport};
use crate::search::SearchQuery;
mod index;

#[derive(Debug)]
pub enum Event {
    Ready,
    Page {
        generation: u64,
        result: std::result::Result<SearchPage, String>,
    },
    Libraries(std::result::Result<Vec<Library>, String>),
    LibraryEdited {
        token: u64,
        result: std::result::Result<EditOutcome, String>,
    },
    Folders {
        generation: u64,
        result: std::result::Result<FolderPage, String>,
    },
    Progress(ScanReport),
    ScanFinished(std::result::Result<ScanReport, String>),
    StartupFailed(String),
    SyncStarted,
    SyncFinished {
        report: ScanReport,
        touched: Vec<PathBuf>,
        full_roots: Vec<PathBuf>,
        forced_roots: Vec<PathBuf>,
    },
    WatchStatus {
        enabled: bool,
        message: String,
    },
    SourceReady {
        generation: u64,
        refresh: SourceRefresh,
        sample: Option<Sample>,
        stamp: Option<FileStamp>,
        availability: Availability,
    },
}

struct QueryRequest {
    generation: u64,
    query: SearchQuery,
    offset: usize,
    scope: Scope,
}

enum ReadRequest {
    Search(QueryRequest),
    Folders {
        generation: u64,
        parent: PathBuf,
        offset: usize,
    },
    Source {
        generation: u64,
        reference: Reference,
        refresh: SourceRefresh,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum SourceRefresh {
    Verify,
    ObservedWrite,
}

#[derive(Clone, Debug)]
pub enum LibraryEdit {
    Create(String),
    Rename(LibraryId, String),
    Remove(LibraryId),
    Attach(LibraryId, PathBuf),
    Detach(LibraryId, SourceId),
}

#[derive(Debug)]
pub struct EditOutcome {
    pub selected: Option<LibraryId>,
    pub scan_folder: Option<PathBuf>,
}

enum IndexRequest {
    Scan(PathBuf, Cancellation),
    Edit { token: u64, edit: LibraryEdit },
    Refresh,
    Wake,
    Live(bool),
}

pub struct Workers {
    events: Option<Receiver<Event>>,
    queries: Option<SyncSender<ReadRequest>>,
    scans: Option<SyncSender<IndexRequest>>,
    shutdown: Arc<AtomicBool>,
    latest_generation: Arc<AtomicU64>,
    latest_folder_generation: Arc<AtomicU64>,
    latest_source_generation: Arc<AtomicU64>,
    scan_cancellation: Option<Cancellation>,
    automatic_cancellation: Cancellation,
    threads: Vec<JoinHandle<()>>,
}

impl Workers {
    pub fn start(
        database: PathBuf,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let (event_sender, events) = mpsc::sync_channel(8);
        let (query_sender, query_receiver) = mpsc::sync_channel::<ReadRequest>(4);
        let (scan_sender, scan_receiver) = mpsc::sync_channel::<IndexRequest>(4);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let shutdown = Arc::new(AtomicBool::new(false));
        let latest_generation = Arc::new(AtomicU64::new(0));
        let reader_generation = latest_generation.clone();
        let automatic_cancellation = Cancellation::default();
        let automatic_token = automatic_cancellation.clone();
        let latest_folder_generation = Arc::new(AtomicU64::new(0));
        let folder_generation = latest_folder_generation.clone();
        let latest_source_generation = Arc::new(AtomicU64::new(0));
        let source_generation = latest_source_generation.clone();
        let mut workers = Self {
            events: Some(events),
            queries: Some(query_sender),
            scans: Some(scan_sender),
            shutdown: shutdown.clone(),
            latest_generation,
            latest_folder_generation,
            latest_source_generation,
            scan_cancellation: None,
            automatic_cancellation,
            threads: Vec::new(),
        };

        let sender = event_sender.clone();
        let notify = wake.clone();
        let stop = shutdown.clone();
        let writer_database = database.clone();
        let watch_requests = workers
            .scans
            .as_ref()
            .cloned()
            .expect("Index sender was initialized above");
        let writer = thread::Builder::new()
            .name("sift-index".into())
            .spawn(move || {
                index::run(
                    writer_database,
                    index::Runtime {
                        events: sender,
                        notify,
                        stop,
                        automatic: automatic_token,
                        requests: scan_receiver,
                        ready: ready_sender,
                        watch_requests,
                    },
                )
            })?;
        workers.threads.push(writer);

        let reader = thread::Builder::new()
            .name("sift-search".into())
            .spawn(move || {
                if ready_receiver.recv().is_err() {
                    return;
                }
                let mut reader = match Catalog::open_reader(&database) {
                    Ok(reader) => reader,
                    Err(error) => {
                        deliver(
                            &event_sender,
                            &wake,
                            Event::StartupFailed(error.to_string()),
                        );
                        return;
                    }
                };
                if !deliver(&event_sender, &wake, Event::Ready) {
                    return;
                }
                while let Ok(request) = query_receiver.recv() {
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                    // Discard obsolete queries, but preserve every visible page of
                    // the current generation even when two pages are needed together.
                    let event = match request {
                        ReadRequest::Search(request) => {
                            if request.generation < reader_generation.load(Ordering::Relaxed) {
                                wake();
                                continue;
                            }
                            Event::Page {
                                generation: request.generation,
                                result: reader
                                    .search_scoped(&request.query, &request.scope, request.offset)
                                    .map_err(|error| error.to_string()),
                            }
                        }
                        ReadRequest::Folders {
                            generation,
                            parent,
                            offset,
                        } => {
                            if generation < folder_generation.load(Ordering::Relaxed) {
                                wake();
                                continue;
                            }
                            Event::Folders {
                                generation,
                                result: reader
                                    .folders(&parent, offset)
                                    .map_err(|error| error.to_string()),
                            }
                        }
                        ReadRequest::Source {
                            generation,
                            reference,
                            refresh,
                        } => {
                            if generation < source_generation.load(Ordering::Relaxed) {
                                wake();
                                continue;
                            }
                            let result = (|| -> crate::Result<(Option<Sample>, FileStamp)> {
                                let info = std::fs::symlink_metadata(&reference.source)?;
                                if !info.is_file() || library::is_link(&info) {
                                    return Err(crate::Error::InvalidPath);
                                }
                                if let Some(parent) = reference.source.parent() {
                                    library::ordinary_directory(parent)?;
                                }
                                let source = reference.source.canonicalize()?;
                                Ok((reader.sample(&source)?, FileStamp::read(&info)))
                            })();
                            let (sample, stamp, availability) = match result {
                                Ok((sample, stamp)) => {
                                    let availability = if stamp.size_bytes != reference.size_bytes {
                                        Availability::Changed
                                    } else {
                                        Availability::Ready
                                    };
                                    (sample, Some(stamp), availability)
                                }
                                Err(crate::Error::Io(error))
                                    if error.kind() == std::io::ErrorKind::NotFound =>
                                {
                                    (None, None, Availability::Missing)
                                }
                                Err(error) => {
                                    (None, None, Availability::Unavailable(error.to_string()))
                                }
                            };
                            Event::SourceReady {
                                generation,
                                refresh,
                                sample,
                                stamp,
                                availability,
                            }
                        }
                    };
                    if !deliver(&event_sender, &wake, event) {
                        break;
                    }
                }
            })?;
        workers.threads.push(reader);
        Ok(workers)
    }

    pub fn try_event(&self) -> Option<Event> {
        self.events.as_ref()?.try_recv().ok()
    }

    pub fn query(&self, generation: u64, query: SearchQuery, offset: usize) -> bool {
        self.query_scoped(generation, query, Scope::All, offset)
    }

    pub fn query_scoped(
        &self,
        generation: u64,
        query: SearchQuery,
        scope: Scope,
        offset: usize,
    ) -> bool {
        self.latest_generation
            .fetch_max(generation, Ordering::Relaxed);
        self.queries.as_ref().is_some_and(|sender| {
            sender
                .try_send(ReadRequest::Search(QueryRequest {
                    generation,
                    query,
                    offset,
                    scope,
                }))
                .is_ok()
        })
    }

    pub fn folders(&self, generation: u64, parent: PathBuf, offset: usize) -> bool {
        self.latest_folder_generation
            .fetch_max(generation, Ordering::Relaxed);
        self.queries.as_ref().is_some_and(|sender| {
            sender
                .try_send(ReadRequest::Folders {
                    generation,
                    parent,
                    offset,
                })
                .is_ok()
        })
    }
    pub fn check_source(
        &self,
        generation: u64,
        reference: Reference,
        refresh: SourceRefresh,
    ) -> bool {
        self.latest_source_generation
            .fetch_max(generation, Ordering::Relaxed);
        self.queries.as_ref().is_some_and(|sender| {
            sender
                .try_send(ReadRequest::Source {
                    generation,
                    reference,
                    refresh,
                })
                .is_ok()
        })
    }

    pub fn edit_library(&self, token: u64, edit: LibraryEdit) -> std::result::Result<(), String> {
        self.send_index(IndexRequest::Edit { token, edit })
    }

    pub fn refresh_libraries(&self) -> std::result::Result<(), String> {
        self.send_index(IndexRequest::Refresh)
    }
    pub fn set_live_updates(&self, enabled: bool) -> std::result::Result<(), String> {
        self.send_index(IndexRequest::Live(enabled))?;
        if !enabled {
            self.automatic_cancellation.cancel();
        }
        Ok(())
    }

    fn send_index(&self, request: IndexRequest) -> std::result::Result<(), String> {
        match self
            .scans
            .as_ref()
            .ok_or("The index worker is unavailable")?
            .try_send(request)
        {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err("The index worker is busy; try again".into()),
            Err(TrySendError::Disconnected(_)) => Err("The index worker has stopped".into()),
        }
    }

    pub fn scan(&mut self, folder: PathBuf) -> std::result::Result<(), String> {
        if self.scan_cancellation.is_some() {
            return Err("A scan is already running".into());
        }
        let cancellation = Cancellation::default();
        match self
            .scans
            .as_ref()
            .ok_or("The index worker is unavailable")?
            .try_send(IndexRequest::Scan(folder, cancellation.clone()))
        {
            Ok(()) => {
                self.scan_cancellation = Some(cancellation);
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err("The index worker is busy".into()),
            Err(TrySendError::Disconnected(_)) => Err("The index worker has stopped".into()),
        }
    }

    pub fn cancel_scan(&self) {
        self.automatic_cancellation.cancel();
        if let Some(token) = &self.scan_cancellation {
            token.cancel();
        }
    }
    pub fn finish_scan(&mut self) {
        self.scan_cancellation = None;
    }
}

fn apply_edit(
    store: &mut libraries::Store,
    edit: LibraryEdit,
) -> std::result::Result<EditOutcome, String> {
    let result = (|| -> libraries::Result<EditOutcome> {
        let mut scan_folder = None;
        let selected = match edit {
            LibraryEdit::Create(name) => Some(store.create(&name)?),
            LibraryEdit::Rename(id, name) => {
                store.rename(id, &name)?;
                Some(id)
            }
            LibraryEdit::Remove(id) => {
                store.remove(id)?;
                None
            }
            LibraryEdit::Attach(id, path) => {
                scan_folder = Some(store.attach(id, &path)?);
                Some(id)
            }
            LibraryEdit::Detach(id, source) => {
                store.detach(id, source)?;
                Some(id)
            }
        };
        Ok(EditOutcome {
            selected,
            scan_folder,
        })
    })();
    result.map_err(|error| {
        if matches!(
            error,
            libraries::Error::Database(_) | libraries::Error::Catalog(_) | libraries::Error::Io(_)
        ) {
            eprintln!("Library edit failed: {error}");
        }
        error.to_string()
    })
}

fn deliver(sender: &SyncSender<Event>, wake: &Arc<dyn Fn() + Send + Sync>, event: Event) -> bool {
    if sender.send(event).is_err() {
        return false;
    }
    wake();
    true
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.cancel_scan();
        if let Some(sender) = &self.scans {
            // A full queue already wakes the owner; disconnected means it exited.
            match sender.try_send(IndexRequest::Wake) {
                Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
            }
        }
        // Drop consumers before joining: a worker blocked by a full event queue
        // must be released even if the UI never reads another event.
        drop(self.events.take());
        drop(self.queries.take());
        drop(self.scans.take());
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                eprintln!("Sift worker panicked during shutdown");
            }
        }
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/unit/worker.rs"));
}
