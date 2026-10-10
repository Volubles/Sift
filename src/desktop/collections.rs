use std::{collections::BTreeSet, path::PathBuf};

use super::favorites::{Favorites, star_button};
use eframe::egui::{self, Color32, RichText};

use crate::collections::{
    Availability, Collection, CollectionId, Entry, EntryId, PAGE_SIZE, Page, Reference,
    worker::{Change, Event, Mutation, Worker},
};

pub(super) struct Collections {
    worker: Worker,
    pub(super) favorites: Favorites,
    pub(super) favorite_paths_dirty: bool,
    ready: bool,
    groups: Option<Page<Collection>>,
    groups_generation: u64,
    groups_offset: usize,
    groups_pending: bool,
    groups_failed: bool,
    entries: Option<Page<Entry>>,
    entries_generation: u64,
    entries_offset: usize,
    entries_pending: bool,
    entries_failed: bool,
    active: Option<CollectionId>,
    error: Option<String>,
    save: Option<PendingSave>,
    operation: u64,
    dialog: Option<Dialog>,
}

struct PendingSave {
    operation: u64,
    mutation: Mutation,
    error: Option<String>,
}

enum Dialog {
    Name {
        collection: Option<CollectionId>,
        text: String,
        focus: bool,
    },
    Delete {
        collection: CollectionId,
        name: String,
    },
}

pub(super) enum Action {
    Select(Entry),
    Drag(Entry),
}

impl Collections {
    pub(super) fn new(worker: Worker) -> Self {
        Self {
            worker,
            favorites: Favorites::default(),
            favorite_paths_dirty: true,
            ready: false,
            groups: None,
            groups_generation: 0,
            groups_offset: 0,
            groups_pending: false,
            groups_failed: false,
            entries: None,
            entries_generation: 0,
            entries_offset: 0,
            entries_pending: false,
            entries_failed: false,
            active: None,
            error: None,
            save: None,
            operation: 0,
            dialog: None,
        }
    }

    pub(super) fn settled(&self) -> bool {
        self.ready
            && self.groups.is_some()
            && (self.active.is_none() || self.entries.is_some())
            && self.save.is_none()
    }

    pub(super) fn poll(&mut self) {
        while let Some(event) = self.worker.poll() {
            match event {
                Event::Ready => {
                    self.ready = true;
                    self.refresh_groups();
                }
                Event::Favorites { generation, result } => self.favorites.page(generation, result),
                Event::FavoriteStatus { generation, result } => {
                    self.favorites.membership(generation, result)
                }
                Event::Saved { operation, result } if self.favorites.owns_save(operation) => {
                    self.favorites.saved(result)
                }
                Event::StartupFailed(error) => {
                    self.favorites.startup_failed(&error);
                    eprintln!("Collection storage startup failed: {error}");
                    self.error = Some(format!("Collections unavailable: {error}"));
                }
                Event::Groups { generation, result } if generation == self.groups_generation => {
                    self.groups_pending = false;
                    match result {
                        Ok(page) => {
                            if page.total > 0 && page.offset >= page.total {
                                self.groups_offset = (page.total - 1) / PAGE_SIZE * PAGE_SIZE;
                                self.refresh_groups();
                                continue;
                            }
                            if self.active.is_none()
                                && let Some(collection) = page.rows.first()
                            {
                                self.activate(collection.id);
                            }
                            self.groups = Some(page);
                        }
                        Err(error) => {
                            self.groups_failed = true;
                            self.error = Some(format!("Could not read collections: {error}"));
                        }
                    }
                }
                Event::Entries {
                    generation,
                    collection,
                    result,
                } if generation == self.entries_generation && self.active == Some(collection) => {
                    self.entries_pending = false;
                    match result {
                        Ok(page) => {
                            if page.total > 0 && page.offset >= page.total {
                                self.entries_offset = (page.total - 1) / PAGE_SIZE * PAGE_SIZE;
                                self.refresh_entries();
                                continue;
                            }
                            self.entries = Some(page);
                            self.favorite_paths_dirty = true;
                            self.error = None;
                        }
                        Err(error) => {
                            self.entries_failed = true;
                            self.error = Some(format!("Could not read collection: {error}"));
                        }
                    }
                }
                Event::Saved { operation, result }
                    if self
                        .save
                        .as_ref()
                        .is_some_and(|save| save.operation == operation) =>
                {
                    match result {
                        Ok(change) => {
                            self.save = None;
                            self.dialog = None;
                            match change {
                                Change::Created(id) => {
                                    self.activate(id);
                                    self.groups_offset = self
                                        .groups
                                        .as_ref()
                                        .map(|page| page.total / PAGE_SIZE * PAGE_SIZE)
                                        .unwrap_or(0);
                                }
                                Change::Updated(id) if self.active == Some(id) => {
                                    self.refresh_entries()
                                }
                                Change::Deleted(id) if self.active == Some(id) => {
                                    self.favorite_paths_dirty = true;
                                    self.active = None;
                                    self.entries = None;
                                    self.entries_generation += 1;
                                }
                                _ => {}
                            }
                            self.refresh_groups();
                        }
                        Err(error) => {
                            eprintln!("Collection edit failed: {error}");
                            if let Some(save) = &mut self.save {
                                save.error = Some(error);
                            }
                        }
                    }
                }
                _ => {} // stale pages/operations cannot replace the active selection
            }
        }
        self.request_pages();
        self.favorites.request(&self.worker, self.ready);
    }

