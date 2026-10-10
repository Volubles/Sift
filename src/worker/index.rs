use super::*;
use std::sync::mpsc::{RecvTimeoutError, TryRecvError};

pub(super) struct Runtime {
    pub events: SyncSender<Event>,
    pub notify: Arc<dyn Fn() + Send + Sync>,
    pub stop: Arc<AtomicBool>,
    pub automatic: Cancellation,
    pub requests: Receiver<IndexRequest>,
    pub ready: SyncSender<()>,
    pub watch_requests: SyncSender<IndexRequest>,
}

impl Runtime {
    fn send(&self, event: Event) -> bool {
        deliver(&self.events, &self.notify, event)
    }
    fn configure(&self, engine: &mut Option<crate::watch::Engine>, registry: &libraries::Store) {
        let result = (|| -> std::result::Result<(), String> {
            if engine.is_none() {
                let sender = self.watch_requests.clone();
                let stop = self.stop.clone();
                *engine = Some(
                    crate::watch::Engine::new(move || {
                        if !stop.load(Ordering::Relaxed) {
                            // Coalesced wake only; pending filesystem data remains in
                            // the watcher's bounded set even if commands fill the queue.
                            match sender.try_send(IndexRequest::Wake) {
                                Ok(()) | Err(TrySendError::Full(_)) => {}
                                Err(TrySendError::Disconnected(_)) => {}
                            }
                        }
                    })
                    .map_err(|error| error.to_string())?,
                );
            }
            let roots = registry.source_paths().map_err(|error| error.to_string())?;
            engine
                .as_mut()
                .expect("Engine was initialized above")
                .set_roots(roots)
                .map_err(|error| error.to_string())
        })();
        match result {
            Ok(()) => {
                self.send(Event::WatchStatus {
                    enabled: true,
                    message: "Live updates active".into(),
                });
            }
            Err(error) => {
                *engine = None;
                self.send(Event::WatchStatus {
                    enabled: false,
                    message: format!(
                        "Live updates unavailable: {error}. Use Rescan source folder."
                    ),
                });
            }
        }
    }
    fn snapshot(&self, registry: &libraries::Store, catalog: &Catalog) {
        self.send(Event::Libraries(
            registry
                .snapshot(catalog)
                .map_err(|error| error.to_string()),
        ));
    }
    fn progress(&self, report: &ScanReport) {
        if self
            .events
            .try_send(Event::Progress(report.clone()))
            .is_ok()
        {
            (self.notify)();
        }
    }
}

