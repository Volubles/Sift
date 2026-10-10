//! Native notifications are hints, coalesced in a bounded pending set. The
//! index owner inspects filesystem state; the notification thread performs no I/O.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MAX_PATHS: usize = 1024;
const MAX_PATH_BYTES: usize = 1024 * 1024;
const QUIET: Duration = Duration::from_millis(200);
const MAX_BURST: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct Batch {
    pub paths: Vec<(PathBuf, PathBuf)>,
    pub roots: Vec<PathBuf>,
    pub forced_roots: Vec<PathBuf>,
    pub status_changed: bool,
    pub errors: Vec<String>,
}

#[derive(Default)]
struct Pending {
    paths: BTreeMap<PathBuf, PathBuf>,
    roots: BTreeSet<PathBuf>,
    forced_roots: BTreeSet<PathBuf>,
    bytes: usize,
    first: Option<Instant>,
    last: Option<Instant>,
    status_changed: bool,
    errors: Vec<String>,
}

impl Pending {
    fn touch(&mut self, now: Instant) {
        self.first.get_or_insert(now);
        self.last = Some(now);
    }
    fn root(&mut self, root: PathBuf, now: Instant) {
        self.paths.retain(|path, _| !path.starts_with(&root));
        self.bytes = self.paths.keys().map(|path| path.as_os_str().len()).sum();
        self.roots.insert(root);
        self.touch(now);
    }
    fn path(&mut self, root: PathBuf, path: PathBuf, now: Instant) {
        if self.roots.contains(&root) {
            self.forced_roots.insert(root);
            self.touch(now);
            return;
        }
        if !self.paths.contains_key(&path) {
            let bytes = path.as_os_str().len();
            if self.paths.len() == MAX_PATHS || self.bytes + bytes > MAX_PATH_BYTES {
                self.forced_roots.insert(root.clone());
                self.root(root, now);
                return;
            }
            self.bytes += bytes;
            self.paths.insert(path, root);
        }
        self.touch(now);
    }
    fn deadline(&self) -> Option<Instant> {
        Some((self.last? + QUIET).min(self.first? + MAX_BURST))
    }
    fn take(&mut self, now: Instant) -> Option<Batch> {
        if self.deadline().is_none_or(|deadline| deadline > now) {
            return None;
        }
        let pending = std::mem::take(self);
        Some(Batch {
            paths: pending
                .paths
                .into_iter()
                .map(|(path, root)| (root, path))
                .collect(),
            roots: pending.roots.into_iter().collect(),
            forced_roots: pending.forced_roots.into_iter().collect(),
            status_changed: pending.status_changed,
            errors: pending.errors,
        })
    }
}

