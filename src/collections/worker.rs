use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
use std::thread::{self, JoinHandle};

use super::favorites::Favorite;
use crate::search::SearchQuery;

use super::{Collection, CollectionId, Entry, EntryId, Page, Reference, Store};

#[derive(Clone, Debug)]
pub enum Mutation {
    Favorite(Reference),
    Unfavorite(PathBuf),
    Create(String),
    Rename {
        collection: CollectionId,
        name: String,
    },
    Delete(CollectionId),
    Add {
        collection: CollectionId,
        references: Vec<Reference>,
    },
    Remove {
        collection: CollectionId,
        entry: EntryId,
    },
}

#[derive(Debug)]
pub enum Change {
    FavoriteChanged(PathBuf),
    Created(CollectionId),
    Updated(CollectionId),
    Deleted(CollectionId),
}

#[derive(Debug)]
pub enum Event {
    Ready,
    StartupFailed(String),
    Favorites {
        generation: u64,
        result: Result<Page<Favorite>, String>,
    },
    FavoriteStatus {
        generation: u64,
        result: Result<Vec<(PathBuf, bool)>, String>,
    },
    Groups {
        generation: u64,
        result: Result<Page<Collection>, String>,
    },
    Entries {
        generation: u64,
        collection: CollectionId,
        result: Result<Page<Entry>, String>,
    },
    Saved {
        operation: u64,
        result: Result<Change, String>,
    },
}

enum Command {
    Favorites {
        generation: u64,
        query: SearchQuery,
        offset: usize,
    },
    FavoriteStatus {
        generation: u64,
        paths: Vec<PathBuf>,
    },
    Groups {
        generation: u64,
        offset: usize,
    },
    Entries {
        generation: u64,
        collection: CollectionId,
        offset: usize,
    },
    Mutate {
        operation: u64,
        mutation: Mutation,
    },
}

pub struct Worker {
    commands: Option<SyncSender<Command>>,
    events: Option<Receiver<Event>>,
    favorites_generation: Arc<AtomicU64>,
    status_generation: Arc<AtomicU64>,
    groups_generation: Arc<AtomicU64>,
    entries_generation: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(path: PathBuf, wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (commands, jobs) = mpsc::sync_channel(4);
        let (sender, events) = mpsc::sync_channel(4);
        let favorites_generation = Arc::new(AtomicU64::new(0));
        let status_generation = Arc::new(AtomicU64::new(0));
        let favorites = favorites_generation.clone();
        let status = status_generation.clone();
        let groups_generation = Arc::new(AtomicU64::new(0));
        let entries_generation = Arc::new(AtomicU64::new(0));
        let groups = groups_generation.clone();
        let entries = entries_generation.clone();
        let thread = thread::Builder::new()
            .name("sift-collections".into())
            .spawn(move || {
                let mut store = match Store::open(&path) {
                    Ok(store) => store,
                    Err(error) => {
                        if sender.send(Event::StartupFailed(error.to_string())).is_ok() {
                            wake();
                        }
                        return;
                    }
                };
                let mut listening = sender.send(Event::Ready).is_ok();
                if listening {
                    wake();
                }
                while let Ok(command) = jobs.recv() {
                    let event = match command {
                        Command::Favorites {
                            generation,
                            query,
                            offset,
                        } => {
                            if !listening || generation < favorites.load(Ordering::Acquire) {
                                continue;
                            }
                            Event::Favorites {
                                generation,
                                result: store
                                    .favorites(&query, offset)
                                    .map_err(|error| error.to_string()),
                            }
                        }
                        Command::FavoriteStatus { generation, paths } => {
                            if !listening || generation < status.load(Ordering::Acquire) {
                                continue;
                            }
                            Event::FavoriteStatus {
                                generation,
                                result: store
                                    .favorite_status(&paths)
                                    .map_err(|error| error.to_string()),
                            }
                        }
                        Command::Groups { generation, offset } => {
                            if !listening || generation < groups.load(Ordering::Acquire) {
                                continue;
                            }
                            Event::Groups {
                                generation,
                                result: store.list(offset).map_err(|error| error.to_string()),
                            }
                        }
                        Command::Entries {
                            generation,
                            collection,
                            offset,
                        } => {
                            if !listening || generation < entries.load(Ordering::Acquire) {
                                continue;
                            }
                            Event::Entries {
                                generation,
                                collection,
                                result: store
                                    .entries(collection, offset)
                                    .map_err(|error| error.to_string()),
                            }
                        }
                        Command::Mutate {
                            operation,
                            mutation,
                        } => {
                            // Accepted edits are drained even after the UI disconnects.
                            // Successful completion is emitted only after SQL commit.
                            let result = match mutation {
                                Mutation::Favorite(reference) => store
                                    .favorite(&reference)
                                    .map(|()| Change::FavoriteChanged(reference.source)),
                                Mutation::Unfavorite(source) => store
                                    .unfavorite(&source)
                                    .map(|()| Change::FavoriteChanged(source)),
                                Mutation::Create(name) => store.create(&name).map(Change::Created),
                                Mutation::Rename { collection, name } => store
                                    .rename(collection, &name)
                                    .map(|()| Change::Updated(collection)),
                                Mutation::Delete(collection) => store
                                    .delete(collection)
                                    .map(|()| Change::Deleted(collection)),
                                Mutation::Add {
                                    collection,
                                    references,
                                } => store
                                    .add(collection, &references)
                                    .map(|()| Change::Updated(collection)),
                                Mutation::Remove { collection, entry } => store
                                    .remove(collection, entry)
                                    .map(|()| Change::Updated(collection)),
                            }
                            .map_err(|error| error.to_string());
                            Event::Saved { operation, result }
                        }
                    };
                    listening = sender.send(event).is_ok();
                    if listening {
                        wake();
                    }
                }
            })?;
        Ok(Self {
            commands: Some(commands),
            events: Some(events),
            favorites_generation,
            status_generation,
            groups_generation,
            entries_generation,
            thread: Some(thread),
        })
    }