pub(super) fn run(database: PathBuf, runtime: Runtime) {
    let mut catalog = match Catalog::open(&database) {
        Ok(catalog) => catalog,
        Err(error) => {
            runtime.send(Event::StartupFailed(error.to_string()));
            return;
        }
    };
    let mut registry =
        match libraries::Store::open(&libraries::data_path(&database)).and_then(|mut store| {
            store.import_legacy(&catalog.roots()?)?;
            Ok(store)
        }) {
            Ok(store) => store,
            Err(error) => {
                runtime.send(Event::StartupFailed(error.to_string()));
                return;
            }
        };
    runtime.snapshot(&registry, &catalog);
    let mut engine = None;
    let mut live = cfg!(windows);
    if live && !runtime.stop.load(Ordering::Relaxed) {
        runtime.configure(&mut engine, &registry);
    }
    if runtime.ready.send(()).is_err() {
        return;
    }
    let mut next_request = None;
    loop {
        let request = if let Some(request) = next_request.take() {
            Some(request)
        } else if runtime.stop.load(Ordering::Relaxed) {
            match runtime.requests.try_recv() {
                Ok(request) => Some(request),
                Err(_) => break,
            }
        } else {
            let deadline = match engine.as_ref().map(|engine| engine.deadline()).transpose() {
                Ok(deadline) => deadline.flatten(),
                Err(error) => {
                    runtime.send(Event::WatchStatus {
                        enabled: false,
                        message: format!("File event state failed: {error}"),
                    });
                    engine = None;
                    continue;
                }
            };
            match deadline {
                Some(deadline) => match runtime
                    .requests
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                {
                    Ok(request) => Some(request),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match runtime.requests.recv() {
                    Ok(request) => Some(request),
                    Err(_) => break,
                },
            }
        };
        if let Some(request) = request {
            match request {
                IndexRequest::Edit { token, edit } => {
                    let result = apply_edit(&mut registry, edit);
                    let observed = runtime.send(Event::LibraryEdited { token, result });
                    if observed && !runtime.stop.load(Ordering::Relaxed) {
                        if live {
                            runtime.configure(&mut engine, &registry);
                        }
                        runtime.snapshot(&registry, &catalog);
                    }
                }
                IndexRequest::Refresh => {
                    if !runtime.stop.load(Ordering::Relaxed) {
                        if live {
                            runtime.configure(&mut engine, &registry);
                        }
                        runtime.snapshot(&registry, &catalog);
                    }
                }
                IndexRequest::Live(enabled) => {
                    if runtime.stop.load(Ordering::Relaxed) {
                        continue;
                    }
                    live = enabled;
                    if enabled {
                        runtime.automatic.reset();
                        runtime.configure(&mut engine, &registry);
                    } else {
                        engine = None;
                        runtime.send(Event::WatchStatus {
                            enabled: false,
                            message: "Live updates paused".into(),
                        });
                    }
                }
                IndexRequest::Scan(folder, cancellation) => {
                    if runtime.stop.load(Ordering::Relaxed) {
                        continue;
                    }
                    let mut last = Instant::now();
                    let result = library::scan(&folder, &mut catalog, &cancellation, |report| {
                        if last.elapsed() >= Duration::from_millis(100) {
                            runtime.progress(report);
                            last = Instant::now();
                        }
                    })
                    .map_err(|error| error.to_string());
                    runtime.send(Event::ScanFinished(result));
                    runtime.snapshot(&registry, &catalog);
                    if cancellation.is_cancelled() {
                        engine = None;
                        live = false;
                        runtime.send(Event::WatchStatus {
                            enabled: false,
                            message: "Live updates paused after cancellation".into(),
                        });
                    }
                }
                IndexRequest::Wake => {}
            }
        }
        if runtime.stop.load(Ordering::Relaxed) {
            continue;
        }
        // Drain ordinary commands before starting another background batch.
        match runtime.requests.try_recv() {
            Ok(IndexRequest::Wake) => {}
            Ok(request) => {
                next_request = Some(request);
                continue;
            }
            Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }
        let batch = match engine.as_mut().map(|engine| engine.take_due()).transpose() {
            Ok(batch) => batch.flatten(),
            Err(error) => {
                engine = None;
                runtime.send(Event::WatchStatus {
                    enabled: false,
                    message: format!("Live update failed: {error}"),
                });
                None
            }
        };
        if let Some(batch) = batch {
            for error in &batch.errors {
                eprintln!("Sift file watching failed: {error}");
                runtime.send(Event::WatchStatus {
                    enabled: live,
                    message: error.clone(),
                });
            }
            let mut report = ScanReport::default();
            let has_work = !batch.roots.is_empty() || !batch.paths.is_empty();
            if has_work {
                runtime.send(Event::SyncStarted);
            }
            for root in &batch.roots {
                if runtime.automatic.is_cancelled() {
                    break;
                }
                let mut last = Instant::now();
                let mut progress = |part: &ScanReport| {
                    if last.elapsed() >= Duration::from_millis(100) {
                        runtime.progress(part);
                        last = Instant::now();
                    }
                };
                let result = if batch.forced_roots.contains(root) {
                    library::scan_after_event_loss(
                        root,
                        &mut catalog,
                        &runtime.automatic,
                        &mut progress,
                    )
                } else {
                    library::scan(root, &mut catalog, &runtime.automatic, &mut progress)
                };
                accumulate(&mut report, result);
            }
            for (root, path) in &batch.paths {
                if runtime.automatic.is_cancelled() {
                    break;
                }
                let result =
                    library::synchronize_path(root, path, &mut catalog, &runtime.automatic);
                if result.as_ref().is_ok_and(|part| part.deferred > 0)
                    && let Some(engine) = &engine
                {
                    engine.retry_path(root.clone(), path.clone());
                }
                accumulate(&mut report, result);
            }
            report.cancelled = runtime.automatic.is_cancelled();
            if has_work {
                runtime.send(Event::SyncFinished {
                    report,
                    touched: batch.paths.into_iter().map(|(_, path)| path).collect(),
                    full_roots: batch.roots,
                    forced_roots: batch.forced_roots,
                });
            }
            if has_work || batch.status_changed {
                runtime.snapshot(&registry, &catalog);
            }
            if runtime.automatic.is_cancelled() {
                engine = None;
                live = false;
                runtime.send(Event::WatchStatus {
                    enabled: false,
                    message: "Live updates paused after cancellation".into(),
                });
            }
        }
    }
}

fn accumulate(report: &mut ScanReport, result: crate::Result<ScanReport>) {
    match result {
        Ok(part) => {
            report.visited += part.visited;
            report.indexed += part.indexed;
            report.unchanged += part.unchanged;
            report.removed += part.removed;
            report.deferred += part.deferred;
            report.errors += part.errors;
            report.skipped_links += part.skipped_links;
            report.incomplete |= part.incomplete;
            report
                .issues
                .extend(part.issues.into_iter().take(16 - report.issues.len()));
        }
        Err(error) => {
            report.incomplete = true;
            if !matches!(&error,crate::Error::Io(error) if error.kind()==std::io::ErrorKind::NotFound)
            {
                report.errors += 1;
                eprintln!("Sift source synchronization failed; cached records retained: {error}");
            }
        }
    }
}
