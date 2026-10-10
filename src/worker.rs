use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::catalog::{Catalog, FolderPage, Scope, SearchPage};
use crate::libraries::{self, Library, LibraryId, SourceId};
use crate::library::{self, Cancellation, ScanReport};
use crate::search::SearchQuery;

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
}

pub struct Workers {
    events: Option<Receiver<Event>>,
    queries: Option<SyncSender<ReadRequest>>,
    scans: Option<SyncSender<IndexRequest>>,
    shutdown: Arc<AtomicBool>,
    latest_generation: Arc<AtomicU64>,
    latest_folder_generation: Arc<AtomicU64>,
    scan_cancellation: Option<Cancellation>,
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
        let latest_folder_generation = Arc::new(AtomicU64::new(0));
        let folder_generation = latest_folder_generation.clone();
        let mut workers = Self {
            events: Some(events),
            queries: Some(query_sender),
            scans: Some(scan_sender),
            shutdown: shutdown.clone(),
            latest_generation,
            latest_folder_generation,
            scan_cancellation: None,
            threads: Vec::new(),
        };

        let sender = event_sender.clone();
        let notify = wake.clone();
        let stop = shutdown.clone();
        let writer_database = database.clone();
        let writer = thread::Builder::new()
            .name("sift-index".into())
            .spawn(move || {
                let mut writer = match Catalog::open(&writer_database) {
                    Ok(writer) => writer,
                    Err(error) => {
                        deliver(&sender, &notify, Event::StartupFailed(error.to_string()));
                        return;
                    }
                };
                let mut registry = match libraries::Store::open(&libraries::data_path(
                    &writer_database,
                ))
                .and_then(|mut store| {
                    store.import_legacy(&writer.roots()?)?;
                    Ok(store)
                }) {
                    Ok(store) => store,
                    Err(error) => {
                        deliver(&sender, &notify, Event::StartupFailed(error.to_string()));
                        return;
                    }
                };
                let snapshot = registry
                    .snapshot(&writer)
                    .map_err(|error| error.to_string());
                // Closing during startup must still drain already accepted edits.
                deliver(&sender, &notify, Event::Libraries(snapshot));
                if ready_sender.send(()).is_err() {
                    return;
                }
                while let Ok(request) = scan_receiver.recv() {
                    let (folder, cancellation) = match request {
                        IndexRequest::Edit { token, edit } => {
                            // Accepted edits drain even after the observer closes.
                            let result = apply_edit(&mut registry, edit);
                            let observed =
                                deliver(&sender, &notify, Event::LibraryEdited { token, result });
                            if observed && !stop.load(Ordering::Relaxed) {
                                deliver(
                                    &sender,
                                    &notify,
                                    Event::Libraries(
                                        registry
                                            .snapshot(&writer)
                                            .map_err(|error| error.to_string()),
                                    ),
                                );
                            }
                            continue;
                        }
                        IndexRequest::Refresh => {
                            if !stop.load(Ordering::Relaxed) {
                                deliver(
                                    &sender,
                                    &notify,
                                    Event::Libraries(
                                        registry
                                            .snapshot(&writer)
                                            .map_err(|error| error.to_string()),
                                    ),
                                );
                            }
                            continue;
                        }
                        IndexRequest::Scan(folder, cancellation) => (folder, cancellation),
                    };
                    if stop.load(Ordering::Relaxed) {
                        continue;
                    }
                    let mut last_progress = Instant::now();
                    let result = library::scan(&folder, &mut writer, &cancellation, |report| {
                        if last_progress.elapsed() >= Duration::from_millis(100) {
                            // Progress may be coalesced; completion and errors must not be lost.
                            if sender.try_send(Event::Progress(report.clone())).is_ok() {
                                notify();
                            }
                            last_progress = Instant::now();
                        }
                    })
                    .map_err(|error| error.to_string());
                    if !deliver(&sender, &notify, Event::ScanFinished(result)) {
                        continue;
                    }
                    deliver(
                        &sender,
                        &notify,
                        Event::Libraries(
                            registry
                                .snapshot(&writer)
                                .map_err(|error| error.to_string()),
                        ),
                    );
                }
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

    pub fn edit_library(&self, token: u64, edit: LibraryEdit) -> std::result::Result<(), String> {
        self.send_index(IndexRequest::Edit { token, edit })
    }

    pub fn refresh_libraries(&self) -> std::result::Result<(), String> {
        self.send_index(IndexRequest::Refresh)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::IndexedFile;
    use crate::library::AudioMetadata;

    fn receive(workers: &Workers) -> Event {
        loop {
            let event = workers
                .events
                .as_ref()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            if !matches!(event, Event::Libraries(_)) {
                return event;
            }
        }
    }

    #[test]
    fn two_visible_pages_of_one_generation_both_complete() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("catalog.sqlite");
        let mut catalog = Catalog::open(&database).unwrap();
        let root = catalog
            .register_root(&temporary.path().join("synthetic"))
            .unwrap();
        let files: Vec<_> = (0..300)
            .map(|index| {
                let name = format!("sound_{index:03}.wav");
                IndexedFile {
                    source: temporary.path().join(&name),
                    display_path: name.clone(),
                    name,
                    format: "wav".into(),
                    size_bytes: 44,
                    audio: AudioMetadata::default(),
                    metadata_error: None,
                    stamp: None,
                }
            })
            .collect();
        catalog.upsert_batch(root, &files).unwrap();
        let workers = Workers::start(database, || {}).unwrap();
        assert!(matches!(receive(&workers), Event::Ready));
        assert!(workers.query(1, SearchQuery::All, 0));
        assert!(workers.query(1, SearchQuery::All, 128));
        let mut offsets = BTreeSet::new();
        for _ in 0..2 {
            let Event::Page { generation, result } = receive(&workers) else {
                panic!("Expected page")
            };
            assert_eq!(generation, 1);
            offsets.insert(result.unwrap().offset);
        }
        assert_eq!(offsets, BTreeSet::from([0, 128]));
    }

    #[test]
    fn startup_failure_is_reported_and_threads_can_shut_down() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("catalog.sqlite");
        rusqlite::Connection::open(&database)
            .unwrap()
            .pragma_update(None, "user_version", 99)
            .unwrap();
        let workers = Workers::start(database, || {}).unwrap();
        assert!(matches!(receive(&workers), Event::StartupFailed(_)));
        drop(workers);
    }

    #[test]
    fn shutdown_releases_a_worker_with_a_full_event_queue() {
        let temporary = tempfile::tempdir().unwrap();
        let (wake_sender, wake_receiver) = mpsc::channel();
        let workers = Workers::start(temporary.path().join("catalog.sqlite"), move || {
            wake_sender.send(()).unwrap();
        })
        .unwrap();
        assert!(matches!(receive(&workers), Event::Ready));
        wake_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        let query_sender = workers.queries.as_ref().unwrap().clone();
        let producer = thread::spawn(move || {
            for offset in 0..16 {
                if query_sender
                    .send(ReadRequest::Search(QueryRequest {
                        generation: 1,
                        query: SearchQuery::All,
                        offset,
                        scope: Scope::All,
                    }))
                    .is_err()
                {
                    break;
                }
            }
        });
        for _ in 0..8 {
            wake_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        drop(workers);
        producer.join().unwrap();
    }

    use std::collections::BTreeSet;

    #[test]
    fn shutdown_commits_accepted_library_edits_without_reading_their_results() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("catalog.sqlite");
        let workers = Workers::start(path.clone(), || {}).unwrap();
        workers
            .edit_library(1, LibraryEdit::Create("Accepted bank".into()))
            .unwrap();
        drop(workers);
        let catalog = Catalog::open(&path).unwrap();
        let store = libraries::Store::open(&libraries::data_path(&path)).unwrap();
        assert_eq!(store.snapshot(&catalog).unwrap()[0].name, "Accepted bank");
    }

    #[test]
    fn stale_requests_cannot_displace_new_scoped_results_or_folder_pages() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("catalog.sqlite");
        let root = temporary.path().join("sounds");
        let child = root.join("selected");
        let mut catalog = Catalog::open(&path).unwrap();
        let id = catalog.register_root(&root).unwrap();
        let files: Vec<_> = [
            child.join("nested").join("keep.wav"),
            root.join("other").join("exclude.wav"),
        ]
        .into_iter()
        .map(|source| IndexedFile {
            name: source.file_name().unwrap().to_string_lossy().into_owned(),
            source,
            display_path: "fixture".into(),
            format: "wav".into(),
            size_bytes: 44,
            audio: AudioMetadata::default(),
            metadata_error: None,
            stamp: None,
        })
        .collect();
        catalog.upsert_batch(id, &files).unwrap();
        let workers = Workers::start(path, || {}).unwrap();
        assert!(matches!(receive(&workers), Event::Ready));
        workers.latest_generation.store(2, Ordering::Relaxed);
        workers.latest_folder_generation.store(2, Ordering::Relaxed);
        assert!(workers.query(1, SearchQuery::All, 0));
        assert!(workers.folders(1, root, 0));
        assert!(workers.query_scoped(2, SearchQuery::All, Scope::Folders(vec![child.clone()]), 0));
        assert!(workers.folders(2, child, 0));
        match receive(&workers) {
            Event::Page {
                generation: 2,
                result: Ok(page),
            } => {
                assert_eq!(page.total, 1);
                assert_eq!(page.rows[0].name, "keep.wav");
            }
            event => panic!("Unexpected current result: {event:?}"),
        }
        match receive(&workers) {
            Event::Folders {
                generation: 2,
                result: Ok(page),
            } => {
                assert_eq!(page.total, 1);
                assert_eq!(page.rows[0].name, "nested");
            }
            event => panic!("Unexpected current folders: {event:?}"),
        }
    }
}