    pub fn poll(&self) -> Option<Event> {
        self.events.as_ref()?.try_recv().ok()
    }

    pub fn favorites(&self, generation: u64, query: SearchQuery, offset: usize) -> bool {
        self.favorites_generation
            .fetch_max(generation, Ordering::AcqRel);
        self.send_query(Command::Favorites {
            generation,
            query,
            offset,
        })
    }

    pub fn favorite_status(&self, generation: u64, paths: Vec<PathBuf>) -> bool {
        self.status_generation
            .fetch_max(generation, Ordering::AcqRel);
        self.send_query(Command::FavoriteStatus { generation, paths })
    }

    pub fn groups(&self, generation: u64, offset: usize) -> bool {
        self.groups_generation
            .fetch_max(generation, Ordering::AcqRel);
        self.send_query(Command::Groups { generation, offset })
    }

    pub fn entries(&self, generation: u64, collection: CollectionId, offset: usize) -> bool {
        self.entries_generation
            .fetch_max(generation, Ordering::AcqRel);
        self.send_query(Command::Entries {
            generation,
            collection,
            offset,
        })
    }

    fn send_query(&self, command: Command) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|sender| sender.try_send(command).is_ok())
    }

    pub fn mutate(&self, operation: u64, mutation: Mutation) -> Result<(), String> {
        match self
            .commands
            .as_ref()
            .ok_or("Saved sound storage is unavailable")?
            .try_send(Command::Mutate {
                operation,
                mutation,
            }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                Err("Saved sound storage is busy; retry the pending change".into())
            }
            Err(TrySendError::Disconnected(_)) => {
                Err("Saved sound storage has stopped; reopen Sift to retry".into())
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.commands.take();
        self.events.take();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("Saved sound storage worker panicked during shutdown");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn shutdown_drains_favorite_saves_and_removals_without_reading_results() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved.sqlite");
        let source = directory.path().join("sound.wav");
        std::fs::write(&source, b"fixture").unwrap();
        let reference = Reference {
            source: source.clone(),
            name: "sound.wav".into(),
            display_path: "sound.wav".into(),
            format: "wav".into(),
            size_bytes: 7,
            audio: Default::default(),
            metadata_error: None,
        };
        let worker = Worker::start(path.clone(), || {}).unwrap();
        assert!(matches!(
            worker
                .events
                .as_ref()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap(),
            Event::Ready
        ));
        worker
            .mutate(1, Mutation::Favorite(reference.clone()))
            .unwrap();
        drop(worker);
        assert_eq!(
            Store::open(&path)
                .unwrap()
                .favorites(&SearchQuery::All, 0)
                .unwrap()
                .total,
            1
        );
        let worker = Worker::start(path.clone(), || {}).unwrap();
        worker
            .mutate(2, Mutation::Unfavorite(source.clone()))
            .unwrap();
        drop(worker);
        assert_eq!(
            Store::open(&path)
                .unwrap()
                .favorites(&SearchQuery::All, 0)
                .unwrap()
                .total,
            0
        );
        assert_eq!(std::fs::read(source).unwrap(), b"fixture");
    }

    #[test]
    fn shutdown_preserves_an_accepted_edit_with_unread_completion() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("collections.sqlite");
        let worker = Worker::start(path.clone(), || {}).unwrap();
        assert!(matches!(
            worker
                .events
                .as_ref()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap(),
            Event::Ready
        ));
        worker
            .mutate(1, Mutation::Create("Saved before closing".into()))
            .unwrap();
        drop(worker);
        let page = Store::open(&path).unwrap().list(0).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].name, "Saved before closing");
    }
}
