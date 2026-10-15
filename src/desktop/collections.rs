use std::path::PathBuf;

use super::favorites::{Favorites, star_button};
use super::saved::{PendingSave, Storage};
use super::{
    SoundKey,
    navigation::{RetainedState, Views},
    selection::{self, Gesture, Selection, SoundDrag},
};
use eframe::egui::{self, Color32, RichText};

use crate::collections::{
    Availability, Collection, CollectionId, Entry, EntryId, PAGE_SIZE, Page, Reference,
    worker::{Change, Event, Mutation},
};

pub(super) struct Collections {
    selection: Selection,
    views: Views<CollectionId, CollectionView>,
    restore_preview: bool,
    previous_marks: Option<CollectionMarks>,
    pub(super) favorite_paths_dirty: bool,
    groups: Option<Page<Collection>>,
    groups_generation: u64,
    groups_offset: usize,
    groups_pending: bool,
    groups_failed: bool,
    entries: Option<Page<Entry>>,
    entries_total: usize,
    order: crate::search::SortOrder,
    entries_generation: u64,
    entries_offset: usize,
    entries_pending: bool,
    entries_failed: bool,
    active: Option<CollectionId>,
    error: Option<String>,
    save: Option<CollectionSave>,
    dialog: Option<Dialog>,
}

struct CollectionMarks {
    collection: CollectionId,
    sources: Result<crate::search::SourceSelection, String>,
    count: usize,
    order: crate::search::SortOrder,
}

struct CollectionView {
    selection: Selection,
    offset: usize,
    total: usize,
    order: crate::search::SortOrder,
}

impl RetainedState for CollectionView {
    fn retained_bytes(&self) -> usize {
        self.selection.retained_bytes()
    }
}

struct CollectionSave {
    request: PendingSave,
    removed_source: Option<PathBuf>,
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
    AddMarked(CollectionId),
}

impl Collections {
    pub(super) fn tag_selection_key(&self) -> (u64, u64, bool) {
        (
            self.selection.generation(),
            self.entries_generation,
            self.previous_marks.as_ref().map_or_else(
                || self.selection.snapshot_pending(),
                |marks| marks.sources.is_err(),
            ),
        )
    }
    pub(super) fn active_reference(&self) -> Option<(SoundKey, Reference)> {
        Some((
            self.selection.active()?,
            self.selection.reference()?.clone(),
        ))
    }

    pub(super) fn take_preview_restore(&mut self) -> Option<Option<(SoundKey, Reference)>> {
        if std::mem::take(&mut self.restore_preview) {
            Some(self.active_reference())
        } else {
            None
        }
    }

    pub(super) fn suspend_view(&mut self, storage: &Storage) {
        self.selection.suspend();
        storage
            .worker
            .invalidate_entry_selection(self.selection.generation());
    }

    pub(super) fn resume_view(&mut self, storage: &Storage) -> Option<(SoundKey, Reference)> {
        self.restore_preview = false;
        self.refresh_entries();
        self.request_pages(storage);
        self.active_reference()
    }
    pub(super) fn selected_count(&self) -> usize {
        self.selection.count(self.entries_total)
    }

    pub(super) fn marked_count(&self) -> usize {
        self.previous_marks
            .as_ref()
            .map_or_else(|| self.selected_count(), |marks| marks.count)
    }

    pub(super) fn selected_references(
        &self,
    ) -> Result<crate::collections::ReferenceSelection, String> {
        if let Some(marks) = &self.previous_marks {
            return Ok(crate::collections::ReferenceSelection {
                order: marks.order,
                origin: crate::collections::SelectionOrigin::Collection(marks.collection),
                sources: marks.sources.clone()?,
            });
        }
        let collection = self.active.ok_or("Choose a collection")?;
        Ok(crate::collections::ReferenceSelection {
            order: self.order,
            origin: crate::collections::SelectionOrigin::Collection(collection),
            sources: self.selection.snapshot(self.entries_total)?,
        })
    }

    pub(super) fn add_selected(
        &mut self,
        storage: &mut Storage,
        collection: CollectionId,
        selection: Result<crate::collections::ReferenceSelection, String>,
    ) {
        match selection {
            Ok(selection) if self.can_edit(storage) => {
                self.error = None;
                self.mutate(
                    storage,
                    Mutation::AddSelected {
                        collection,
                        selection,
                    },
                );
            }
            Err(error) => self.error = Some(error),
            _ => {}
        }
    }