    fn activate(&mut self, id: CollectionId) {
        self.active = Some(id);
        self.entries_offset = 0;
        self.refresh_entries();
    }

    fn refresh_groups(&mut self) {
        self.groups_generation += 1;
        self.groups = None;
        self.groups_pending = false;
        self.groups_failed = false;
    }
    fn refresh_entries(&mut self) {
        self.favorite_paths_dirty = true;
        self.entries_generation += 1;
        self.entries = None;
        self.entries_pending = false;
        self.entries_failed = false;
    }

    fn request_pages(&mut self) {
        if self.ready && self.groups.is_none() && !self.groups_pending && !self.groups_failed {
            self.groups_pending = self
                .worker
                .groups(self.groups_generation, self.groups_offset);
        }
        if self.ready
            && self.entries.is_none()
            && !self.entries_pending
            && !self.entries_failed
            && let Some(id) = self.active
        {
            self.entries_pending =
                self.worker
                    .entries(self.entries_generation, id, self.entries_offset);
        }
    }

    fn saving(&self) -> bool {
        self.save.as_ref().is_some_and(|save| save.error.is_none())
    }
    fn can_edit(&self) -> bool {
        self.ready && self.save.is_none()
    }

    fn mutate(&mut self, mutation: Mutation) {
        if self.saving() {
            return;
        }
        self.operation += 1;
        let error = self.worker.mutate(self.operation, mutation.clone()).err();
        self.save = Some(PendingSave {
            operation: self.operation,
            mutation,
            error,
        });
    }

    pub(super) fn track_favorites(&mut self, mut paths: BTreeSet<PathBuf>) {
        self.favorite_paths_dirty = false;
        if let Some(page) = &self.entries {
            paths.extend(page.rows.iter().map(|entry| entry.reference.source.clone()));
        }
        self.favorites.track(paths);
        self.favorites.request(&self.worker, self.ready);
    }

    pub(super) fn toggle_favorite(&mut self, reference: &Reference) {
        if self.ready {
            self.favorites
                .toggle(&self.worker, &mut self.operation, reference);
        }
    }

    pub(super) fn favorite_feedback(&mut self, ui: &mut egui::Ui) {
        self.favorites
            .feedback(ui, &self.worker, &mut self.operation);
    }

    pub(super) fn favorite_list(
        &mut self,
        ui: &mut egui::Ui,
        selected: Option<crate::collections::favorites::FavoriteId>,
    ) -> Option<super::favorites::Action> {
        self.favorites
            .list(ui, &self.worker, &mut self.operation, self.ready, selected)
    }

    pub(super) fn sources_changed(&mut self, paths: &[PathBuf]) {
        self.favorites.sources_changed(paths);
        if self.entries.as_ref().is_some_and(|page| {
            page.rows.iter().any(|entry| {
                paths
                    .iter()
                    .any(|path| entry.reference.source.starts_with(path))
            })
        }) {
            self.refresh_entries();
        }
    }

