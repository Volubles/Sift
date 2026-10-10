use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver, SyncSender, TrySendError},
};
use std::thread::{self, JoinHandle};

use super::favorites::Favorite;
use crate::search::SearchQuery;

use super::{
    Collection, CollectionId, Entry, EntryId, Page, Reference, ReferenceSelection, SelectionOrigin,
    Store,
};
use crate::search::SourceSelection;

#[derive(Clone, Debug)]
pub enum Mutation {
    Favorite(Reference),
    ToggleFavorite(Reference),
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
    AddSelected {
        collection: CollectionId,
        selection: ReferenceSelection,
    },
    RemoveSelected {
        collection: CollectionId,
        selection: SourceSelection,
    },
    Remove {
        collection: CollectionId,
        entry: EntryId,
    },
}

#[derive(Debug)]
pub enum Change {
    FavoriteChanged(PathBuf),
    FavoriteToggled {
        source: PathBuf,
        favorite: bool,
    },
    Created(CollectionId),
    Updated(CollectionId),
    Removed {
        collection: CollectionId,
        sources: Vec<PathBuf>,
    },
    Deleted(CollectionId),
}

#[derive(Debug)]
pub enum Event {
    Positions {
        generation: u64,
        origin: SelectionOrigin,
        result: Result<crate::search::SelectionPositions, String>,
    },
    Ready,
    FavoriteSelection {
        generation: u64,
        result: Result<crate::search::SelectionSources, String>,
    },
    EntrySelection {
        generation: u64,
        result: Result<crate::search::SelectionSources, String>,
    },
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
    Positions {
        generation: u64,
        origin: SelectionOrigin,
        order: crate::search::SortOrder,
        active: PathBuf,
        anchor: Option<PathBuf>,
    },
    FavoriteSelection {
        order: crate::search::SortOrder,
        generation: u64,
        query: SearchQuery,
        anchor: PathBuf,
        target: PathBuf,
    },
    EntrySelection {
        order: crate::search::SortOrder,
        generation: u64,
        collection: CollectionId,
        anchor: PathBuf,
        target: PathBuf,
    },
    Favorites {
        order: crate::search::SortOrder,
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
        order: crate::search::SortOrder,
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
    favorite_selection_generation: Arc<AtomicU64>,
    entry_selection_generation: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(path: PathBuf, wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        Self::start_with_catalog(path, None, wake)
    }

    pub fn start_with_catalog(
        path: PathBuf,
        catalog_path: Option<PathBuf>,
        wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
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
        let favorite_selection_generation = Arc::new(AtomicU64::new(0));
        let entry_selection_generation = Arc::new(AtomicU64::new(0));
        let favorite_selection = favorite_selection_generation.clone();
        let entry_selection = entry_selection_generation.clone();
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
                // One lazy read-only connection, owned and closed by this actor.
                // Accepted group edits resolve their inputs here and still drain
                // on shutdown; no second UI round-trip is needed to commit them.
                let mut catalog = None;
                if listening {
                    wake();
                }
                while let Ok(command) = jobs.recv() {
                    let stale_read = match &command {
                        Command::Positions {
                            generation, origin, ..
                        } => {
                            *generation
                                < match origin {
                                    SelectionOrigin::Favorites { .. } => {
                                        favorite_selection.load(Ordering::Acquire)
                                    }
                                    _ => entry_selection.load(Ordering::Acquire),
                                }
                        }
                        Command::FavoriteSelection { generation, .. } => {
                            *generation < favorite_selection.load(Ordering::Acquire)
                        }
                        Command::EntrySelection { generation, .. } => {
                            *generation < entry_selection.load(Ordering::Acquire)
                        }
                        Command::Favorites { generation, .. } => {
                            *generation < favorites.load(Ordering::Acquire)
                        }
                        Command::FavoriteStatus { generation, .. } => {
                            *generation < status.load(Ordering::Acquire)
                        }
                        Command::Groups { generation, .. } => {
                            *generation < groups.load(Ordering::Acquire)
                        }
                        Command::Entries { generation, .. } => {
                            *generation < entries.load(Ordering::Acquire)
                        }
                        Command::Mutate { .. } => false,
                    };
                    if !matches!(command, Command::Mutate { .. }) && (!listening || stale_read) {
                        // A superseded read still frees command capacity. Wake the
                        // UI so a query rejected by backpressure can retry.
                        if listening {
                            wake();
                        }
                        continue;
                    }
                    let event = match command {
                        Command::Positions {
                            generation,
                            origin,
                            order,
                            active,
                            anchor,
                        } => {
                            let result = match &origin {
                                SelectionOrigin::Favorites { query } => store
                                    .favorite_selection_positions(
                                        query,
                                        order,
                                        &active,
                                        anchor.as_deref(),
                                    ),
                                SelectionOrigin::Collection(id) => store.entry_selection_positions(
                                    *id,
                                    order,
                                    &active,
                                    anchor.as_deref(),
                                ),
                                SelectionOrigin::Library { .. } => {
                                    Err(super::Error::CatalogUnavailable)
                                }
                            }
                            .map_err(|error| error.to_string());
                            Event::Positions {
                                generation,
                                origin,
                                result,
                            }
                        }
                        Command::FavoriteSelection {
                            order,
                            generation,
                            query,
                            anchor,
                            target,
                        } => Event::FavoriteSelection {
                            generation,
                            result: store
                                .favorite_selection_sources_ordered(&query, order, &anchor, &target)
                                .map_err(|error| error.to_string()),
                        },
                        Command::EntrySelection {
                            order,
                            generation,
                            collection,
                            anchor,
                            target,
                        } => Event::EntrySelection {
                            generation,
                            result: store
                                .entry_selection_sources_ordered(
                                    collection, order, &anchor, &target,
                                )
                                .map_err(|error| error.to_string()),
                        },
                        Command::Favorites {
                            order,
                            generation,
                            query,
                            offset,
                        } => Event::Favorites {
                            generation,
                            result: store
                                .favorites_ordered(&query, order, offset)
                                .map_err(|error| error.to_string()),
                        },
                        Command::FavoriteStatus { generation, paths } => Event::FavoriteStatus {
                            generation,
                            result: store
                                .favorite_status(&paths)
                                .map_err(|error| error.to_string()),
                        },
                        Command::Groups { generation, offset } => Event::Groups {
                            generation,
                            result: store.list(offset).map_err(|error| error.to_string()),
                        },
                        Command::Entries {
                            order,
                            generation,
                            collection,
                            offset,
                        } => Event::Entries {
                            generation,
                            collection,
                            result: store
                                .entries_ordered(collection, order, offset)
                                .map_err(|error| error.to_string()),
                        },
                        Command::Mutate {
                            operation,
                            mutation,
                        } => {
                            // Accepted edits are drained even after the UI disconnects.
                            // Successful completion is emitted only after SQL commit.
                            let result = match mutation {
                                Mutation::ToggleFavorite(reference) => store
                                    .toggle_favorite(&reference)
                                    .map(|favorite| Change::FavoriteToggled {
                                        source: reference.source,
                                        favorite,
                                    }),
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
                                Mutation::AddSelected {
                                    collection,
                                    selection,
                                } => resolve_references(
                                    &mut store,
                                    &mut catalog,
                                    catalog_path.as_deref(),
                                    &selection,
                                )
                                .and_then(|references| store.add(collection, &references))
                                .map(|()| Change::Updated(collection)),
                                Mutation::RemoveSelected {
                                    collection,
                                    selection,
                                } => store
                                    .remove_selected(collection, &selection)
                                    .map(|sources| Change::Removed {
                                        collection,
                                        sources,
                                    }),
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
            favorite_selection_generation,
            entry_selection_generation,
            thread: Some(thread),
        })
    }

    pub fn poll(&self) -> Option<Event> {
        self.events.as_ref()?.try_recv().ok()
    }

    pub fn favorites(&self, generation: u64, query: SearchQuery, offset: usize) -> bool {
        self.favorites_ordered(
            generation,
            query,
            crate::search::SortOrder::default(),
            offset,
        )
    }
    pub fn favorites_ordered(
        &self,
        generation: u64,
        query: SearchQuery,
        order: crate::search::SortOrder,
        offset: usize,
    ) -> bool {
        self.favorites_generation
            .fetch_max(generation, Ordering::AcqRel);
        self.send_query(Command::Favorites {
            order,
            generation,
            query,
            offset,
        })
    }

    pub fn favorite_selection(
        &self,
        generation: u64,
        query: SearchQuery,
        anchor: PathBuf,
        target: PathBuf,
    ) -> bool {
        self.favorite_selection_ordered(
            generation,
            query,
            crate::search::SortOrder::default(),
            anchor,
            target,
        )
    }
    pub fn favorite_selection_ordered(
        &self,
        generation: u64,
        query: SearchQuery,
        order: crate::search::SortOrder,
        anchor: PathBuf,
        target: PathBuf,
    ) -> bool {
        self.favorite_selection_generation
            .fetch_max(generation, Ordering::Relaxed);
        self.send_query(Command::FavoriteSelection {
            order,
            generation,
            query,
            anchor,
            target,
        })
    }

    pub fn invalidate_favorite_selection(&self, generation: u64) {
        self.favorite_selection_generation
            .fetch_max(generation, Ordering::Relaxed);
    }

    pub fn invalidate_entry_selection(&self, generation: u64) {
        self.entry_selection_generation
            .fetch_max(generation, Ordering::Relaxed);
    }

    pub fn entry_selection(
        &self,
        generation: u64,
        collection: CollectionId,
        anchor: PathBuf,
        target: PathBuf,
    ) -> bool {
        self.entry_selection_ordered(
            generation,
            collection,
            crate::search::SortOrder::INSERTION,
            anchor,
            target,
        )
    }
    pub fn entry_selection_ordered(
        &self,
        generation: u64,
        collection: CollectionId,
        order: crate::search::SortOrder,
        anchor: PathBuf,
        target: PathBuf,
    ) -> bool {
        self.entry_selection_generation
            .fetch_max(generation, Ordering::Relaxed);
        self.send_query(Command::EntrySelection {
            order,
            generation,
            collection,
            anchor,
            target,
        })
    }

    pub fn positions(
        &self,
        generation: u64,
        origin: SelectionOrigin,
        order: crate::search::SortOrder,
        active: PathBuf,
        anchor: Option<PathBuf>,
    ) -> bool {
        match &origin {
            SelectionOrigin::Favorites { .. } => self.invalidate_favorite_selection(generation),
            _ => self.invalidate_entry_selection(generation),
        }
        self.send_query(Command::Positions {
            generation,
            origin,
            order,
            active,
            anchor,
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
        self.entries_ordered(
            generation,
            collection,
            crate::search::SortOrder::INSERTION,
            offset,
        )
    }
    pub fn entries_ordered(
        &self,
        generation: u64,
        collection: CollectionId,
        order: crate::search::SortOrder,
        offset: usize,
    ) -> bool {
        self.entries_generation
            .fetch_max(generation, Ordering::AcqRel);
        self.send_query(Command::Entries {
            order,
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

fn resolve_references(
    store: &mut Store,
    catalog: &mut Option<crate::catalog::Catalog>,
    catalog_path: Option<&std::path::Path>,
    selection: &ReferenceSelection,
) -> super::Result<Vec<Reference>> {
    if let SelectionOrigin::Library { query, scope } = &selection.origin {
        if catalog.is_none() {
            *catalog = Some(crate::catalog::Catalog::open_reader(
                catalog_path.ok_or(super::Error::CatalogUnavailable)?,
            )?);
        }
        let catalog = catalog.as_mut().ok_or(super::Error::CatalogUnavailable)?;
        Ok(catalog.selected_references_ordered(
            query,
            scope,
            selection.order,
            &selection.sources,
        )?)
    } else {
        store.selected_references(selection)
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

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/collections/worker.rs"
    ));
}
