use crate::tags::{Change, Cursor, MAX_NAME_CHARS, Mutation, Page, Tag, TagUse, worker::Worker};
use eframe::egui::{self, Color32, RichText};
use std::collections::VecDeque;

const MAX_PREVIOUS_PAGES: usize = 32;

#[derive(Default)]
enum Mode {
    #[default]
    Details,
    Merge {
        destination: Option<Tag>,
    },
    Delete,
}

struct Selected {
    used: TagUse,
    name: String,
    mode: Mode,
}

struct Save {
    operation: u64,
    mutation: Mutation,
    error: Option<String>,
}

/// The global-definition dialog owns its paging, confirmation and accepted mutation.
#[derive(Default)]
pub(super) struct Manager {
    pub open: bool,
    prefix: String,
    after: Option<Cursor>,
    previous: VecDeque<Option<Cursor>>,
    page: Option<Page>,
    generation: u64,
    pending: bool,
    needed: bool,
    scroll_to_top: bool,
    error: Option<String>,
    selected: Option<Selected>,
    operation: u64,
    save: Option<Save>,
}

impl Manager {
    pub fn open(&mut self) {
        self.open = true;
        if self.save.is_none() {
            self.selected = None;
            self.prefix.clear();
        }
        self.first_page();
    }

    fn invalidate(&mut self) {
        self.generation += 1;
        self.page = None;
        self.pending = false;
        self.needed = true;
        self.scroll_to_top = true;
        self.error = None;
    }

    fn first_page(&mut self) {
        self.after = None;
        self.previous.clear();
        self.invalidate();
    }

    pub fn settled(&self) -> bool {
        (!self.open || (!self.pending && !self.needed)) && self.save.is_none()
    }

    pub fn request(&mut self, worker: &Worker) {
        if self.open
            && self.needed
            && !self.pending
            && worker.management_page(self.generation, self.prefix.clone(), self.after.clone())
        {
            self.pending = true;
            self.needed = false;
        }
    }

    pub fn page(&mut self, generation: u64, result: Result<Page, String>) {
        if generation != self.generation {
            return;
        }
        self.pending = false;
        match result {
            Ok(page) => {
                self.page = Some(page);
                self.error = None;
            }
            Err(error) => {
                eprintln!("Tag management listing failed: {error}");
                self.error = Some(error);
            }
        }
    }

    pub fn saved(&mut self, operation: u64, result: Result<Change, String>) -> Option<Change> {
        if !self
            .save
            .as_ref()
            .is_some_and(|save| save.operation == operation)
        {
            return None;
        }
        match result {
            Ok(change) => {
                self.save = None;
                self.selected = None;
                self.first_page();
                Some(change)
            }
            Err(error) => {
                if let Some(save) = &mut self.save {
                    save.error = Some(error);
                }
                None
            }
        }
    }

    fn submit(&mut self, worker: &Worker, mutation: Mutation) {
        if self.save.is_some() {
            return;
        }
        self.operation += 1;
        let error = worker.manage(self.operation, mutation.clone()).err();
        self.save = Some(Save {
            operation: self.operation,
            mutation,
            error,
        });
    }

