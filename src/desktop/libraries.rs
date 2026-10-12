use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText};

use crate::catalog::{FolderPage, Scope};
use crate::libraries::{Library, LibraryId};
use crate::worker::{EditOutcome, LibraryEdit, Workers};

mod tree;
use super::navigation::{RetainedState, Views};
use tree::TreeFocus;

const MAX_OPEN_FOLDERS: usize = 32;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Location {
    pub library: Option<LibraryId>,
    pub folder: Option<PathBuf>,
}

#[derive(Default)]
struct BankView {
    folder: Option<PathBuf>,
    expanded: BTreeSet<PathBuf>,
    offsets: BTreeMap<PathBuf, usize>,
    tree_offset: f32,
}

impl RetainedState for BankView {
    fn retained_bytes(&self) -> usize {
        self.folder
            .as_ref()
            .map_or(0, |path| path.as_os_str().len())
            + self
                .expanded
                .iter()
                .chain(self.offsets.keys())
                .map(|path| path.as_os_str().len())
                .sum::<usize>()
    }
}

struct Breadcrumb {
    label: String,
    folder: Option<PathBuf>,
}

#[derive(Default)]
pub(super) struct Libraries {
    bank_views: Views<Option<LibraryId>, BankView>,
    groups: Vec<Library>,
    ready: bool,
    initialized: bool,
    active: Option<LibraryId>,
    folder: Option<PathBuf>,
    expanded: BTreeSet<PathBuf>,
    pages: BTreeMap<PathBuf, FolderPage>,
    offsets: BTreeMap<PathBuf, usize>,
    pending: BTreeSet<PathBuf>,
    pub(super) generation: u64,
    error: Option<String>,
    save: Option<PendingSave>,
    operation: u64,
    dialog: Option<Dialog>,
    initial_folder: Option<PathBuf>,
    tree_focus: Option<TreeFocus>,
    focus_scroll: bool,
    tree_offset: f32,
}

struct PendingSave {
    token: u64,
    edit: LibraryEdit,
    error: Option<String>,
}

enum Dialog {
    Name {
        library: Option<LibraryId>,
        text: String,
        focus: bool,
    },
    Remove {
        edit: LibraryEdit,
        label: String,
    },
}

pub(super) enum Action {
    ScopeChanged,
    Scan(PathBuf),
}

impl Libraries {
    pub(super) fn location(&self) -> Location {
        Location {
            library: self.active,
            folder: self.folder.clone(),
        }
    }

    pub(super) fn valid_location(&self, location: &Location) -> bool {
        let roots: Vec<_> = self
            .groups
            .iter()
            .filter(|bank| location.library.is_none() || location.library == Some(bank.id))
            .flat_map(|bank| bank.sources.iter().map(|source| &source.path))
            .collect();
        (location.library.is_none()
            || self
                .groups
                .iter()
                .any(|bank| location.library == Some(bank.id)))
            && location
                .folder
                .as_ref()
                .is_none_or(|folder| roots.iter().any(|root| folder.starts_with(root)))
    }
    pub(super) fn scope(&self) -> Scope {
        if let Some(folder) = &self.folder {
            return Scope::Folders(vec![folder.clone()]);
        }
        if let Some(group) = self
            .groups
            .iter()
            .find(|group| Some(group.id) == self.active)
        {
            return group.scope();
        }
        Scope::Folders(
            self.groups
                .iter()
                .flat_map(|group| group.sources.iter().map(|source| source.path.clone()))
                .collect(),
        )
    }

    fn breadcrumbs(&self) -> Vec<Breadcrumb> {
        let bank = self
            .groups
            .iter()
            .find(|group| Some(group.id) == self.active);
        let mut crumbs = vec![Breadcrumb {
            label: bank
                .map_or("All libraries", |bank| bank.name.as_str())
                .to_owned(),
            folder: None,
        }];
        let Some(folder) = &self.folder else {
            return crumbs;
        };
        let roots = self.roots();
        let Some(root) = roots
            .iter()
            .filter(|root| folder.starts_with(root))
            .max_by_key(|root| root.components().count())
        else {
            return crumbs;
        };
        let skip_root = roots.len() == 1
            && bank.is_some_and(|bank| {
                root.file_name()
                    .is_some_and(|name| name.to_string_lossy() == bank.name)
            });
        let mut ancestors: Vec<_> = folder
            .ancestors()
            .take_while(|path| path.starts_with(root))
            .collect();
        ancestors.reverse();
        for path in ancestors {
            if skip_root && path == root {
                continue;
            }
            crumbs.push(Breadcrumb {
                label: path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned(),
                folder: Some(path.to_path_buf()),
            });
        }
        crumbs
    }

