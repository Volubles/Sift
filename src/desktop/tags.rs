use super::Sidebar;
use crate::tags::worker::{Edit, Event, Target, Worker};
use crate::tags::{Filter, MAX_NAME_CHARS, Membership, Summary, Tag, TagId};
use eframe::egui::{self, Color32, RichText};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod manager;

pub(super) const BUTTON_WIDTH: f32 = 88.0;
const PANEL_PAGE: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Browse {
    CurrentLibrary,
    AllLibraries,
}
pub(super) enum Action {
    Toggle(Tag, Browse),
    RemoveFilter(TagId),
    ClearFilters,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum TargetKey {
    None,
    Sound(Vec<u8>),
    Marks {
        view: Sidebar,
        selection_generation: u64,
        view_generation: u64,
        pending: bool,
        count: usize,
        preview: Option<Vec<u8>>,
    },
}

#[derive(Default)]
struct Memberships {
    observed: BTreeMap<Vec<u8>, PathBuf>,
    desired: BTreeMap<Vec<u8>, PathBuf>,
    cache: BTreeMap<Vec<u8>, Vec<Tag>>,
    filter: Filter,
    filter_tags: Vec<Tag>,
    generation: u64,
    pending: bool,
    needed: bool,
    error: Option<String>,
}
impl Memberships {
    fn invalidate(&mut self) {
        self.generation += 1;
        self.pending = false;
        self.needed = true;
        self.cache.clear();
    }
    fn accept(&mut self, generation: u64, result: Result<Membership, String>) {
        if generation != self.generation {
            return;
        }
        self.pending = false;
        match result {
            Ok(values) => {
                self.cache = values
                    .sources
                    .into_iter()
                    .map(|row| (crate::path_codec::encode(&row.source), row.tags))
                    .collect();
                self.filter_tags = values.filter_tags;
                self.error = None;
            }
            Err(error) => {
                eprintln!("Tag membership read failed: {error}");
                self.error = Some(error);
            }
        }
    }
    fn request(&mut self, worker: &Worker) {
        let observed = std::mem::take(&mut self.observed);
        if observed.keys().ne(self.desired.keys()) {
            self.desired = observed;
            self.cache.retain(|key, _| self.desired.contains_key(key));
            self.generation += 1;
            self.pending = false;
            self.needed = true;
        }
        if self.needed
            && !self.pending
            && worker.status(
                self.generation,
                self.desired.values().cloned().collect(),
                self.filter.clone(),
            )
        {
            self.pending = true;
            self.needed = false;
        }
    }
}

#[derive(Default)]
struct Suggestions {
    prefix: String,
    generation: u64,
    pending: bool,
    needed: bool,
    items: Vec<Tag>,
    error: Option<String>,
}
impl Suggestions {
    fn search(&mut self, prefix: &str) {
        if self.prefix == prefix {
            return;
        }
        self.prefix = prefix.to_owned();
        self.generation += 1;
        self.pending = false;
        self.needed = true;
        self.items.clear();
        self.error = None;
    }
    fn request(&mut self, worker: &Worker) {
        if self.needed && !self.pending && worker.suggestions(self.generation, self.prefix.clone())
        {
            self.pending = true;
            self.needed = false;
        }
    }
}

struct PendingEdit {
    operation: u64,
    target: Target,
    edit: Edit,
    error: Option<String>,
}
struct Editor {
    key: TargetKey,
    target: Option<Target>,
    caption: String,
    browse: Browse,
    input: String,
    generation: u64,
    pending: bool,
    needed: bool,
    summary: Option<Summary>,
    error: Option<String>,
    page: usize,
    operation: u64,
    save: Option<PendingEdit>,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            key: TargetKey::None,
            target: None,
            caption: "Select a sound to edit tags".into(),
            browse: Browse::CurrentLibrary,
            input: String::new(),
            generation: 0,
            pending: false,
            needed: false,
            summary: None,
            error: None,
            page: 0,
            operation: 0,
            save: None,
        }
    }
}
impl Editor {
    fn invalidate(&mut self) {
        self.generation += 1;
        self.pending = false;
        self.needed = self.target.is_some();
        self.summary = None;
    }
    fn request(&mut self, worker: &Worker) {
        if self.needed
            && !self.pending
            && let Some(target) = &self.target
            && worker.summary(self.generation, target.clone())
        {
            self.pending = true;
            self.needed = false;
        }
    }
    fn submit(&mut self, worker: &Worker, edit: Edit) {
        if self.save.is_some() {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        self.operation += 1;
        let error = worker
            .edit(self.operation, target.clone(), edit.clone())
            .err();
        self.save = Some(PendingEdit {
            operation: self.operation,
            target,
            edit,
            error,
        });
    }
}

/// Tag input, source membership and their independent reply generations stay here.
pub(super) struct Tags {
    worker: Worker,
    ready: bool,
    startup_error: Option<String>,
    membership: Memberships,
    suggestions: Suggestions,
    editor: Editor,
    filter_text: String,
    filtering: bool,
    action: Option<Action>,
    manager: manager::Manager,
    management_change: Option<crate::tags::Change>,
}
impl Tags {
    pub(super) fn new(worker: Worker) -> Self {
        Self {
            worker,
            ready: false,
            startup_error: None,
            membership: Memberships::default(),
            suggestions: Suggestions {
                needed: true,
                ..Default::default()
            },
            editor: Editor::default(),
            filter_text: String::new(),
            filtering: false,
            action: None,
            manager: manager::Manager::default(),
            management_change: None,
        }
    }
    pub(super) fn begin_frame(&mut self) {
        self.filtering = false;
    }
    pub(super) fn settled(&self) -> bool {
        self.ready
            && !self.membership.pending
            && !self.membership.needed
            && !self.editor.pending
            && !self.editor.needed
            && !self.suggestions.pending
            && !self.suggestions.needed
            && self.manager.settled()
    }
    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Some(event) = self.worker.poll() {
            match event {
                Event::ManagementPage { generation, result } => {
                    self.manager.page(generation, result)
                }
                Event::Managed { operation, result } => {
                    if let Some(change) = self.manager.saved(operation, result) {
                        self.invalidate();
                        self.action = None;
                        self.management_change = Some(change);
                    }
                }
                Event::Ready => self.ready = true,
                Event::StartupFailed(error) => {
                    eprintln!("Tag storage startup failed: {error}");
                    self.startup_error = Some(error);
                }
                Event::Status { generation, result } => self.membership.accept(generation, result),
                Event::Suggestions { generation, result }
                    if generation == self.suggestions.generation =>
                {
                    self.suggestions.pending = false;
                    match result {
                        Ok(items) => {
                            self.suggestions.items = items;
                            self.suggestions.error = None;
                        }
                        Err(error) => {
                            eprintln!("Tag suggestions failed: {error}");
                            self.suggestions.error = Some(error);
                        }
                    }
                }
                Event::Summary { generation, result } if generation == self.editor.generation => {
                    self.editor.pending = false;
                    match result {
                        Ok(summary) => {
                            self.editor.summary = Some(summary);
                            self.editor.error = None;
                        }
                        Err(error) => {
                            self.editor.error = Some(error);
                        }
                    }
                }
                Event::Saved { operation, result }
                    if self
                        .editor
                        .save
                        .as_ref()
                        .is_some_and(|save| save.operation == operation) =>
                {
                    match result {
                        Ok(_) => {
                            self.editor.save = None;
                            self.editor.input.clear();
                            self.invalidate();
                            changed = true;
                        }
                        Err(error) => {
                            if let Some(save) = &mut self.editor.save {
                                save.error = Some(error);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        changed
    }
    fn invalidate(&mut self) {
        self.editor.page = 0;
        self.editor.invalidate();
        self.membership.filter_tags.clear();
        self.membership.invalidate();
        self.suggestions.generation += 1;
        self.suggestions.pending = false;
        self.suggestions.needed = true;
        self.suggestions.items.clear();
    }
    pub(super) fn take_management_change(&mut self) -> Option<crate::tags::Change> {
        self.management_change.take()
    }
    pub(super) fn management_open(&self) -> bool {
        self.manager.open
    }
    pub(super) fn management_ui(&mut self, ctx: &egui::Context) {
        self.manager.ui(ctx, &self.worker);
    }
    pub(super) fn set_target(
        &mut self,
        key: TargetKey,
        target: Result<Option<Target>, String>,
        caption: String,
        browse: Browse,
    ) {
        if self.editor.key == key {
            return;
        }
        self.editor.key = key;
        self.editor.caption = caption;
        self.editor.browse = browse;
        self.editor.input.clear();
        self.editor.page = 0;
        match target {
            Ok(target) => {
                self.editor.target = target;
                self.editor.error = None;
            }
            Err(error) => {
                self.editor.target = None;
                self.editor.error = Some(error);
            }
        }
        self.editor.invalidate();
    }
    pub(super) fn set_filter(&mut self, filter: &Filter) {
        if &self.membership.filter != filter {
            self.membership.filter = filter.clone();
            self.membership.filter_tags.clear();
            self.membership.invalidate();
        }
    }
    pub(super) fn remember_filter_label(&mut self, tag: &Tag) {
        if self.membership.filter.contains(tag.id)
            && !self
                .membership
                .filter_tags
                .iter()
                .any(|item| item.id == tag.id)
        {
            self.membership.filter_tags.push(tag.clone());
        }
    }
    pub(super) fn request(&mut self) {
        if !self.ready {
            self.membership.observed.clear();
            return;
        }
        self.manager.request(&self.worker);
        self.membership.request(&self.worker);
        self.editor.request(&self.worker);
        self.suggestions.search(if self.filtering {
            &self.filter_text
        } else {
            &self.editor.input
        });
        self.suggestions.request(&self.worker);
    }
    pub(super) fn take_action(&mut self) -> Option<Action> {
        self.action.take()
    }
    pub(super) fn target_key(&self) -> &TargetKey {
        &self.editor.key
    }

    pub(super) fn row(
        &mut self,
        ui: &mut egui::Ui,
        source: &Path,
        rect: egui::Rect,
        browse: Browse,
    ) -> bool {
        if rect.width() < 24.0 {
            return false;
        }
        let key = crate::path_codec::encode(source);
        self.membership
            .observed
            .entry(key.clone())
            .or_insert_with(|| source.to_path_buf());
        let Some(tags) = self.membership.cache.get(&key) else {
            return false;
        };
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("sound-tags", &key))
                .max_rect(rect.shrink2(egui::vec2(4.0, 3.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child.spacing_mut().item_spacing.x = 4.0;
        let mut action = None;
        let mut displayed = 0;
        for tag in tags.iter().take(2) {
            let reserve = if tags.len() > displayed + 1 {
                34.0
            } else {
                0.0
            };
            let available = child.available_width() - reserve;
            if available < 36.0 {
                break;
            }
            let response = chip(
                &mut child,
                &tag.name,
                self.membership.filter.contains(tag.id),
                available.min(110.0),
            );
            if response
                .on_hover_text(format!("{} — click to filter library sounds", tag.name))
                .clicked()
            {
                action = Some(Action::Toggle(tag.clone(), browse));
            }
            displayed += 1;
        }
        if displayed < tags.len() {
            child
                .label(
                    RichText::new(format!("+{}", tags.len() - displayed))
                        .size(11.0)
                        .weak(),
                )
                .on_hover_text(
                    tags.iter()
                        .map(|t| t.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
        }
        let clicked = action.is_some();
        if clicked {
            self.action = action;
        }
        clicked
    }

    pub(super) fn filter_button(&mut self, ui: &mut egui::Ui) {
        let count = self.membership.filter.ids().len();
        let response = ui.add_sized(
            [BUTTON_WIDTH, 34.0],
            egui::Button::new(if count == 0 {
                "Tags".into()
            } else {
                format!("Tags ({count})")
            }),
        );
        if response.clicked() {
            self.filter_text.clear();
            self.suggestions.generation += 1;
            self.suggestions.pending = false;
            self.suggestions.needed = true;
        }
        self.filtering = false;
        egui::Popup::menu(&response)
            .width(280.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                self.filtering = true;
                ui.label("Match all selected tags");
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter_text)
                        .char_limit(MAX_NAME_CHARS)
                        .hint_text("Find tags..."),
                );
                ui.separator();
                for tag in &self.suggestions.items {
                    if ui
                        .selectable_label(self.membership.filter.contains(tag.id), &tag.name)
                        .clicked()
                    {
                        self.action = Some(Action::Toggle(tag.clone(), Browse::CurrentLibrary));
                    }
                }
                if self.suggestions.pending || self.suggestions.needed {
                    ui.weak("Loading tags...");
                } else if self.suggestions.items.is_empty() {
                    ui.weak("No matching tags. Add a tag to a sound first.");
                }
                if let Some(error) = &self.suggestions.error {
                    ui.colored_label(Color32::from_rgb(245, 143, 140), error);
                }
            });
    }
    pub(super) fn active_filters(&mut self, ui: &mut egui::Ui) {
        if self.membership.filter.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for id in self.membership.filter.ids() {
                let name = self
                    .membership
                    .filter_tags
                    .iter()
                    .find(|tag| tag.id == *id)
                    .map_or_else(|| format!("Tag {}", id.0), |tag| tag.name.clone());
                if chip(ui, &format!("{name} ×"), true, 180.0).clicked() {
                    self.action = Some(Action::RemoveFilter(*id));
                }
            }
            if ui.small_button("Clear tag filters").clicked() {
                self.action = Some(Action::ClearFilters);
            }
        });
    }

    pub(super) fn panel(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        let mut add = None;
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Tags").strong());
                    ui.weak(&self.editor.caption);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(self.ready && self.editor.save.is_none(), egui::Button::new("Manage tags").small()).clicked() {
                            self.manager.open();
                        }
                    });
                });
                if let Some(error) = &self.startup_error {
                    ui.colored_label(Color32::from_rgb(245, 143, 140), error);
                    return;
                }
                if let Some(error) = &self.editor.error {
                    ui.colored_label(Color32::from_rgb(245, 143, 140), error);
                    if self.editor.target.is_some() && ui.button("Retry tag summary").clicked() { self.editor.invalidate(); }
                }
                if self.editor.target.is_none() { return; }
                if let Some(summary) = &self.editor.summary {
                    egui::ScrollArea::vertical()
                        .id_salt("tag-chip-editor")
                        .max_height(90.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for used in summary.tags.iter().skip(self.editor.page * PANEL_PAGE).take(PANEL_PAGE) {
                                    let label = if used.count == summary.sources {
                                        used.tag.name.clone()
                                    } else {
                                        format!("{} ({}/{})", used.tag.name, used.count, summary.sources)
                                    };
                                    ui.horizontal(|ui| {
                                        if chip(ui, &label, self.membership.filter.contains(used.tag.id), 160.0)
                                            .on_hover_text(format!("{} — present on {} of {} selected sounds; click to filter library sounds", used.tag.name, used.count, summary.sources)).clicked() {
                                            self.action = Some(Action::Toggle(used.tag.clone(), self.editor.browse));
                                        }
                                        if ui.add_enabled(self.editor.save.is_none(), egui::Button::new("×").small())
                                            .on_hover_text("Remove this tag from the selected sounds; keep their other tags").clicked() {
                                            remove = Some(used.tag.id);
                                        }
                                    });
                                }
                            });
                        });
                    if summary.tags.len() > PANEL_PAGE {
                        ui.horizontal(|ui| {
                            if ui.add_enabled(self.editor.page > 0, egui::Button::new("Previous tags")).clicked() {
                                self.editor.page -= 1;
                            }
                            ui.weak(format!("Page {} of {}", self.editor.page + 1, summary.tags.len().div_ceil(PANEL_PAGE)));
                            if ui.add_enabled((self.editor.page + 1) * PANEL_PAGE < summary.tags.len(), egui::Button::new("Next tags")).clicked() {
                                self.editor.page += 1;
                            }
                        });
                    }
                } else if self.editor.pending || self.editor.needed {
                    ui.weak("Loading tags...");
                }
                let can_edit = self.ready && self.editor.save.is_none() && self.editor.summary.is_some();
                ui.horizontal(|ui| {
                    let input = ui.add_enabled(can_edit, egui::TextEdit::singleline(&mut self.editor.input)
                        .char_limit(MAX_NAME_CHARS).desired_width(220.0).hint_text("Add tag..."));
                    let add_button = ui.add_enabled(can_edit && !self.editor.input.trim().is_empty(), egui::Button::new("Add"));
                    let enter = can_edit && (input.has_focus() || input.lost_focus())
                        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                    if add_button.clicked() || enter { add = Some(self.editor.input.clone()); }
                    let existing = ui.add_enabled(can_edit, egui::Button::new("Existing"));
                    egui::Popup::menu(&existing)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            for tag in &self.suggestions.items {
                                if ui.button(&tag.name).clicked() { add = Some(tag.name.clone()); ui.close(); }
                            }
                            if self.suggestions.items.is_empty() { ui.weak("No matching tags"); }
                        });
                });
                if let Some(save) = &mut self.editor.save {
                    if let Some(error) = &save.error {
                        ui.colored_label(Color32::from_rgb(245, 143, 140), format!("Tag edit failed: {error}"));
                        if ui.button("Retry tag edit").clicked() {
                            save.error = self.worker.edit(save.operation, save.target.clone(), save.edit.clone()).err();
                        }
                    } else { ui.weak("Saving tags..."); }
                }
                if let Some(error) = &self.membership.error {
                    ui.colored_label(Color32::from_rgb(245, 143, 140), error);
                    if ui.button("Retry tag lookup").clicked() { self.membership.invalidate(); }
                }
            });
        if let Some(name) = add {
            self.editor.submit(&self.worker, Edit::Add(name));
        }
        if let Some(id) = remove {
            self.editor.submit(&self.worker, Edit::Remove(id));
        }
    }
}

fn chip(ui: &mut egui::Ui, label: &str, selected: bool, max_width: f32) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let width = (ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::WHITE)
        .size()
        .x
        + 14.0)
        .min(max_width)
        .min(ui.available_width())
        .max(20.0);
    ui.add_sized(
        [width, 22.0],
        egui::Button::new(RichText::new(label).size(12.0))
            .truncate()
            .corner_radius(5)
            .fill(if selected {
                Color32::from_rgb(37, 69, 63)
            } else {
                Color32::from_rgb(43, 47, 49)
            })
            .stroke(egui::Stroke::new(
                1.0,
                if selected {
                    Color32::from_rgb(99, 172, 153)
                } else {
                    Color32::from_gray(71)
                },
            )),
    )
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/tags.rs"
    ));
}
