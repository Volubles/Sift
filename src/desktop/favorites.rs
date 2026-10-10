use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, RichText};

use crate::collections::{
    Page, Reference,
    favorites::{Favorite, FavoriteId, MAX_STATUS_PATHS},
    worker::{Change, Mutation, Worker},
};
use crate::search::{PAGE_SIZE, SearchQuery};

use super::sound_table::{reference_row, table_heading};

#[derive(Default)]
pub(super) struct Favorites {
    pub(super) query_text: String,
    query: SearchQuery,
    generation: u64,
    view_generation: u64,
    pages: VecDeque<Page<Favorite>>,
    pending: BTreeSet<usize>,
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

struct PendingSave {
    operation: u64,
    mutation: Mutation,
    error: Option<String>,
}

pub(super) enum Action {
    Select(Favorite),
    Drag(Favorite),
}

impl Favorites {
    pub(super) fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    pub(super) fn reset_query(&mut self) {
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
        self.generation += 1;
        self.pages.clear();
        self.pending.clear();
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
        if self.active && self.pages.is_empty() {
            self.request_page(worker, 0);
        }
    }

    fn request_page(&mut self, worker: &Worker, offset: usize) {
        if self.error.is_none()
            && !self.pending.contains(&offset)
            && !self.pages.iter().any(|page| page.offset == offset)
            && worker.favorites(self.generation, self.query.clone(), offset)
        {
            self.pending.insert(offset);
        }
    }

    pub(super) fn page(&mut self, generation: u64, result: Result<Page<Favorite>, String>) {
        if generation != self.generation {
            return;
        }
        match result {
            Ok(page) => {
                self.total = page.total;
                self.pending.remove(&page.offset);
                self.pages.retain(|cached| cached.offset != page.offset);
                self.pages.push_back(page);
                while self.pages.len() > 4 {
                    self.pages.pop_front();
                }
            }
            Err(error) => {
                eprintln!("Favorites page failed: {error}");
                self.pending.clear();
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

    pub(super) fn toggle(&mut self, worker: &Worker, operation: &mut u64, reference: &Reference) {
        if !self.can_edit() {
            return;
        }
        let mutation = match self.status(&reference.source) {
            Some(true) => Mutation::Unfavorite(reference.source.clone()),
            Some(false) => Mutation::Favorite(reference.clone()),
            None => return,
        };
        self.mutate(worker, operation, mutation);
    }

    fn mutate(&mut self, worker: &Worker, operation: &mut u64, mutation: Mutation) {
        *operation += 1;
        let error = worker.mutate(*operation, mutation.clone()).err();
        self.save = Some(PendingSave {
            operation: *operation,
            mutation,
            error,
        });
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
                self.save = None;
                // Discard older membership replies before publishing the committed value.
                self.invalidate_status();
                self.status_needed = !self.desired.is_empty();
                if self.desired.contains(&source) {
                    self.status.insert(source, is_favorite);
                }
                self.refresh();
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

    pub(super) fn sources_changed(&mut self, paths: &[PathBuf]) {
        if self.pages.iter().flat_map(|page| &page.rows).any(|sound| {
            paths
                .iter()
                .any(|path| sound.reference.source.starts_with(path))
        }) {
            self.refresh();
        }
    }

    pub(super) fn feedback(&mut self, ui: &mut egui::Ui, worker: &Worker, operation: &mut u64) {
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
                    self.mutate(worker, operation, mutation);
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
        worker: &Worker,
        operation: &mut u64,
        ready: bool,
        selected: Option<FavoriteId>,
    ) -> Option<Action> {
        ui.heading("Favorites");
        ui.separator();
        if !ready {
            ui.label("Opening saved sounds...");
            return None;
        }
        self.request(worker, ready);
        if self.total == 0 {
            ui.label(if !self.pending.is_empty() {
                "Loading favorites..."
            } else if self.query_text.is_empty() {
                "Star a sound in Libraries or Collections to save it here."
            } else {
                "No matching favorites."
            });
            return None;
        }
        let mut needed = BTreeSet::new();
        let mut action = None;
        let mut remove = None;
        table_heading(ui);
        egui::ScrollArea::vertical()
            .id_salt(("favorites", self.view_generation))
            .auto_shrink([false, false])
            .show_rows(ui, 28.0, self.total, |ui, range| {
                for index in range {
                    let offset = index / PAGE_SIZE * PAGE_SIZE;
                    needed.insert(offset);
                    if let Some(sound) = self
                        .pages
                        .iter()
                        .find(|page| page.offset == offset)
                        .and_then(|page| page.rows.get(index - offset))
                    {
                        ui.horizontal(|ui| {
                            if star_button(ui, Some(true), self.can_edit(), &sound.reference.name)
                                .clicked()
                            {
                                remove = Some(sound.reference.source.clone());
                            }
                            let response = reference_row(
                                ui,
                                &sound.reference,
                                &sound.availability,
                                selected == Some(sound.id),
                                index,
                            );
                            if response.clicked() {
                                action = Some(Action::Select(sound.clone()));
                            }
                            if sound.availability.usable()
                                && response.drag_started_by(egui::PointerButton::Primary)
                            {
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
            });
        if let Some(source) = remove {
            self.mutate(worker, operation, Mutation::Unfavorite(source));
        }
        for offset in needed {
            self.request_page(worker, offset);
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