    pub(super) fn selected_update(&self, id: EntryId) -> Option<&Entry> {
        self.entries
            .as_ref()?
            .rows
            .iter()
            .find(|entry| entry.id == id)
    }

    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        selected: Option<&Reference>,
        selected_entry: Option<EntryId>,
    ) -> Option<Action> {
        self.request_pages();
        let can_edit = self.can_edit();
        ui.horizontal(|ui| {
            ui.heading("Collections");
            if ui
                .add_enabled(can_edit, egui::Button::new("+"))
                .on_hover_text("New collection")
                .clicked()
            {
                self.dialog = Some(Dialog::Name {
                    collection: None,
                    text: String::new(),
                    focus: true,
                });
            }
        });
        ui.separator();
        let mut activate = None;
        let mut add = None;
        let mut dialog = None;
        if let Some(page) = &self.groups {
            if page.total == 0 {
                ui.label("Create a collection to save sounds.");
            }
            egui::ScrollArea::vertical()
                .id_salt("collection-groups")
                .max_height(180.0)
                .auto_shrink([false, false])
                .show_rows(ui, 28.0, page.rows.len(), |ui, range| {
                    for index in range {
                        let group = &page.rows[index];
                        let response = ui.add_sized(
                            [ui.available_width(), 28.0],
                            egui::Button::new(format!("{}  ({})", group.name, group.count))
                                .truncate()
                                .selected(self.active == Some(group.id)),
                        );
                        if response.clicked() {
                            activate = Some(group.id);
                        }
                        if can_edit && response.dnd_hover_payload::<Reference>().is_some() {
                            ui.painter().rect_stroke(
                                response.rect,
                                0.0,
                                (1.0, Color32::from_rgb(135, 222, 194)),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if can_edit
                            && let Some(reference) = response.dnd_release_payload::<Reference>()
                        {
                            add = Some((group.id, (*reference).clone()));
                        }
                        response.context_menu(|ui| {
                            if ui
                                .add_enabled(can_edit, egui::Button::new("Rename"))
                                .clicked()
                            {
                                dialog = Some(Dialog::Name {
                                    collection: Some(group.id),
                                    text: group.name.clone(),
                                    focus: true,
                                });
                                ui.close();
                            }
                            if ui
                                .add_enabled(can_edit, egui::Button::new("Delete collection..."))
                                .clicked()
                            {
                                dialog = Some(Dialog::Delete {
                                    collection: group.id,
                                    name: group.name.clone(),
                                });
                                ui.close();
                            }
                        });
                    }
                });
            let total = page.total;
            if let Some(offset) = pager(ui, self.groups_offset, total) {
                self.groups_offset = offset;
                self.refresh_groups();
            }
        } else {
            ui.label(if self.ready {
                "Loading collections..."
            } else {
                "Opening collections..."
            });
        }
        if let Some(id) = activate {
            self.activate(id);
        }
        if let Some(dialog) = dialog {
            self.dialog = Some(dialog);
        }
        if let Some((collection, reference)) = add {
            self.mutate(Mutation::Add {
                collection,
                references: vec![reference],
            });
        }
        ui.add_space(10.0);
        ui.separator();
        let active_name = self
            .groups
            .as_ref()
            .and_then(|page| page.rows.iter().find(|group| Some(group.id) == self.active))
            .map(|group| group.name.as_str())
            .unwrap_or("Collection contents");
        let response = ui.heading(active_name);
        if can_edit
            && let Some(collection) = self.active
            && let Some(reference) = response.dnd_release_payload::<Reference>()
        {
            self.mutate(Mutation::Add {
                collection,
                references: vec![(*reference).clone()],
            });
        }
        let mut action = None;
        let mut remove = None;
        let mut favorite = None;
        if let Some(collection) = self.active {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        can_edit && selected.is_some(),
                        egui::Button::new("Add selected sound"),
                    )
                    .clicked()
                    && let Some(reference) = selected
                {
                    self.mutate(Mutation::Add {
                        collection,
                        references: vec![reference.clone()],
                    });
                }
                if ui
                    .button("Refresh")
                    .on_hover_text("Check saved sources again")
                    .clicked()
                {
                    self.refresh_entries();
                }
            });
            if let Some(page) = &self.entries {
                if page.total == 0 {
                    ui.label("Drag a sound onto the collection above, or add the selected sound.");
                }
                let height = (ui.available_height() - 86.0).max(80.0);
                egui::ScrollArea::vertical()
                    .id_salt(("collection-entries", collection.0))
                    .max_height(height)
                    .show_rows(ui, 28.0, page.rows.len(), |ui, range| {
                        for index in range {
                            let entry = &page.rows[index];
                            let status = match &entry.availability {
                                Availability::Ready => "",
                                Availability::Changed => "Changed · ",
                                Availability::Missing => "Missing · ",
                                Availability::Unavailable(_) => "Unavailable · ",
                            };
                            ui.horizontal(|ui| {
                                let membership = self.favorites.status(&entry.reference.source);
                                if star_button(
                                    ui,
                                    membership,
                                    self.ready
                                        && self.favorites.can_edit()
                                        && (membership == Some(true)
                                            || entry.availability.usable()),
                                    &entry.reference.name,
                                )
                                .clicked()
                                {
                                    favorite = Some(entry.reference.clone());
                                }
                                let response = ui.add_sized(
                                    [ui.available_width(), 28.0],
                                    egui::Button::new(format!(
                                        "{}{}",
                                        status, entry.reference.name
                                    ))
                                    .truncate()
                                    .selected(selected_entry == Some(entry.id))
                                    .sense(egui::Sense::click_and_drag()),
                                );
                                if response.clicked() {
                                    action = Some(Action::Select(entry.clone()));
                                }
                                if entry.availability.usable()
                                    && response.drag_started_by(egui::PointerButton::Primary)
                                {
                                    action = Some(Action::Drag(entry.clone()));
                                }
                                response.context_menu(|ui| {
                                    if ui
                                        .add_enabled(
                                            can_edit,
                                            egui::Button::new("Remove from collection"),
                                        )
                                        .clicked()
                                    {
                                        remove = Some(entry.id);
                                        ui.close();
                                    }
                                });
                                response
                                    .on_hover_text(entry.reference.source.display().to_string());
                            });
                        }
                    });
                let total = page.total;
                if let Some(offset) = pager(ui, self.entries_offset, total) {
                    self.entries_offset = offset;
                    self.refresh_entries();
                }
                ui.label(
                    RichText::new(format!("{total} saved sounds"))
                        .small()
                        .weak(),
                );
            } else {
                ui.label("Loading saved sounds...");
            }
            if let Some(entry) = remove {
                self.mutate(Mutation::Remove { collection, entry });
            }
        } else {
            ui.label("Choose a collection.");
        }
        if let Some(reference) = favorite {
            self.toggle_favorite(&reference);
        }
        if let Some(error) = &self.error {
            ui.colored_label(Color32::from_rgb(245, 143, 140), error);
        }
        if (self.groups_failed || self.entries_failed) && ui.button("Retry loading").clicked() {
            if self.groups_failed {
                self.refresh_groups();
            }
            if self.entries_failed {
                self.refresh_entries();
            }
        }
        if self.saving() {
            ui.label(RichText::new("Saving...").small().weak());
        }
        let failed = self.save.as_ref().and_then(|save| {
            save.error
                .as_ref()
                .map(|error| (error.clone(), save.mutation.clone()))
        });
        if let Some((error, mutation)) = failed {
            ui.colored_label(
                Color32::from_rgb(245, 143, 140),
                format!("Not saved: {error}"),
            );
            ui.horizontal(|ui| {
                if ui.button("Retry save").clicked() {
                    self.mutate(mutation);
                }
                if ui.button("Discard unsaved change").clicked() {
                    self.save = None;
                }
            });
        }
        self.dialog(ui.ctx());
        action
    }

    fn dialog(&mut self, ctx: &egui::Context) {
        let saving = self.saving();
        let mut submit = None;
        let mut close = false;
        if let Some(dialog) = &mut self.dialog {
            let mut open = true;
            egui::Window::new("Collection")
                .id(egui::Id::new("collection-dialog"))
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    match dialog {
                        Dialog::Name {
                            collection,
                            text,
                            focus,
                        } => {
                            ui.label(if collection.is_some() {
                                "Rename collection"
                            } else {
                                "New collection"
                            });
                            let response = ui.add_enabled(
                                !saving,
                                egui::TextEdit::singleline(text)
                                    .char_limit(120)
                                    .hint_text("Collection name"),
                            );
                            let just_opened = *focus;
                            if just_opened {
                                response.request_focus();
                                *focus = false;
                            }
                            let enter = !just_opened
                                && (response.has_focus() || response.lost_focus())
                                && ui.input(|input| input.key_pressed(egui::Key::Enter));
                            if !saving
                                && (enter
                                    || ui
                                        .button(if collection.is_some() {
                                            "Rename"
                                        } else {
                                            "Create"
                                        })
                                        .clicked())
                            {
                                submit = Some(match *collection {
                                    Some(collection) => Mutation::Rename {
                                        collection,
                                        name: text.clone(),
                                    },
                                    None => Mutation::Create(text.clone()),
                                });
                            }
                        }
                        Dialog::Delete { collection, name } => {
                            ui.label(format!(
                                "Delete collection '{name}' and its saved references?"
                            ));
                            ui.label("Original audio stays in its source folders.");
                            if ui
                                .add_enabled(!saving, egui::Button::new("Delete collection"))
                                .clicked()
                            {
                                submit = Some(Mutation::Delete(*collection));
                            }
                        }
                    }
                    if ui
                        .add_enabled(!saving, egui::Button::new("Cancel"))
                        .clicked()
                    {
                        close = true;
                    }
                });
            close |= !open;
        }
        if close {
            self.dialog = None;
        }
        if let Some(mutation) = submit {
            self.mutate(mutation);
        }
    }
}