    fn select_folder(&mut self, folder: Option<PathBuf>) {
        if let Some(path) = &folder {
            if path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return;
            }
            if !self.roots().iter().any(|root| path.starts_with(root)) {
                return;
            }
            // Native ancestors remain locators. No filesystem read or lossy
            // display-to-path conversion is needed to reveal a saved scope.
            let roots = self.roots();
            let mut ancestors: Vec<_> = path
                .ancestors()
                .skip(1)
                .take_while(|ancestor| roots.iter().any(|root| ancestor.starts_with(root)))
                .collect();
            ancestors.reverse();
            for ancestor in ancestors {
                if self.expanded.len() >= MAX_OPEN_FOLDERS {
                    break;
                }
                self.expanded.insert(ancestor.to_path_buf());
            }
        }
        self.folder = folder;
    }

    pub(super) fn select_smoke_subfolder(&mut self, relative: PathBuf) -> bool {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return false;
        }
        let Some(root) = self.roots().into_iter().next() else {
            return false;
        };
        self.select_folder(Some(root.join(relative)));
        true
    }

    pub(super) fn breadcrumb_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let before = self.scope();
        let crumbs = self.breadcrumbs();
        let mut choice = None;
        let width = (ui.available_width() / 3.0).clamp(90.0, 240.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for (index, crumb) in crumbs.iter().enumerate() {
                if crumbs.len() > 5 && index == 2 {
                    ui.label(RichText::new("›").weak());
                    ui.menu_button("…", |ui| {
                        for hidden in &crumbs[2..crumbs.len() - 2] {
                            if ui
                                .button(&hidden.label)
                                .on_hover_text(
                                    hidden
                                        .folder
                                        .as_ref()
                                        .map(|path| path.display().to_string())
                                        .unwrap_or_default(),
                                )
                                .clicked()
                            {
                                choice = Some(hidden.folder.clone());
                                ui.close();
                            }
                        }
                    });
                }
                if crumbs.len() > 5 && (2..crumbs.len() - 2).contains(&index) {
                    continue;
                }
                if index != 0 {
                    ui.label(RichText::new("›").weak());
                }
                let text = RichText::new(&crumb.label).size(13.0);
                let text = if index == crumbs.len() - 1 {
                    text.strong()
                } else {
                    text.color(tree::secondary_color(ui))
                };
                let response = ui.add_sized(
                    [
                        width.min(
                            ui.painter()
                                .layout_no_wrap(
                                    crumb.label.clone(),
                                    egui::FontId::proportional(13.0),
                                    ui.visuals().text_color(),
                                )
                                .size()
                                .x
                                + 14.0,
                        ),
                        24.0,
                    ],
                    egui::Button::new(text).frame(false).truncate(),
                );
                let response = response.on_hover_text(
                    crumb
                        .folder
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| {
                            "All sounds in this library, including subfolders".into()
                        }),
                );
                if response.clicked() {
                    choice = Some(crumb.folder.clone());
                }
            }
        });
        if let Some(folder) = choice {
            self.select_folder(folder);
        }
        before != self.scope()
    }

    pub(super) fn snapshot(&mut self, result: Result<Vec<Library>, String>) -> bool {
        let before = self.scope();
        match result {
            Ok(groups) => {
                self.groups = groups;
                self.bank_views.retain(|id| {
                    id.is_none() || self.groups.iter().any(|bank| Some(bank.id) == *id)
                });
                self.ready = true;
                self.error = None;
                if !self.initialized {
                    self.active = self.groups.first().map(|group| group.id);
                    self.initialized = true;
                    self.open_first_source();
                } else if self.active.is_some()
                    && !self
                        .groups
                        .iter()
                        .any(|group| Some(group.id) == self.active)
                {
                    self.active = None;
                    self.folder = None;
                }
                let roots = self.roots();
                if self
                    .folder
                    .as_ref()
                    .is_some_and(|folder| !roots.iter().any(|root| folder.starts_with(root)))
                {
                    self.folder = None;
                }
                self.expanded
                    .retain(|folder| roots.iter().any(|root| folder.starts_with(root)));
                if self.tree_focus.as_ref().is_some_and(|target| match target {
                    TreeFocus::All => false,
                    TreeFocus::Folder { root, path } => {
                        !roots.contains(root) || !path.starts_with(root)
                    }
                }) {
                    self.tree_focus = None;
                    self.focus_scroll = false;
                }
                self.invalidate();
            }
            Err(error) => {
                eprintln!("Library snapshot failed: {error}");
                self.error = Some(format!("Libraries unavailable: {error}"));
            }
        }
        before != self.scope()
    }

    pub(super) fn folder_page(&mut self, generation: u64, result: Result<FolderPage, String>) {
        if generation != self.generation {
            return;
        }
        match result {
            Ok(page) => {
                if page.offset != self.offsets.get(&page.parent).copied().unwrap_or(0) {
                    return;
                }
                self.pending.remove(&page.parent);
                if self.expanded.contains(&page.parent) {
                    self.pages.insert(page.parent.clone(), page);
                }
            }
            Err(error) => {
                self.pending.clear();
                eprintln!("Folder query failed: {error}");
                self.error = Some(format!("Folders unavailable: {error}"));
            }
        }
    }

    pub(super) fn edited(
        &mut self,
        workers: &Workers,
        token: u64,
        result: Result<EditOutcome, String>,
    ) -> Option<PathBuf> {
        if !self.save.as_ref().is_some_and(|save| save.token == token) {
            return None;
        }
        match result {
            Ok(outcome) => {
                let changes_selection = match self.save.as_ref().map(|save| &save.edit) {
                    Some(LibraryEdit::Create(_)) => true,
                    Some(LibraryEdit::Remove(id)) => self.active == Some(*id),
                    _ => false,
                };
                self.save = None;
                self.dialog = None;
                if changes_selection {
                    self.active = outcome.selected;
                    self.folder = None;
                    self.expanded.clear();
                    self.invalidate();
                }
                if let Some(folder) = &outcome.scan_folder
                    && self.expanded.len() < MAX_OPEN_FOLDERS
                {
                    self.expanded.insert(folder.clone());
                }
                if let Some(folder) = self.initial_folder.take()
                    && let Some(id) = outcome.selected
                {
                    self.mutate(workers, LibraryEdit::Attach(id, folder));
                }
                outcome.scan_folder
            }
            Err(error) => {
                if let Some(save) = &mut self.save {
                    save.error = Some(error);
                }
                None
            }
        }
    }

    pub(super) fn add_initial_folder(&mut self, workers: &Workers, folder: PathBuf) {
        if let Some(id) = self.active {
            self.mutate(workers, LibraryEdit::Attach(id, folder));
        } else {
            self.initial_folder = Some(folder);
            self.mutate(workers, LibraryEdit::Create("My library".into()));
        }
    }

    pub(super) fn settled(&self) -> bool {
        self.ready
            && self.save.is_none()
            && self.pending.is_empty()
            && self
                .expanded
                .iter()
                .all(|folder| self.pages.contains_key(folder))
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.groups
            .iter()
            .filter(|group| self.active.is_none() || Some(group.id) == self.active)
            .flat_map(|group| group.sources.iter().map(|source| source.path.clone()))
            .collect()
    }

    fn invalidate(&mut self) {
        self.generation += 1;
        self.pages.clear();
        self.pending.clear();
    }

    fn open_first_source(&mut self) {
        if let Some(source) = self
            .groups
            .iter()
            .find(|group| Some(group.id) == self.active)
            .and_then(|group| group.sources.first())
        {
            self.expanded.insert(source.path.clone());
        }
    }

    fn choose_bank(&mut self, id: Option<LibraryId>) {
        if self.active == id {
            return;
        }
        let remembered = self.bank_views.switch(
            self.active,
            BankView {
                folder: self.folder.take(),
                expanded: std::mem::take(&mut self.expanded),
                offsets: std::mem::take(&mut self.offsets),
                tree_offset: self.tree_offset,
            },
            &id,
        );
        let first_visit = remembered.is_none();
        let restored = remembered.unwrap_or_default();
        self.active = id;
        self.tree_focus = None;
        self.focus_scroll = false;
        self.tree_offset = restored.tree_offset;
        self.expanded = restored.expanded;
        self.offsets = restored.offsets;
        self.folder = restored.folder;
        if !self.valid_location(&self.location()) {
            self.folder = None;
        }
        let roots = self.roots();
        self.expanded
            .retain(|path| roots.iter().any(|root| path.starts_with(root)));
        self.offsets
            .retain(|path, _| roots.iter().any(|root| path.starts_with(root)));
        self.invalidate();
        if first_visit {
            self.open_first_source();
        }
    }

    fn mutate(&mut self, workers: &Workers, edit: LibraryEdit) {
        self.operation += 1;
        let error = workers.edit_library(self.operation, edit.clone()).err();
        self.save = Some(PendingSave {
            token: self.operation,
            edit,
            error,
        });
    }

    fn toggle_folder(&mut self, path: &PathBuf) {
        if self.expanded.contains(path) {
            self.expanded.retain(|folder| !folder.starts_with(path));
            self.pages.retain(|folder, _| !folder.starts_with(path));
            self.offsets.retain(|folder, _| !folder.starts_with(path));
            self.pending.retain(|folder| !folder.starts_with(path));
        } else if self.expanded.len() < MAX_OPEN_FOLDERS {
            self.expanded.insert(path.clone());
        } else {
            self.error = Some("Close a branch before opening more than 32 folders at once.".into());
        }
    }

    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        workers: &Workers,
        scanning: bool,
    ) -> Option<Action> {
        let before = self.scope();
        let can_edit = self.ready && self.save.is_none() && !scanning;
        ui.horizontal(|ui| {
            ui.label(RichText::new("LIBRARIES").strong());
            if ui
                .add_enabled(can_edit, egui::Button::new("New"))
                .on_hover_text("Create a named library")
                .clicked()
            {
                self.dialog = Some(Dialog::Name {
                    library: None,
                    text: String::new(),
                    focus: true,
                });
            }
            if ui
                .add_enabled(can_edit, egui::Button::new("Refresh"))
                .on_hover_text("Check source folders again")
                .clicked()
            {
                self.error = workers.refresh_libraries().err();
            }
        });
        let can_save =
            self.ready && !scanning && self.save.as_ref().is_none_or(|save| save.error.is_some());
        self.form(ui, workers, can_save);
        self.feedback(ui, workers);
        ui.separator();
        if !self.ready {
            ui.label("Opening libraries...");
            return None;
        }
        if ui
            .selectable_label(self.active.is_none(), "All libraries")
            .clicked()
        {
            self.choose_bank(None);
        }
        let mut bank_choice = None;
        let mut dialog = None;
        egui::ScrollArea::vertical()
            .id_salt("library-banks")
            .max_height(180.0)
            .auto_shrink([false, true])
            .show_rows(ui, 28.0, self.groups.len(), |ui, range| {
                for index in range {
                    let bank = &self.groups[index];
                    let response = ui
                        .add_sized(
                            [ui.available_width(), 28.0],
                            egui::Button::selectable(self.active == Some(bank.id), &bank.name)
                                .shortcut_text(bank.count.to_string())
                                .truncate()
                                .selected(self.active == Some(bank.id)),
                        )
                        .on_hover_text(&bank.name);
                    if response.clicked() {
                        bank_choice = Some(bank.id);
                    }
                    response.context_menu(|ui| {
                        if ui
                            .add_enabled(can_edit, egui::Button::new("Rename library..."))
                            .clicked()
                        {
                            dialog = Some(Dialog::Name {
                                library: Some(bank.id),
                                text: bank.name.clone(),
                                focus: true,
                            });
                            ui.close();
                        }
                        if ui
                            .add_enabled(can_edit, egui::Button::new("Remove library..."))
                            .clicked()
                        {
                            dialog = Some(Dialog::Remove {
                                edit: LibraryEdit::Remove(bank.id),
                                label: bank.name.clone(),
                            });
                            ui.close();
                        }
                    });
                }
            });
        if self.groups.is_empty() {
            ui.label("Create a library, then add its source folders.");
        }
        if let Some(id) = bank_choice {
            self.choose_bank(Some(id));
        }
        if let Some(dialog) = dialog {
            self.dialog = Some(dialog);
        }
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("FOLDERS").strong());
            let response = ui
                .add_enabled(
                    can_edit && self.active.is_some(),
                    egui::Button::new("+").small(),
                )
                .on_hover_text("Add source folder...");
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    can_edit && self.active.is_some(),
                    "Add source folder",
                )
            });
            if let Some(id) = self.active
                && response.clicked()
            {
                #[cfg(windows)]
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Add a source folder to this library")
                    .pick_folder()
                {
                    self.mutate(workers, LibraryEdit::Attach(id, path));
                }
            }
        });
        let action = self.tree_ui(ui, workers, can_edit);
        if before != self.scope() {
            Some(Action::ScopeChanged)
        } else {
            action
        }
    }

    fn feedback(&mut self, ui: &mut egui::Ui, workers: &Workers) {
        if let Some(error) = &self.error {
            ui.colored_label(Color32::from_rgb(245, 143, 140), error);
            if ui.button("Retry loading libraries").clicked() {
                self.error = workers.refresh_libraries().err();
            }
        }
        if let Some(save) = &self.save {
            if let Some(error) = &save.error {
                ui.colored_label(
                    Color32::from_rgb(245, 143, 140),
                    format!("Not saved: {error}"),
                );
                let edit = save.edit.clone();
                ui.horizontal(|ui| {
                    if ui.button("Retry save").clicked() {
                        self.mutate(workers, edit);
                    }
                    if ui.button("Discard edit").clicked() {
                        self.save = None;
                        self.initial_folder = None;
                    }
                });
            } else {
                ui.label("Saving library...");
            }
        }
    }

    fn form(&mut self, ui: &mut egui::Ui, workers: &Workers, can_edit: bool) {
        let mut edit = None;
        let mut cancel = false;
        if let Some(dialog) = &mut self.dialog {
            ui.group(|ui| match dialog {
                Dialog::Name {
                    library,
                    text,
                    focus,
                } => {
                    ui.label(if library.is_some() {
                        "Rename library"
                    } else {
                        "New library"
                    });
                    let just_opened = *focus;
                    let response = ui.add_enabled(
                        can_edit,
                        egui::TextEdit::singleline(text)
                            .char_limit(crate::names::MAX_CHARS)
                            .hint_text("Library name"),
                    );
                    if *focus {
                        response.request_focus();
                        *focus = false;
                    }
                    let submit = !just_opened
                        && (response.has_focus() || response.lost_focus())
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                can_edit && !text.trim().is_empty(),
                                egui::Button::new("Save"),
                            )
                            .clicked()
                            || can_edit && submit
                        {
                            edit = Some(match library {
                                Some(id) => LibraryEdit::Rename(*id, text.clone()),
                                None => LibraryEdit::Create(text.clone()),
                            });
                        }
                        cancel = ui
                            .add_enabled(can_edit, egui::Button::new("Cancel"))
                            .clicked();
                    });
                }
                Dialog::Remove {
                    edit: mutation,
                    label,
                } => {
                    ui.label(format!("Remove {label} from Sift?"));
                    ui.label("Source audio and saved collections stay in place.");
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(can_edit, egui::Button::new("Remove registration"))
                            .clicked()
                        {
                            edit = Some(mutation.clone());
                        }
                        cancel = ui
                            .add_enabled(can_edit, egui::Button::new("Cancel"))
                            .clicked();
                    });
                }
            });
        }
        if let Some(edit) = edit {
            self.mutate(workers, edit);
        }
        if cancel {
            self.dialog = None;
        }
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/libraries.rs"
    ));
}
