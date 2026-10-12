use std::collections::BTreeSet;
use std::path::PathBuf;

use eframe::egui;

use crate::collections::{
    Reference,
    worker::{Event, Mutation, Worker},
};

use super::{collections::Collections, favorites::Favorites};

/// One durable-storage actor shared by two independently owned sound views.
pub(super) struct SavedSounds {
    pub(super) collections: Collections,
    pub(super) favorites: Favorites,
    pub(super) storage: Storage,
}

pub(super) struct Storage {
    pub(super) worker: Worker,
    pub(super) ready: bool,
    operation: u64,
}

pub(super) struct PendingSave {
    pub(super) operation: u64,
    pub(super) mutation: Mutation,
    pub(super) error: Option<String>,
}

impl Storage {
    pub(super) fn new(worker: Worker) -> Self {
        Self {
            worker,
            ready: false,
            operation: 0,
        }
    }
    pub(super) fn submit(&mut self, mutation: Mutation) -> PendingSave {
        self.operation += 1;
        let error = self.worker.mutate(self.operation, mutation.clone()).err();
        PendingSave {
            operation: self.operation,
            mutation,
            error,
        }
    }
}

impl SavedSounds {
    pub(super) fn new(worker: Worker) -> Self {
        Self {
            collections: Collections::new(),
            favorites: Favorites::default(),
            storage: Storage::new(worker),
        }
    }

    pub(super) fn poll(&mut self) {
        self.collections.invalidate_selection(&self.storage);
        while let Some(event) = self.storage.worker.poll() {
            match event {
                Event::Ready => {
                    self.storage.ready = true;
                    self.collections.storage_ready();
                }
                Event::StartupFailed(error) => {
                    self.favorites.startup_failed(&error);
                    self.collections.startup_failed(&error);
                }
                Event::FavoriteSelection { generation, result } => {
                    self.favorites.selection_result(generation, result)
                }
                Event::Favorites { generation, result } => self.favorites.page(generation, result),
                Event::FavoriteStatus { generation, result } => {
                    self.favorites.membership(generation, result)
                }
                Event::Positions {
                    generation,
                    origin: crate::collections::SelectionOrigin::Favorites { .. },
                    result,
                } => self
                    .favorites
                    .positions(&self.storage.worker, generation, result),
                Event::Saved { operation, result } if self.favorites.owns_save(operation) => {
                    self.favorites.saved(result)
                }
                event => self.collections.event(&self.storage, event),
            }
        }
        self.collections.request_pages(&self.storage);
        self.favorites
            .request(&self.storage.worker, self.storage.ready);
    }

    pub(super) fn settled(&self) -> bool {
        self.storage.ready && self.collections.settled() && self.favorites.settled()
    }

    pub(super) fn track_favorites(&mut self, mut paths: BTreeSet<PathBuf>) {
        self.collections.favorite_paths_dirty = false;
        paths.extend(self.collections.visible_sources());
        self.favorites.track(paths);
        self.favorites
            .request(&self.storage.worker, self.storage.ready);
    }

    pub(super) fn toggle_favorite(&mut self, reference: &Reference) {
        self.favorites.toggle(&mut self.storage, reference);
    }

    pub(super) fn favorite_feedback(&mut self, ui: &mut egui::Ui) {
        self.favorites.feedback(ui, &mut self.storage);
    }

    pub(super) fn favorite_list(
        &mut self,
        ui: &mut egui::Ui,
        selected: Option<crate::collections::favorites::FavoriteId>,
        active_reference: Option<&Reference>,
    ) -> Option<super::favorites::Action> {
        self.favorites
            .list(ui, &mut self.storage, selected, active_reference)
    }

    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        selected: Option<&Reference>,
        selected_entry: Option<crate::collections::EntryId>,
        marked_count: usize,
    ) -> Option<super::collections::Action> {
        self.collections.ui(
            ui,
            &mut self.storage,
            &mut self.favorites,
            selected,
            selected_entry,
            marked_count,
        )
    }

    pub(super) fn sources_changed(&mut self, paths: &[PathBuf]) {
        self.favorites.sources_changed(paths);
        self.collections.sources_changed(paths);
    }

    pub(super) fn sound_has_focus(&self, context: &egui::Context) -> bool {
        self.collections.sound_has_focus(context) || self.favorites.has_focus(context)
    }
}