    fn remove_selected(&mut self, storage: &mut Storage, collection: CollectionId) {
        let total = self.entries_total;
        match self.selection.snapshot(total) {
            Ok(selection) => {
                self.error = None;
                self.mutate(
                    storage,
                    Mutation::RemoveSelected {
                        collection,
                        selection,
                    },
                );
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn new() -> Self {
        Self {
            selection: Selection::default(),
            views: Views::default(),
            restore_preview: false,
            previous_marks: None,
            favorite_paths_dirty: true,
            groups: None,
            groups_generation: 0,
            groups_offset: 0,
            groups_pending: false,
            groups_failed: false,
            entries: None,
            entries_total: 0,
            order: crate::search::SortOrder::INSERTION,
            entries_generation: 0,
            entries_offset: 0,
            entries_pending: false,
            entries_failed: false,
            active: None,
            error: None,
            save: None,
            dialog: None,
        }
    }

    pub(super) fn settled(&self) -> bool {
        self.groups.is_some()
            && (self.active.is_none()
                || self
                    .entries
                    .as_ref()
                    .is_some_and(|page| page.offset == self.entries_offset))
            && !self.entries_pending
            && self.save.is_none()
    }

    pub(super) fn invalidate_selection(&self, storage: &Storage) {
        storage
            .worker
            .invalidate_entry_selection(self.selection.generation());
    }

    pub(super) fn storage_ready(&mut self) {
        self.refresh_groups();
    }

    pub(super) fn startup_failed(&mut self, error: &str) {
        eprintln!("Collection storage startup failed: {error}");
        self.error = Some(format!("Collections unavailable: {error}"));
    }

    pub(super) fn event(&mut self, storage: &Storage, event: Event) {
        match event {
            Event::Positions {
                generation,
                origin: crate::collections::SelectionOrigin::Collection(id),
                result,
            } if self.active == Some(id) => {
                if let Some(index) = self.selection.positions_resolved(generation, result) {
                    self.load_entry_page(storage, index / PAGE_SIZE * PAGE_SIZE);
                }
            }
            Event::EntrySelection { generation, result } => {
                self.selection.resolved(generation, result)
            }
            Event::Groups { generation, result } if generation == self.groups_generation => {
                self.groups_pending = false;
                match result {
                    Ok(page) => {
                        if page.total > 0 && page.offset >= page.total {
                            self.groups_offset = (page.total - 1) / PAGE_SIZE * PAGE_SIZE;
                            self.refresh_groups();
                            return;
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
                        self.entries_total = page.total;
                        for (position, entry) in page.rows.iter().enumerate() {
                            self.selection.observe(
                                SoundKey::Collection(entry.id),
                                page.offset + position,
                                &entry.reference.source,
                            );
                            if self.selection.active_source()
                                == Some(entry.reference.source.as_path())
                            {
                                self.selection.remember(entry.reference.clone());
                            }
                        }
                        if page.total > 0 && page.offset >= page.total {
                            self.entries_offset = (page.total - 1) / PAGE_SIZE * PAGE_SIZE;
                            self.refresh_entries();
                            return;
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
                    .is_some_and(|save| save.request.operation == operation) =>
            {
                match result {
                    Ok(change) => {
                        if let Some(save) = self.save.take()
                            && let Mutation::Remove { collection, .. } = save.request.mutation
                            && let Some(source) = save.removed_source
                        {
                            if self.active == Some(collection) {
                                self.selection.forget(&source);
                            } else if let Some(mut view) = self.views.take(&collection) {
                                view.selection.forget(&source);
                                view.total = view.total.saturating_sub(1);
                                self.views.remember(collection, view);
                            }
                        }
                        self.dialog = None;
                        match &change {
                            Change::Deleted(id) => {
                                drop(self.views.take(id));
                            }
                            Change::Removed {
                                collection,
                                sources,
                            } => {
                                if let Some(mut view) = self.views.take(collection) {
                                    for source in sources {
                                        view.selection.forget(source);
                                    }
                                    view.total = view.total.saturating_sub(sources.len());
                                    self.views.remember(*collection, view);
                                }
                            }
                            _ => {}
                        }
                        let removed_collection = match &change {
                            Change::Removed { collection, .. } | Change::Deleted(collection) => {
                                Some(*collection)
                            }
                            _ => None,
                        };
                        if self
                            .previous_marks
                            .as_ref()
                            .is_some_and(|marks| Some(marks.collection) == removed_collection)
                        {
                            self.previous_marks = None;
                        }
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
                            Change::Removed {
                                collection,
                                sources,
                            } if self.active == Some(collection) => {
                                for source in sources {
                                    self.selection.forget(&source);
                                }
                                self.refresh_entries();
                            }
                            Change::Deleted(id) if self.active == Some(id) => {
                                self.selection.reset();
                                self.favorite_paths_dirty = true;
                                self.active = None;
                                self.entries = None;
                                self.entries_total = 0;
                                self.restore_preview = true;
                                self.entries_generation += 1;
                            }
                            _ => {}
                        }
                        self.refresh_groups();
                    }
                    Err(error) => {
                        eprintln!("Collection edit failed: {error}");
                        if let Some(save) = &mut self.save {
                            save.request.error = Some(error);
                        }
                    }
                }
            }
            _ => {} // Stale pages/operations cannot replace this view.
        }
    }

    fn activate(&mut self, id: CollectionId) {
        if self.active == Some(id) {
            return;
        }
        // Choosing an add destination must not replace the source marks with
        // the first row of that destination. Keep one bounded prior snapshot
        // until the user makes a new selection in the current collection.
        if self.previous_marks.is_none()
            && self.selected_count() > 0
            && let Some(collection) = self.active
        {
            self.previous_marks = Some(CollectionMarks {
                collection,
                sources: self.selection.snapshot(self.entries_total),
                count: self.selected_count(),
                order: self.order,
            });
        }
        let previous_generation = self.selection.generation();
        self.selection.suspend();
        let restored = if let Some(previous) = self.active {
            self.views.switch(
                previous,
                CollectionView {
                    selection: std::mem::take(&mut self.selection),
                    offset: self.entries_offset,
                    total: self.entries_total,
                    order: self.order,
                },
                &id,
            )
        } else {
            self.views.take(&id)
        };
        self.restore_preview = true;
        if let Some(view) = restored {
            self.selection = view.selection;
            self.entries_offset = view.offset;
            self.entries_total = view.total;
            self.order = view.order;
        } else {
            self.selection = Selection::default();
            self.entries_offset = 0;
            self.entries_total = 0;
            self.order = crate::search::SortOrder::INSERTION;
        }
        self.selection.resume(previous_generation);
        self.active = Some(id);
        self.refresh_entries();
    }

    fn refresh_groups(&mut self) {
        self.groups_generation += 1;
        self.groups = None;
        self.groups_pending = false;
        self.groups_failed = false;
    }
    fn refresh_entries(&mut self) {
        self.selection.refresh();
        self.favorite_paths_dirty = true;
        self.entries_generation += 1;
        self.entries = None;
        self.entries_pending = false;
        self.entries_failed = false;
    }

    fn load_entry_page(&mut self, storage: &Storage, offset: usize) {
        if self.entries_offset != offset {
            self.selection.remember_scroll(0.0, offset);
        }
        self.entries_offset = offset;
        self.entries_generation += 1;
        self.entries_pending = false;
        self.entries_failed = false;
        self.request_pages(storage);
    }

    fn reorder(&mut self, storage: &Storage, order: crate::search::SortOrder) {
        self.order = order;
        self.selection.reorder();
        self.entries_offset = 0;
        self.entries_generation += 1;
        self.entries = None;
        self.entries_pending = false;
        self.entries_failed = false;
        self.request_pages(storage);
    }

    pub(super) fn request_pages(&mut self, storage: &Storage) {
        if storage.ready
            && let Some(id) = self.active
            && let Some(request) = self.selection.position_request()
            && storage.worker.positions(
                request.generation,
                crate::collections::SelectionOrigin::Collection(id),
                self.order,
                request.active,
                request.anchor,
            )
        {
            self.selection.position_sent();
        }
        if storage.ready && self.groups.is_none() && !self.groups_pending && !self.groups_failed {
            self.groups_pending = storage
                .worker
                .groups(self.groups_generation, self.groups_offset);
        }
        if storage.ready
            && self
                .entries
                .as_ref()
                .is_none_or(|page| page.offset != self.entries_offset)
            && !self.entries_pending
            && !self.entries_failed
            && let Some(id) = self.active
        {
            self.entries_pending = storage.worker.entries_ordered(
                self.entries_generation,
                id,
                self.order,
                self.entries_offset,
            );
        }
    }

    fn saving(&self) -> bool {
        self.save
            .as_ref()
            .is_some_and(|save| save.request.error.is_none())
    }
    fn can_edit(&self, storage: &Storage) -> bool {
        storage.ready && self.save.is_none()
    }

    fn mutate(&mut self, storage: &mut Storage, mutation: Mutation) {
        if self.saving() {
            return;
        }
        let removed_source = match &mutation {
            Mutation::Remove { entry, collection } if self.active == Some(*collection) => {
                self.entries.as_ref().and_then(|page| page.rows.iter().find(|row| row.id == *entry))
                    .map(|row| row.reference.source.clone())
                    .or_else(|| self.save.as_ref().filter(|save| matches!(&save.request.mutation, Mutation::Remove {entry: old_entry, collection: old_collection} if old_entry == entry && old_collection == collection)).and_then(|save| save.removed_source.clone()))
            }
            _ => None,
        };
        self.save = Some(CollectionSave {
            request: storage.submit(mutation),
            removed_source,
        });
    }

    pub(super) fn visible_sources(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.entries
            .iter()
            .flat_map(|page| page.rows.iter().map(|entry| entry.reference.source.clone()))
    }

    pub(super) fn sound_has_focus(&self, context: &egui::Context) -> bool {
        self.selection.has_focus(context)
    }

    pub(super) fn sources_changed(&mut self, paths: &[PathBuf]) {
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
        storage: &mut Storage,
        favorites: &mut Favorites,
        selected: Option<&Reference>,
        selected_entry: Option<EntryId>,
        marked_count: usize,
    ) -> Option<Action> {
        let mut action = None;
        self.request_pages(storage);
        let can_edit = self.can_edit(storage);
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
                        if can_edit
                            && (response.dnd_hover_payload::<Reference>().is_some()
                                || response.dnd_hover_payload::<SoundDrag>().is_some())
                        {
                            ui.painter().rect_stroke(
                                response.rect,
                                0.0,
                                (1.0, Color32::from_rgb(135, 222, 194)),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if can_edit && let Some(mutation) = dropped_sounds(&response, group.id) {
                            add = Some(mutation);
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
            ui.label(if storage.ready {
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
        if let Some(mutation) = add {
            self.mutate(storage, mutation);
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
            && let Some(mutation) = dropped_sounds(&response, collection)
        {
            self.mutate(storage, mutation);
        }
        if self.active.is_some() {
            let mut order = self.order;
            ui.horizontal(|ui| {
                ui.label("Sort");
                egui::ComboBox::from_id_salt("collection-sort")
                    .selected_text(sort_label(order.field))
                    .show_ui(ui, |ui| {
                        use crate::search::SortField;
                        for field in [
                            SortField::Added,
                            SortField::Filename,
                            SortField::Format,
                            SortField::Length,
                            SortField::Rate,
                            SortField::Channels,
                        ] {
                            if ui
                                .selectable_label(order.field == field, sort_label(field))
                                .clicked()
                            {
                                order.toggle(field);
                                ui.close();
                            }
                        }
                    });
                let reverse =
                    ui.add_sized([24.0, ui.spacing().interact_size.y], egui::Button::new(""));
                reverse.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        ui.is_enabled(),
                        "Reverse sort direction",
                    )
                });
                super::sound_table::paint_sort_direction(
                    ui.painter(),
                    reverse.rect.center(),
                    order.direction,
                    ui.visuals().text_color(),
                );
                if reverse.on_hover_text("Reverse sort direction").clicked() {
                    order.toggle(order.field);
                }
            });
            if order != self.order {
                self.reorder(storage, order);
                selection::focus(ui, ui.make_persistent_id("collection-sound-focus"));
                return None;
            }
        }
        let mut remove = None;
        let mut favorite = None;
        let focus_id = ui.make_persistent_id("collection-sound-focus");
        let total = self.entries.as_ref().map_or(0, |page| page.total);
        let selection_generation = self.selection.generation();
        let favorite_key = self.selection.keyboard(ui, focus_id, total);
        if favorite_key
            || self.selection.generation() != selection_generation
            || self.selection.target().is_some()
        {
            self.previous_marks = None;
        }
        self.selection.feedback(ui, total);
        let target = self.selection.target();
        let mut choice = target.and_then(|(index, gesture)| {
            self.entries
                .as_ref()
                .filter(|page| index >= page.offset && index < page.offset + page.rows.len())
                .and_then(|page| page.rows.get(index - page.offset))
                .map(|entry| (entry.clone(), index, gesture))
        });
        if let Some(collection) = self.active {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        can_edit && marked_count > 0,
                        egui::Button::new(format!("Add selected sounds ({marked_count})")),
                    )
                    .clicked()
                {
                    action = Some(Action::AddMarked(collection));
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        can_edit && self.selected_count() > 0,
                        egui::Button::new("Remove selected"),
                    )
                    .on_hover_text(
                        "Remove marked references from this collection; source files stay in place",
                    )
                    .clicked()
                {
                    self.remove_selected(storage, collection);
                }
                if ui
                    .button("Refresh")
                    .on_hover_text("Check saved sources again")
                    .clicked()
                {
                    self.refresh_entries();
                }
            });
            if let Some(page) = self
                .entries
                .as_ref()
                .filter(|page| page.offset == self.entries_offset)
            {
                if page.total == 0 {
                    ui.label("Drop sounds onto the collection above, or add the marked sounds.");
                }
                let height = (ui.available_height() - 86.0).max(80.0);
                let scroll = self
                    .selection
                    .scroll_area(
                        ui,
                        egui::ScrollArea::vertical()
                            .id_salt(("collection-entries", collection.0))
                            .max_height(height),
                        page.offset,
                        height.min(ui.available_height()),
                    )
                    .show_rows(ui, 28.0, page.rows.len(), |ui, range| {
                        let first_visible = page.offset + range.start;
                        for index in range {
                            let entry = &page.rows[index];
                            let key = SoundKey::Collection(entry.id);
                            let position = page.offset + index;
                            if selected_entry == Some(entry.id) {
                                self.selection.seed(key, position, &entry.reference.source);
                                if self.selection.reference().is_none() {
                                    self.selection.remember(entry.reference.clone());
                                }
                            }
                            let status = match &entry.availability {
                                Availability::Ready => "",
                                Availability::Changed => "Changed · ",
                                Availability::Missing => "Missing · ",
                                Availability::Unavailable(_) => "Unavailable · ",
                            };
                            ui.horizontal(|ui| {
                                let membership = favorites.status(&entry.reference.source);
                                if star_button(
                                    ui,
                                    membership,
                                    storage.ready
                                        && favorites.can_edit()
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
                                    .selected(
                                        self.selection.contains(&entry.reference.source, position),
                                    )
                                    .sense(egui::Sense::click_and_drag()),
                                );
                                selection::active_outline(
                                    ui,
                                    &response,
                                    self.selection.active_source()
                                        == Some(entry.reference.source.as_path()),
                                );
                                if response.clicked() {
                                    choice = Some((
                                        entry.clone(),
                                        position,
                                        Gesture::from_modifiers(ui.input(|input| input.modifiers)),
                                    ));
                                    selection::focus(ui, focus_id);
                                }
                                if entry.availability.usable()
                                    && response.drag_started_by(egui::PointerButton::Primary)
                                {
                                    if !self.selection.contains(&entry.reference.source, position) {
                                        self.selection.choose(
                                            key,
                                            position,
                                            &entry.reference.source,
                                            Gesture::Replace,
                                        );
                                    }
                                    self.selection
                                        .activate(key, position, &entry.reference.source);
                                    self.selection.remember(entry.reference.clone());
                                    self.previous_marks = None;
                                    selection::focus(ui, focus_id);
                                    choice = None;
                                    action = Some(Action::Drag(entry.clone()));
                                }
                                response.context_menu(|ui| {
                                    if ui
                                        .add_enabled(
                                            can_edit,
                                            egui::Button::new(
                                                if self
                                                    .selection
                                                    .contains(&entry.reference.source, position)
                                                    && self.selection.count(total) > 1
                                                {
                                                    "Remove selected sounds"
                                                } else {
                                                    "Remove from collection"
                                                },
                                            ),
                                        )
                                        .clicked()
                                    {
                                        remove = Some((
                                            entry.id,
                                            self.selection
                                                .contains(&entry.reference.source, position),
                                        ));
                                        ui.close();
                                    }
                                });
                                response
                                    .on_hover_text(entry.reference.source.display().to_string());
                            });
                        }
                        first_visible
                    });
                self.selection
                    .remember_scroll(scroll.state.offset.y, scroll.inner);
                let total = page.total;
                if let Some(offset) = pager(ui, self.entries_offset, total) {
                    self.load_entry_page(storage, offset);
                }
                ui.label(
                    RichText::new(format!("{total} saved sounds"))
                        .small()
                        .weak(),
                );
            } else {
                ui.label("Loading saved sounds...");
            }
            if let Some((entry, marked)) = remove {
                if marked {
                    self.remove_selected(storage, collection);
                } else {
                    self.mutate(storage, Mutation::Remove { collection, entry });
                }
            }
        } else {
            ui.label("Choose a collection.");
        }
        if let Some((entry, index, gesture)) = choice {
            self.previous_marks = None;
            if !self.selection.choose(
                SoundKey::Collection(entry.id),
                index,
                &entry.reference.source,
                gesture,
            ) {
                return None;
            }
            self.selection.remember(entry.reference.clone());
            ui.ctx().request_repaint();
            action = Some(Action::Select(entry));
        }
        if let Some((index, _)) = target
            && index / PAGE_SIZE * PAGE_SIZE != self.entries_offset
        {
            self.load_entry_page(storage, index / PAGE_SIZE * PAGE_SIZE);
        }
        if favorite_key && let Some(SoundKey::Collection(id)) = self.selection.active() {
            favorite = selected
                .filter(|reference| {
                    self.selection.active_source() == Some(reference.source.as_path())
                })
                .cloned()
                .or_else(|| self.selection.reference().cloned())
                .or_else(|| {
                    self.entries
                        .as_ref()
                        .and_then(|page| page.rows.iter().find(|entry| entry.id == id))
                        .map(|entry| entry.reference.clone())
                        .or_else(|| {
                            (selected_entry == Some(id))
                                .then_some(selected)
                                .flatten()
                                .cloned()
                        })
                });
        }
        if let Some(request) = self.selection.request()
            && let Some(collection) = self.active
            && storage.worker.entry_selection_ordered(
                request.generation,
                collection,
                self.order,
                request.anchor,
                request.target,
            )
        {
            self.selection.sent();
        }
        if let Some(reference) = favorite {
            favorites.toggle(storage, &reference);
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
            save.request
                .error
                .as_ref()
                .map(|error| (error.clone(), save.request.mutation.clone()))
        });
        if let Some((error, mutation)) = failed {
            ui.colored_label(
                Color32::from_rgb(245, 143, 140),
                format!("Not saved: {error}"),
            );
            ui.horizontal(|ui| {
                if ui.button("Retry save").clicked() {
                    self.mutate(storage, mutation);
                }
                if ui.button("Discard unsaved change").clicked() {
                    self.save = None;
                }
            });
        }
        self.dialog(ui.ctx(), storage);
        action
    }

    fn dialog(&mut self, ctx: &egui::Context, storage: &mut Storage) {
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
                                    .char_limit(crate::names::MAX_CHARS)
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
            self.mutate(storage, mutation);
        }
    }
}

fn dropped_sounds(response: &egui::Response, collection: CollectionId) -> Option<Mutation> {
    if egui::DragAndDrop::has_payload_of_type::<SoundDrag>(&response.ctx) {
        response
            .dnd_release_payload::<SoundDrag>()
            .map(|drag| Mutation::AddSelected {
                collection,
                selection: drag.selection.clone(),
            })
    } else if egui::DragAndDrop::has_payload_of_type::<Reference>(&response.ctx) {
        response
            .dnd_release_payload::<Reference>()
            .map(|reference| Mutation::Add {
                collection,
                references: vec![(*reference).clone()],
            })
    } else {
        None
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

fn sort_label(field: crate::search::SortField) -> &'static str {
    use crate::search::SortField;
    match field {
        SortField::Added => "Added order",
        SortField::Filename => "Filename",
        SortField::Format => "Format",
        SortField::Length => "Length",
        SortField::Rate => "Rate",
        SortField::Channels => "Channels",
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/collections.rs"
    ));
}
