use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::catalog::{Catalog, SearchPage};
use crate::library::{self, Cancellation, ScanReport};
use crate::search::SearchQuery;

#[derive(Debug)]
pub enum Event {
    Ready,
    Page {
        generation: u64,
        result: std::result::Result<SearchPage, String>,
    },
    Progress(ScanReport),
    ScanFinished(std::result::Result<ScanReport, String>),
    StartupFailed(String),
}

struct QueryRequest {
    generation: u64,
    query: SearchQuery,
    offset: usize,
}

pub struct Workers {
    events: Option<Receiver<Event>>,
    queries: Option<SyncSender<QueryRequest>>,
    scans: Option<SyncSender<(PathBuf, Cancellation)>>,
    shutdown: Arc<AtomicBool>,
    latest_generation: Arc<AtomicU64>,
    scan_cancellation: Option<Cancellation>,
    threads: Vec<JoinHandle<()>>,
}

impl Workers {
    pub fn start(
        database: PathBuf,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let (event_sender, events) = mpsc::sync_channel(8);
        let (query_sender, query_receiver) = mpsc::sync_channel::<QueryRequest>(4);
        let (scan_sender, scan_receiver) = mpsc::sync_channel::<(PathBuf, Cancellation)>(1);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let shutdown = Arc::new(AtomicBool::new(false));
        let latest_generation = Arc::new(AtomicU64::new(0));
        let reader_generation = latest_generation.clone();
        let mut workers = Self {
            events: Some(events),
            queries: Some(query_sender),
            scans: Some(scan_sender),
            shutdown: shutdown.clone(),
            latest_generation,
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
                if ready_sender.send(()).is_err() {
                    return;
                }
                while let Ok((folder, cancellation)) = scan_receiver.recv() {
                    if stop.load(Ordering::Relaxed) {
                        break;
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
                        break;
                    }
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
                    if request.generation < reader_generation.load(Ordering::Relaxed) {
                        wake();
                        continue;
                    }
                    let result = reader
                        .search(&request.query, request.offset)
                        .map_err(|error| error.to_string());
                    if !deliver(
                        &event_sender,
                        &wake,
                        Event::Page {
                            generation: request.generation,
                            result,
                        },
                    ) {
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
        self.latest_generation
            .fetch_max(generation, Ordering::Relaxed);
        self.queries.as_ref().is_some_and(|sender| {
            sender
                .try_send(QueryRequest {
                    generation,
                    query,
                    offset,
                })
                .is_ok()
        })
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
            .try_send((folder, cancellation.clone()))
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
        workers
            .events
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
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
                    .send(QueryRequest {
                        generation: 1,
                        query: SearchQuery::All,
                        offset,
                    })
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
}
