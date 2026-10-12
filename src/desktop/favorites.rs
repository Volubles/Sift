use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, RichText};

use crate::collections::{
    Page, Reference,
    favorites::{Favorite, FavoriteId, MAX_STATUS_PATHS},
    worker::{Change, Mutation, Worker},
};
use crate::search::{PAGE_SIZE, SearchQuery};

use super::pages::Pages;
use super::saved::{PendingSave, Storage};
use super::sound_table::{reference_row, table_heading};
use super::{
    SoundKey,
    selection::{self, Gesture, Selection},
};

#[derive(Default)]
pub(super) struct Favorites {
    selection: Selection,
    pub(super) query_text: String,
    query: SearchQuery,
    order: crate::search::SortOrder,
    generation: u64,
    view_generation: u64,
    pages: Pages<Favorite>,
    pub(super) total: usize,
    active: bool,
    desired: BTreeSet<PathBuf>,
    status: BTreeMap<PathBuf, bool>,
    status_generation: u64,
    status_pending: bool,
    status_needed: bool,
    status_failed: bool,
    error: Option<String>,
    save: Option<PendingSave>,
}

pub(super) enum Action {
    Select(Favorite),
    Drag(Favorite),
}

impl Favorites {
    pub(super) fn selected_count(&self) -> usize {
        self.selection.count(self.total)
    }

    pub(super) fn active_reference(&self) -> Option<(super::SoundKey, Reference)> {
        Some((
            self.selection.active()?,
            self.selection.reference()?.clone(),
        ))
    }

    pub(super) fn selected_references(
        &self,
    ) -> Result<crate::collections::ReferenceSelection, String> {
        Ok(crate::collections::ReferenceSelection {
            order: self.order,
            origin: crate::collections::SelectionOrigin::Favorites {
                query: self.query.clone(),
            },
            sources: self.selection.snapshot(self.total)?,
        })
    }

    pub(super) fn has_focus(&self, context: &egui::Context) -> bool {
        self.selection.has_focus(context)
    }
    pub(super) fn set_active(&mut self, active: bool) {
        if self.active && !active {
            self.selection.suspend();
        }
        if active && !self.active {
            self.refresh();
        }
        self.active = active;
    }