fn pager(ui: &mut egui::Ui, offset: usize, total: usize) -> Option<usize> {
    if total <= PAGE_SIZE {
        return None;
    }
    ui.horizontal(|ui| {
        let mut next = None;
        if ui
            .add_enabled(offset > 0, egui::Button::new("Previous"))
            .clicked()
        {
            next = Some(offset.saturating_sub(PAGE_SIZE));
        }
        ui.label(format!(
            "{}–{} / {}",
            offset + 1,
            (offset + PAGE_SIZE).min(total),
            total
        ));
        if ui
            .add_enabled(offset + PAGE_SIZE < total, egui::Button::new("Next"))
            .clicked()
        {
            next = Some(offset + PAGE_SIZE);
        }
        next
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn settle(panel: &mut Collections, signals: &mpsc::Receiver<()>) {
        for _ in 0..32 {
            panel.poll();
            panel.request_pages();
            if panel.ready
                && panel.groups.is_some()
                && (panel.active.is_none() || panel.entries.is_some())
                && !panel.saving()
                && panel.favorites.settled()
            {
                return;
            }
            signals.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        panic!("Collections did not settle after bounded notifications");
    }

    #[test]
    fn favorite_edits_share_status_with_collections_and_keep_offline_references() {
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
        let (wake, signals) = mpsc::sync_channel(32);
        let mut panel =
            Collections::new(Worker::start(path.clone(), move || wake.send(()).unwrap()).unwrap());
        settle(&mut panel, &signals);
        panel.mutate(Mutation::Create("Film".into()));
        settle(&mut panel, &signals);
        panel.mutate(Mutation::Add {
            collection: panel.active.unwrap(),
            references: vec![reference.clone()],
        });
        settle(&mut panel, &signals);
        panel.track_favorites([source.clone()].into_iter().collect());
        settle(&mut panel, &signals);
        assert_eq!(panel.favorites.status(&source), Some(false));
        panel.toggle_favorite(&reference);
        assert_eq!(panel.favorites.status(&source), Some(false));
        settle(&mut panel, &signals);
        assert_eq!(
            panel
                .favorites
                .status(&panel.entries.as_ref().unwrap().rows[0].reference.source),
            Some(true)
        );
        panel.favorites.set_active(true);
        settle(&mut panel, &signals);
        assert_eq!(panel.favorites.total, 1);
        std::fs::remove_file(&source).unwrap();
        panel.sources_changed(std::slice::from_ref(&source));
        settle(&mut panel, &signals);
        assert_eq!(
            panel.entries.as_ref().unwrap().rows[0].availability,
            Availability::Missing
        );
        panel.toggle_favorite(&reference);
        settle(&mut panel, &signals);
        assert_eq!(panel.favorites.total, 0);
        assert_eq!(panel.favorites.status(&source), Some(false));
        assert_eq!(panel.entries.as_ref().unwrap().total, 1);
        drop(panel);
        let mut store = crate::collections::Store::open(&path).unwrap();
        assert_eq!(
            store
                .favorites(&crate::search::SearchQuery::All, 0)
                .unwrap()
                .total,
            0
        );
        assert_eq!(store.list(0).unwrap().rows[0].count, 1);
    }

    #[test]
    fn enter_that_opens_the_name_dialog_does_not_submit_it() {
        let directory = tempfile::tempdir().unwrap();
        let (wake, signals) = mpsc::sync_channel(32);
        let mut panel = Collections::new(
            Worker::start(directory.path().join("collections.sqlite"), move || {
                wake.send(()).unwrap()
            })
            .unwrap(),
        );
        settle(&mut panel, &signals);
        panel.dialog = Some(Dialog::Name {
            collection: None,
            text: "Keyboard collection".into(),
            focus: true,
        });
        let ctx = egui::Context::default();
        let key = |pressed| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut first = ctx.run_ui(
            egui::RawInput {
                events: vec![key(true)],
                ..Default::default()
            },
            |ui| panel.dialog(ui.ctx()),
        );
        first.textures_delta.clear();
        assert!(panel.save.is_none());
        let mut second = ctx.run_ui(
            egui::RawInput {
                events: vec![key(false), key(true)],
                ..Default::default()
            },
            |ui| panel.dialog(ui.ctx()),
        );
        second.textures_delta.clear();
        assert!(panel.saving());
        settle(&mut panel, &signals);
        assert_eq!(
            panel.groups.as_ref().unwrap().rows[0].name,
            "Keyboard collection"
        );
    }

    #[test]
    fn confirmed_ui_edits_reload_pages_and_persist_without_touching_sources() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("collections.sqlite");
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
        let (wake, signals) = mpsc::sync_channel(32);
        let mut panel =
            Collections::new(Worker::start(path.clone(), move || wake.send(()).unwrap()).unwrap());
        settle(&mut panel, &signals);
        panel.mutate(Mutation::Create("Song 1".into()));
        assert!(panel.saving());
        settle(&mut panel, &signals);
        let collection = panel.active.unwrap();
        panel.mutate(Mutation::Add {
            collection,
            references: vec![reference],
        });
        settle(&mut panel, &signals);
        assert_eq!(panel.groups.as_ref().unwrap().rows[0].count, 1);
        let entry = panel.entries.as_ref().unwrap().rows[0].id;
        panel.mutate(Mutation::Rename {
            collection,
            name: "Film FX".into(),
        });
        settle(&mut panel, &signals);
        assert_eq!(panel.active, Some(collection));
        assert_eq!(panel.groups.as_ref().unwrap().rows[0].name, "Film FX");
        panel.mutate(Mutation::Remove { collection, entry });
        settle(&mut panel, &signals);
        assert_eq!(panel.entries.as_ref().unwrap().total, 0);
        drop(panel);
        assert_eq!(
            crate::collections::Store::open(&path)
                .unwrap()
                .list(0)
                .unwrap()
                .rows[0]
                .name,
            "Film FX"
        );
        assert_eq!(std::fs::read(source).unwrap(), b"fixture");
    }

    #[test]
    fn failed_name_edit_keeps_the_draft_until_a_corrected_save_succeeds() {
        let directory = tempfile::tempdir().unwrap();
        let (wake, signals) = mpsc::sync_channel(32);
        let mut panel = Collections::new(
            Worker::start(directory.path().join("collections.sqlite"), move || {
                wake.send(()).unwrap()
            })
            .unwrap(),
        );
        settle(&mut panel, &signals);
        panel.dialog = Some(Dialog::Name {
            collection: None,
            text: "Bad\nName".into(),
            focus: false,
        });
        panel.mutate(Mutation::Create("Bad\nName".into()));
        settle(&mut panel, &signals);
        assert!(panel.save.as_ref().unwrap().error.is_some());
        assert!(matches!(&panel.dialog, Some(Dialog::Name { text, .. }) if text == "Bad\nName"));
        panel.mutate(Mutation::Create("Corrected name".into()));
        settle(&mut panel, &signals);
        assert!(panel.save.is_none());
        assert!(panel.dialog.is_none());
        assert_eq!(
            panel.groups.as_ref().unwrap().rows[0].name,
            "Corrected name"
        );
    }
}