    pub fn ui(&mut self, ctx: &egui::Context, worker: &Worker) {
        if !self.open {
            return;
        }
        let mut close = false;
        let mut mutation = None;
        let response = egui::Modal::new(egui::Id::new("manage-tags")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 40.0).clamp(240.0, 520.0));
            ui.heading("Manage tags");
            ui.weak("Global labels across libraries, favorites and collections.");
            ui.add_space(6.0);
            ui.add_enabled_ui(self.save.is_none(), |ui| {
                self.listing(ui, ctx);
                ui.separator();
                mutation = self.details(ui);
            });
            self.save_controls(ui, worker);
            ui.add_space(6.0);
            if ui
                .add_enabled(!self.saving(), egui::Button::new("Close"))
                .clicked()
            {
                close = true;
            }
        });
        if let Some(mutation) = mutation {
            self.submit(worker, mutation);
        }
        if !self.saving() && (close || response.should_close()) {
            self.open = false;
        }
    }

    fn listing(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(Selected {
            used,
            mode: Mode::Merge { .. },
            ..
        }) = &self.selected
        {
            ui.label(format!("Choose a destination for “{}”", used.tag.name));
        }
        if ui
            .add(
                egui::TextEdit::singleline(&mut self.prefix)
                    .id_salt("manage-tags-search")
                    .char_limit(MAX_NAME_CHARS)
                    .desired_width(f32::INFINITY)
                    .hint_text("Find tags by name..."),
            )
            .changed()
        {
            self.first_page();
        }
        ui.add_space(4.0);
        let area = egui::ScrollArea::vertical()
            .id_salt("manage-tags-list")
            .max_height((ctx.content_rect().height() * 0.35).clamp(100.0, 280.0))
            .auto_shrink([false, true]);
        let area = if std::mem::take(&mut self.scroll_to_top) {
            area.vertical_scroll_offset(0.0)
        } else {
            area
        };
        area.show(ui, |ui| {
            if let Some(page) = &self.page {
                for used in &page.rows {
                    let merging = self
                        .selected
                        .as_ref()
                        .is_some_and(|item| matches!(item.mode, Mode::Merge { .. }));
                    let selected = self.selected.as_ref().is_some_and(|item| match &item.mode {
                        Mode::Merge { destination } => destination
                            .as_ref()
                            .is_some_and(|tag| tag.id == used.tag.id),
                        _ => item.used.tag.id == used.tag.id,
                    });
                    let source = merging
                        && self
                            .selected
                            .as_ref()
                            .is_some_and(|item| item.used.tag.id == used.tag.id);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !source,
                                egui::Button::selectable(selected, ())
                                    .left_text(&used.tag.name)
                                    .truncate()
                                    .min_size(egui::vec2(
                                        (ui.available_width() - 92.0).max(80.0),
                                        26.0,
                                    )),
                            )
                            .on_hover_text(&used.tag.name)
                            .clicked()
                        {
                            if let Some(Selected {
                                mode: Mode::Merge { destination },
                                ..
                            }) = &mut self.selected
                            {
                                *destination = Some(used.tag.clone());
                            } else {
                                self.selected = Some(Selected {
                                    used: used.clone(),
                                    name: used.tag.name.clone(),
                                    mode: Mode::Details,
                                });
                            }
                        }
                        ui.weak(format!(
                            "{} {}",
                            used.count,
                            if used.count == 1 { "sound" } else { "sounds" }
                        ));
                    });
                }
                if page.rows.is_empty() {
                    ui.weak("No matching tags");
                }
            } else if self.pending || self.needed {
                ui.weak("Loading tags...");
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.after.is_some(), egui::Button::new("First"))
                .clicked()
            {
                self.first_page();
            }
            if ui
                .add_enabled(!self.previous.is_empty(), egui::Button::new("Previous"))
                .clicked()
                && let Some(after) = self.previous.pop_back()
            {
                self.after = after;
                self.invalidate();
            }
            let next = self.page.as_ref().and_then(|page| page.next.clone());
            if ui
                .add_enabled(next.is_some(), egui::Button::new("Next"))
                .clicked()
            {
                if self.previous.len() == MAX_PREVIOUS_PAGES {
                    self.previous.pop_front();
                }
                self.previous.push_back(self.after.clone());
                self.after = next;
                self.invalidate();
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(Color32::from_rgb(245, 143, 140), error);
            if ui.button("Retry tag list").clicked() {
                self.invalidate();
            }
        }
    }

    fn details(&mut self, ui: &mut egui::Ui) -> Option<Mutation> {
        let mut mutation = None;
        let mut search_destinations = false;
        if let Some(item) = &mut self.selected {
            ui.label(RichText::new(&item.used.tag.name).strong());
            match &mut item.mode {
                Mode::Details => {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        let input = ui.add(
                            egui::TextEdit::singleline(&mut item.name)
                                .id_salt("manage-tag-name")
                                .char_limit(MAX_NAME_CHARS)
                                .desired_width(260.0),
                        );
                        let can_rename =
                            !item.name.trim().is_empty() && item.name.trim() != item.used.tag.name;
                        let enter = can_rename
                            && (input.has_focus() || input.lost_focus())
                            && ui.input_mut(|input| {
                                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                            });
                        if ui
                            .add_enabled(can_rename, egui::Button::new("Rename"))
                            .clicked()
                            || enter
                        {
                            mutation = Some(Mutation::Rename {
                                id: item.used.tag.id,
                                name: item.name.clone(),
                            });
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Merge into...").clicked() {
                            item.mode = Mode::Merge { destination: None };
                            search_destinations = true;
                        }
                        if ui.button("Delete...").clicked() {
                            item.mode = Mode::Delete;
                        }
                    });
                }
                Mode::Merge { destination } => {
                    if let Some(tag) = destination {
                        ui.label(format!(
                            "Merge “{}” into “{}”?",
                            item.used.tag.name, tag.name
                        ));
                        ui.weak("Combine all linked sounds, then remove the source tag. Duplicate links are kept once.");
                        if ui.button("Merge tags").clicked() {
                            mutation = Some(Mutation::Merge {
                                source: item.used.tag.id,
                                target: tag.id,
                            });
                        }
                    } else {
                        ui.weak("Select the tag to keep from the list above.");
                    }
                    if ui.button("Cancel merge").clicked() {
                        item.mode = Mode::Details;
                    }
                }
                Mode::Delete => {
                    ui.label(format!(
                        "Delete “{}” from {} {}?",
                        item.used.tag.name,
                        item.used.count,
                        if item.used.count == 1 {
                            "sound"
                        } else {
                            "sounds"
                        }
                    ));
                    ui.weak("This removes the tag everywhere. Audio files and other tags stay unchanged.");
                    ui.horizontal(|ui| {
                        if ui.button("Delete tag").clicked() {
                            mutation = Some(Mutation::Delete(item.used.tag.id));
                        }
                        if ui.button("Cancel delete").clicked() {
                            item.mode = Mode::Details;
                        }
                    });
                }
            }
        } else {
            ui.weak("Select a tag to rename, merge or delete it.");
        }
        if search_destinations {
            self.prefix.clear();
            self.first_page();
        }
        mutation
    }

    fn save_controls(&mut self, ui: &mut egui::Ui, worker: &Worker) {
        if let Some(save) = &mut self.save {
            if let Some(error) = &save.error {
                ui.colored_label(Color32::from_rgb(245, 143, 140), error);
                let mut back = false;
                ui.horizontal(|ui| {
                    if ui.button("Retry change").clicked() {
                        save.error = worker.manage(save.operation, save.mutation.clone()).err();
                    }
                    if ui.button("Back to editing").clicked() {
                        back = true;
                    }
                });
                if back {
                    self.save = None;
                }
            } else {
                ui.weak("Saving tag change...");
            }
        }
    }

    fn saving(&self) -> bool {
        self.save.as_ref().is_some_and(|save| save.error.is_none())
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/tag_manager.rs"
    ));
}