    pub(super) fn reset_query(&mut self) {
        self.selection.reset();
        self.view_generation += 1;
        self.total = 0;
        self.refresh();
        match SearchQuery::parse(&self.query_text) {
            Ok(query) => {
                self.query = query;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn refresh(&mut self) {
        self.selection.refresh();
        self.generation += 1;
        self.pages.clear();
    }

    pub(super) fn track(&mut self, paths: BTreeSet<PathBuf>) {
        debug_assert!(paths.len() <= MAX_STATUS_PATHS);
        if paths == self.desired {
            return;
        }
        self.status.retain(|path, _| paths.contains(path));
        self.desired = paths;
        self.invalidate_status();
        self.status_needed = self
            .desired
            .iter()
            .any(|path| !self.status.contains_key(path));
    }

    fn invalidate_status(&mut self) {
        self.status_generation += 1;
        self.status_pending = false;
        self.status_failed = false;
    }

    pub(super) fn status(&self, path: &Path) -> Option<bool> {
        self.status.get(path).copied()
    }

    pub(super) fn request(&mut self, worker: &Worker, ready: bool) {
        worker.invalidate_favorite_selection(self.selection.generation());
        if !ready {
            return;
        }
        if !self.desired.is_empty()
            && !self.status_pending
            && !self.status_failed
            && self.status_needed
        {
            self.status_pending = worker.favorite_status(
                self.status_generation,
                self.desired.iter().cloned().collect(),
            );
        }
        if self.active
            && let Some(request) = self.selection.position_request()
            && worker.positions(
                request.generation,
                crate::collections::SelectionOrigin::Favorites {
                    query: self.query.clone(),
                },
                self.order,
                request.active,
                request.anchor,
            )
        {
            self.selection.position_sent();
        }
        if self.active && self.pages.is_empty() {
            self.request_page(worker, 0);
        }
    }

    fn request_page(&mut self, worker: &Worker, offset: usize) {
        if self.error.is_none()
            && self.pages.needs(offset)
            && worker.favorites_ordered(self.generation, self.query.clone(), self.order, offset)
        {
            self.pages.requested(offset);
        }
    }

    pub(super) fn positions(
        &mut self,
        worker: &Worker,
        generation: u64,
        result: Result<crate::search::SelectionPositions, String>,
    ) {
        if let Some(index) = self.selection.positions_resolved(generation, result) {
            self.request_page(worker, index / PAGE_SIZE * PAGE_SIZE);
        }
    }

    fn reorder(&mut self) {
        self.selection.reorder();
        self.generation += 1;
        self.pages.clear();
    }

    pub(super) fn page(&mut self, generation: u64, result: Result<Page<Favorite>, String>) {
        if generation != self.generation {
            return;
        }
        match result {
            Ok(page) => {
                self.total = page.total;
                for (position, sound) in page.rows.iter().enumerate() {
                    self.selection.observe(
                        SoundKey::Favorite(sound.id),
                        page.offset + position,
                        &sound.reference.source,
                    );
                }
                self.pages.insert(page);
            }
            Err(error) => {
                eprintln!("Favorites page failed: {error}");
                self.pages.clear_pending();
                self.error = Some(format!("Could not read favorites: {error}"));
            }
        }
    }

    pub(super) fn membership(
        &mut self,
        generation: u64,
        result: Result<Vec<(PathBuf, bool)>, String>,
    ) {
        if generation != self.status_generation {
            return;
        }
        self.status_pending = false;
        match result {
            Ok(status) => {
                self.status = status.into_iter().collect();
                self.status_needed = false;
            }
            Err(error) => {
                eprintln!("Favorite status failed: {error}");
                self.status_failed = true;
                self.error = Some(format!("Could not read favorite status: {error}"));
            }
        }
    }

    pub(super) fn selection_result(
        &mut self,
        generation: u64,
        result: Result<crate::search::SelectionSources, String>,
    ) {
        self.selection.resolved(generation, result);
    }

    pub(super) fn can_edit(&self) -> bool {
        self.save.is_none()
    }

    pub(super) fn startup_failed(&mut self, error: &str) {
        self.error = Some(format!(
            "Saved sounds unavailable: {error}; reopen Sift to retry"
        ));
    }

    pub(super) fn settled(&self) -> bool {
        (!self.active || !self.pages.is_empty()) && !self.status_needed && self.save.is_none()
    }

    pub(super) fn toggle(&mut self, storage: &mut Storage, reference: &Reference) {
        if !storage.ready || !self.can_edit() {
            return;
        }
        let mutation = match self.status(&reference.source) {
            Some(true) => Mutation::Unfavorite(reference.source.clone()),
            Some(false) => Mutation::Favorite(reference.clone()),
            None => Mutation::ToggleFavorite(reference.clone()),
        };
        self.mutate(storage, mutation);
    }

    fn mutate(&mut self, storage: &mut Storage, mutation: Mutation) {
        self.save = Some(storage.submit(mutation));
    }

    pub(super) fn owns_save(&self, operation: u64) -> bool {
        self.save
            .as_ref()
            .is_some_and(|save| save.operation == operation)
    }

    pub(super) fn saved(&mut self, result: Result<Change, String>) {
        match result {
            Ok(Change::FavoriteChanged(source)) => {
                let is_favorite = self
                    .save
                    .as_ref()
                    .is_some_and(|save| matches!(save.mutation, Mutation::Favorite(_)));
                self.favorite_committed(source, is_favorite);
            }
            Ok(Change::FavoriteToggled { source, favorite }) => {
                self.favorite_committed(source, favorite)
            }
            Ok(_) => {
                self.error = Some(
                    "Unexpected favorite save result; reopen Sift to reload saved data".into(),
                );
                self.save = None;
            }
            Err(error) => {
                eprintln!("Favorite edit failed: {error}");
                if let Some(save) = &mut self.save {
                    save.error = Some(error);
                }
            }
        }
    }

    fn favorite_committed(&mut self, source: PathBuf, is_favorite: bool) {
        if !is_favorite {
            self.selection.forget(&source);
        }
        self.save = None;
        // Discard older membership replies before publishing the committed value.
        self.invalidate_status();
        self.status_needed = !self.desired.is_empty();
        if self.desired.contains(&source) {
            self.status.insert(source, is_favorite);
        }
        self.refresh();
    }

    pub(super) fn sources_changed(&mut self, paths: &[PathBuf]) {
        if self.pages.iter().flat_map(|page| &page.rows).any(|sound| {
            paths
                .iter()
                .any(|path| sound.reference.source.starts_with(path))
        }) {
            self.refresh();
        }
    }

    pub(super) fn feedback(&mut self, ui: &mut egui::Ui, storage: &mut Storage) {
        if let Some(error) = &self.error {
            ui.colored_label(Color32::from_rgb(245, 143, 140), error);
            if ui.button("Retry favorites").clicked() {
                self.reset_query();
                self.invalidate_status();
                self.status_needed = !self.desired.is_empty();
            }
        }
        let failed = self.save.as_ref().and_then(|save| {
            save.error
                .as_ref()
                .map(|error| (error.clone(), save.mutation.clone()))
        });
        if let Some((error, mutation)) = failed {
            ui.colored_label(
                Color32::from_rgb(245, 143, 140),
                format!("Favorite not saved: {error}"),
            );
            ui.horizontal(|ui| {
                if ui.button("Retry favorite save").clicked() {
                    self.mutate(storage, mutation);
                }
                if ui.button("Discard unsaved favorite change").clicked() {
                    self.save = None;
                }
            });
        } else if self.save.is_some() {
            ui.label(RichText::new("Saving favorite...").small().weak());
        }
    }

    pub(super) fn list(
        &mut self,
        ui: &mut egui::Ui,
        storage: &mut Storage,
        selected: Option<FavoriteId>,
        active_reference: Option<&Reference>,
    ) -> Option<Action> {
        ui.heading("Favorites");
        ui.separator();
        if !storage.ready {
            ui.label("Opening saved sounds...");
            return None;
        }
        self.request(&storage.worker, storage.ready);
        if self.total == 0 {
            ui.label(if self.pages.has_pending() {
                "Loading favorites..."
            } else if self.query_text.is_empty() {
                "Star a sound in Libraries or Collections to save it here."
            } else {
                "No matching favorites."
            });
            return None;
        }
        let can_edit = self.can_edit();
        let mut needed = BTreeSet::new();
        let focus_id = ui.make_persistent_id("favorite-sound-focus");
        let favorite_key = self.selection.keyboard(ui, focus_id, self.total);
        let target = self.selection.target();
        let mut choice = target.and_then(|(index, gesture)| {
            self.pages
                .iter()
                .find(|page| index >= page.offset && index < page.offset + page.rows.len())
                .and_then(|page| page.rows.get(index - page.offset))
                .map(|sound| (sound.clone(), index, gesture))
        });
        let mut action = None;
        let mut remove = None;
        self.selection.feedback(ui, self.total);
        if table_heading(ui, &mut self.order) {
            self.reorder();
            self.request(&storage.worker, storage.ready);
            selection::focus(ui, focus_id);
            return None;
        }
        let scroll = self
            .selection
            .scroll_area(
                ui,
                egui::ScrollArea::vertical()
                    .id_salt(("favorites", self.view_generation))
                    .auto_shrink([false, false]),
                0,
                ui.available_height(),
            )
            .show_rows(ui, 28.0, self.total, |ui, range| {
                let first_visible = range.start;
                for index in range {
                    let offset = index / PAGE_SIZE * PAGE_SIZE;
                    needed.insert(offset);
                    if let Some(sound) = self
                        .pages
                        .iter()
                        .find(|page| page.offset == offset)
                        .and_then(|page| page.rows.get(index - offset))
                    {
                        let key = SoundKey::Favorite(sound.id);
                        if selected == Some(sound.id) {
                            self.selection.seed(key, index, &sound.reference.source);
                            if self.selection.reference().is_none() {
                                self.selection.remember(sound.reference.clone());
                            }
                        }
                        ui.horizontal(|ui| {
                            if star_button(ui, Some(true), can_edit, &sound.reference.name)
                                .clicked()
                            {
                                remove = Some(sound.reference.source.clone());
                            }
                            let response = reference_row(
                                ui,
                                &sound.reference,
                                &sound.availability,
                                self.selection.contains(&sound.reference.source, index),
                                index,
                            );
                            selection::active_outline(
                                ui,
                                &response,
                                self.selection.active_source()
                                    == Some(sound.reference.source.as_path()),
                            );
                            if response.clicked() {
                                choice = Some((
                                    sound.clone(),
                                    index,
                                    Gesture::from_modifiers(ui.input(|input| input.modifiers)),
                                ));
                                selection::focus(ui, focus_id);
                            }
                            if sound.availability.usable()
                                && response.drag_started_by(egui::PointerButton::Primary)
                            {
                                if !self.selection.contains(&sound.reference.source, index) {
                                    self.selection.choose(
                                        key,
                                        index,
                                        &sound.reference.source,
                                        Gesture::Replace,
                                    );
                                }
                                self.selection.activate(key, index, &sound.reference.source);
                                self.selection.remember(sound.reference.clone());
                                selection::focus(ui, focus_id);
                                choice = None;
                                action = Some(Action::Drag(sound.clone()));
                            }
                            response.on_hover_text(sound.reference.source.display().to_string());
                        });
                    } else {
                        ui.add_sized(
                            [ui.available_width(), 28.0],
                            egui::Label::new("Loading favorites..."),
                        );
                    }
                }
                first_visible
            });
        self.selection
            .remember_scroll(scroll.state.offset.y, scroll.inner);
        if let Some((sound, index, gesture)) = choice {
            if !self.selection.choose(
                SoundKey::Favorite(sound.id),
                index,
                &sound.reference.source,
                gesture,
            ) {
                return None;
            }
            self.selection.remember(sound.reference.clone());
            ui.ctx().request_repaint();
            action = Some(Action::Select(sound));
        }
        if favorite_key
            && self.can_edit()
            && let Some(SoundKey::Favorite(id)) = self.selection.active()
        {
            remove = self
                .selection
                .reference()
                .map(|reference| reference.source.clone())
                .or_else(|| {
                    self.pages
                        .iter()
                        .flat_map(|page| &page.rows)
                        .find(|sound| sound.id == id)
                        .map(|sound| sound.reference.source.clone())
                        .or_else(|| {
                            (selected == Some(id))
                                .then_some(active_reference)
                                .flatten()
                                .map(|reference| reference.source.clone())
                        })
                });
        }
        if let Some(source) = remove {
            self.mutate(storage, Mutation::Unfavorite(source));
        }
        for offset in needed {
            self.request_page(&storage.worker, offset);
        }
        if let Some((index, _)) = target {
            self.request_page(&storage.worker, index / PAGE_SIZE * PAGE_SIZE);
        }
        if let Some(request) = self.selection.request()
            && storage.worker.favorite_selection_ordered(
                request.generation,
                self.query.clone(),
                self.order,
                request.anchor,
                request.target,
            )
        {
            self.selection.sent();
        }
        action
    }
}

pub(super) fn star_button(
    ui: &mut egui::Ui,
    status: Option<bool>,
    enabled: bool,
    name: &str,
) -> egui::Response {
    let label = match status {
        Some(true) => "★",
        Some(false) => "☆",
        None => "·",
    };
    let description = match status {
        Some(true) => "Remove from favorites",
        Some(false) => "Add to favorites",
        None => "Loading favorite status",
    };
    let color = if status == Some(true) {
        Color32::from_rgb(235, 193, 103)
    } else {
        Color32::from_gray(155)
    };
    let response = ui
        .add_enabled_ui(enabled && status.is_some(), |ui| {
            ui.add_sized(
                [26.0, 28.0],
                egui::Button::new(RichText::new(label).color(color)).frame(false),
            )
        })
        .inner;
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            enabled && status.is_some(),
            status == Some(true),
            format!("{description}: {name}"),
        )
    });
    response.on_hover_text(format!("{description}: {name}"))
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/favorites.rs"
    ));
}