pub fn coalesce_roots(mut roots: Vec<PathBuf>) -> Vec<PathBuf> {
    roots.sort();
    roots.dedup();
    roots
        .iter()
        .filter(|root| {
            !roots
                .iter()
                .any(|other| other != *root && root.starts_with(other))
        })
        .cloned()
        .collect()
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod native;

#[cfg(windows)]
mod windows {
    use super::native::{StopEvent, Wake};
    use super::*;
    use std::num::NonZeroUsize;
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use windows_file_watcher::{
        Monitor, Notification, Outcome, RetryMode, Session, VolumeChangeDecision,
        VolumeChangePolicy, Watch, WatchId, WatchOptions,
    };

    #[derive(Default)]
    struct Shared {
        routes: BTreeMap<WatchId, PathBuf>,
        pending: Pending,
        blocked: BTreeSet<PathBuf>,
        retries: BTreeMap<WatchId, u32>,
        established: BTreeSet<WatchId>,
    }

    fn handle_notification(
        state: &mut Shared,
        worker_session: &Session,
        notification: Notification,
        id: WatchId,
        root: PathBuf,
    ) {
        let now = Instant::now();
        match notification {
            Notification::Batch { changes, .. } if !state.blocked.contains(&root) => {
                for change in changes {
                    let relative = change.name.to_path_buf();
                    if relative
                        .components()
                        .any(|part| !matches!(part, std::path::Component::Normal(_)))
                    {
                        state.pending.forced_roots.insert(root.clone());
                        state.pending.root(root.clone(), now);
                        continue;
                    }
                    state.pending.path(root.clone(), root.join(relative), now);
                }
            }
            Notification::Desync { cause, .. } => {
                if cause.is_terminal() {
                    state.blocked.insert(root.clone());
                    state.pending.status_changed = true;
                    state.pending.touch(now);
                    if state.pending.errors.len() < 16 {
                        state.pending.errors.push(format!(
                            "File watching stopped for source {}. Refresh its registration.",
                            id.get()
                        ));
                    }
                } else if !state.blocked.contains(&root) {
                    state.pending.forced_roots.insert(root.clone());
                    state.pending.root(root.clone(), now);
                }
            }
            Notification::RetryQuestion { .. } => {
                let attempts = state.retries.entry(id).or_default();
                *attempts = attempts.saturating_add(1);
                worker_session.answer(
                    id,
                    Some(Duration::from_secs(
                        (2u64.saturating_pow((*attempts).min(5))).min(30),
                    )),
                );
                state.pending.status_changed = true;
                state.pending.touch(now);
            }
            Notification::VolumeChanged { .. } => {
                worker_session.answer_volume_change(id, VolumeChangeDecision::Stop);
                state.blocked.insert(root.clone());
                if state.pending.errors.len() < 16 {
                    state.pending.errors.push(format!("The volume changed for source {}. Cached records were retained; reconnect the source explicitly.",id.get()));
                }
                state.pending.status_changed = true;
                state.pending.touch(now);
            }
            Notification::Established { .. } => {
                if !state.established.insert(id) {
                    state.pending.forced_roots.insert(root.clone());
                }
                state.retries.remove(&id);
                state.pending.root(root.clone(), now);
                state.pending.status_changed = true;
            }
            Notification::Resumed { .. } => {
                state.pending.forced_roots.insert(root.clone());
                state.retries.remove(&id);
                state.pending.root(root.clone(), now);
                state.pending.status_changed = true;
            }
            Notification::Suspended { .. } => {
                state.pending.status_changed = true;
                state.pending.touch(now);
            }
            Notification::Completion {
                outcome: Outcome::Failed { detail },
                ..
            } => {
                if state.pending.errors.len() < 16 {
                    state.pending.errors.push(format!(
                        "File watch registration failed for source {}: {detail:?}",
                        id.get()
                    ));
                }
                state.pending.status_changed = true;
                state.pending.touch(now);
            }
            _ => {}
        }
    }

    pub struct Engine {
        monitor: Monitor,
        session: Session,
        watches: BTreeMap<PathBuf, Watch>,
        shared: Arc<Mutex<Shared>>,
        stop: StopEvent,
        thread: Option<JoinHandle<()>>,
    }

    impl Engine {
        pub fn new(wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
            let monitor = Monitor::new()?;
            // One standing fault/volume reservation per source, plus 64 batches.
            let (session, receiver) = monitor.session_with_bound(
                NonZeroUsize::new(576).expect("The constant queue bound is nonzero"),
            );
            let queue_event = receiver.doorbell_owned()?;
            let shared = Arc::new(Mutex::new(Shared::default()));
            let worker_state = shared.clone();
            let worker_session = session.clone();
            let stop = StopEvent::new()?;
            let worker_stop = stop.clone();
            let thread = thread::Builder::new()
                .name("sift-file-events".into())
                .spawn(move || {
                    'events: loop {
                        match worker_stop.wait(&queue_event) {
                            Ok(Wake::Stop) => break,
                            Ok(Wake::Changes) => {}
                            Err(error) => {
                                match worker_state.lock() {
                                    Ok(mut state) => {
                                        state.pending.errors.push(format!("File event wait failed: {error}"));
                                        state.pending.touch(Instant::now());
                                    }
                                    Err(_) => eprintln!("File event wait failed and event state was poisoned: {error}"),
                                }
                                wake();
                                break;
                            }
                        }
                        while let Some(notification) = receiver.try_recv() {
                            if worker_stop.is_requested() { break 'events; }
                            let mut state = match worker_state.lock() {
                                Ok(state) => state,
                                Err(_) => {
                                    eprintln!("File event state was poisoned; watcher stopped");
                                    wake();
                                    break 'events;
                                }
                            };
                            let id = notification.watch();
                            let Some(root) = state.routes.get(&id).cloned() else { continue; };
                            handle_notification(&mut state, &worker_session, notification, id, root);
                            drop(state);
                            wake();
                        }
                    }
                })?;
            Ok(Self {
                monitor,
                session,
                watches: BTreeMap::new(),
                shared,
                stop,
                thread: Some(thread),
            })
        }
        pub fn set_roots(&mut self, roots: Vec<PathBuf>) -> std::io::Result<()> {
            if roots.len() > 512 {
                return Err(std::io::Error::other("Too many registered source folders"));
            }
            let roots = coalesce_roots(roots);
            let mut state = self
                .shared
                .lock()
                .map_err(|_| std::io::Error::other("File event state is unavailable"))?;
            self.watches.retain(|root, watch| {
                if roots.contains(root) {
                    true
                } else {
                    state.routes.remove(&watch.id());
                    state.blocked.remove(root);
                    false
                }
            });
            let ids: BTreeSet<_> = state.routes.keys().copied().collect();
            state.retries.retain(|id, _| ids.contains(id));
            state.established.retain(|id| ids.contains(id));
            state.pending.paths.retain(|_, root| roots.contains(root));
            state.pending.roots.retain(|root| roots.contains(root));
            state
                .pending
                .forced_roots
                .retain(|root| roots.contains(root));
            state.pending.bytes = state
                .pending
                .paths
                .keys()
                .map(|path| path.as_os_str().len())
                .sum();
            for root in roots {
                if !self.watches.contains_key(&root) {
                    let watch = self.session.subscribe(
                        &root,
                        WatchOptions::new()
                            .subtree(true)
                            .report_liveness(true)
                            .retry(RetryMode::Interactive)
                            .on_volume_change(VolumeChangePolicy::Confirm),
                    )?;
                    state.routes.insert(watch.id(), root.clone());
                    self.watches.insert(root, watch);
                }
            }
            drop(state);
            self.monitor.quiesce();
            Ok(())
        }
        pub fn deadline(&self) -> std::io::Result<Option<Instant>> {
            Ok(self
                .shared
                .lock()
                .map_err(|_| std::io::Error::other("File event state is unavailable"))?
                .pending
                .deadline())
        }
        pub fn take_due(&mut self) -> std::io::Result<Option<Batch>> {
            let mut state = self
                .shared
                .lock()
                .map_err(|_| std::io::Error::other("File event state is unavailable"))?;
            let Some(mut batch) = state.pending.take(Instant::now()) else {
                return Ok(None);
            };
            batch.roots.retain(|root| !state.blocked.contains(root));
            batch
                .forced_roots
                .retain(|root| !state.blocked.contains(root));
            batch
                .paths
                .retain(|(root, _)| !state.blocked.contains(root));
            Ok(Some(batch))
        }
        pub fn retry_path(&self, root: PathBuf, path: PathBuf) {
            match self.shared.lock() {
                Ok(mut state) => state.pending.path(root, path, Instant::now()),
                Err(_) => eprintln!("Could not retry a file change: event state is unavailable"),
            }
        }
    }
    impl Drop for Engine {
        fn drop(&mut self) {
            if let Err(error) = self.stop.signal() {
                eprintln!("File event shutdown signal failed: {error}");
            }
            if let Some(thread) = self.thread.take()
                && thread.join().is_err()
            {
                eprintln!("File event worker panicked during shutdown");
            }
            self.watches.clear();
            self.monitor.shut_down();
        }
    }
}

#[cfg(windows)]
pub use windows::Engine;

#[cfg(not(windows))]
pub enum Engine {}
#[cfg(not(windows))]
impl Engine {
    pub fn new(_wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        Err(std::io::Error::other(
            "Live file watching is currently supported on Windows",
        ))
    }
    pub fn set_roots(&mut self, _roots: Vec<PathBuf>) -> std::io::Result<()> {
        match *self {}
    }
    pub fn deadline(&self) -> std::io::Result<Option<Instant>> {
        match *self {}
    }
    pub fn take_due(&mut self) -> std::io::Result<Option<Batch>> {
        match *self {}
    }
    pub fn retry_path(&self, _root: PathBuf, _path: PathBuf) {
        match *self {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changes_coalesce_with_quiet_and_maximum_burst_deadlines() {
        let now = Instant::now();
        let root = PathBuf::from("root");
        let path = root.join("file.wav");
        let mut pending = Pending::default();
        pending.path(root.clone(), path.clone(), now);
        pending.path(root.clone(), path.clone(), now + Duration::from_millis(150));
        assert!(pending.take(now + Duration::from_millis(200)).is_none());
        assert_eq!(
            pending
                .take(now + Duration::from_millis(351))
                .unwrap()
                .paths,
            vec![(root.clone(), path.clone())]
        );
        pending.path(root.clone(), path.clone(), now);
        for offset in [100, 300, 600, 950] {
            pending.path(
                root.clone(),
                path.clone(),
                now + Duration::from_millis(offset),
            );
        }
        assert_eq!(pending.deadline(), Some(now + MAX_BURST));
    }
    #[test]
    fn overflow_promotes_a_root_to_one_bounded_reconciliation() {
        let now = Instant::now();
        let root = PathBuf::from("root");
        let mut pending = Pending::default();
        for index in 0..5000 {
            pending.path(root.clone(), root.join(format!("file{index}.wav")), now);
        }
        assert!(pending.paths.is_empty());
        assert_eq!(pending.roots.len(), 1);
        assert_eq!(pending.bytes, 0);
        assert_eq!(pending.take(now + QUIET).unwrap().roots, vec![root]);
    }
    #[test]
    fn overlapping_roots_are_watched_once_without_matching_sibling_names() {
        let root = PathBuf::from("root");
        assert_eq!(
            coalesce_roots(vec![
                root.join("a"),
                root.clone(),
                root.clone(),
                PathBuf::from("root-other")
            ]),
            vec![root, PathBuf::from("root-other")]
        );
    }
}
