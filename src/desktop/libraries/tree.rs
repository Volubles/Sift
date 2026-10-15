//! Bounded folder rows, interaction, and native-path focus for the library panel.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use eframe::egui::{self, Color32, Id, Rect, Response};

use super::{Action, Dialog, Libraries};
use crate::{
    catalog::FOLDER_PAGE_SIZE,
    libraries::{Availability, Source},
    worker::{LibraryEdit, Workers},
};

const ROW_HEIGHT: f32 = 30.0;
const INDENT: f32 = 16.0;
const TOGGLE_WIDTH: f32 = 26.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TreeFocus {
    All,
    Folder { root: PathBuf, path: PathBuf },
}

pub(super) enum TreeRow<'a> {
    All {
        count: Option<usize>,
    },
    Folder {
        root: &'a Path,
        path: &'a Path,
        name: Cow<'a, str>,
        depth: usize,
        parent: Option<usize>,
        expandable: bool,
        source: Option<&'a Source>,
    },
    Loading {
        parent: &'a Path,
        depth: usize,
    },
    Pager {
        parent: &'a Path,
        depth: usize,
        offset: usize,
        total: usize,
    },
}

impl TreeRow<'_> {
    fn target(&self) -> Option<TreeFocus> {
        match self {
            Self::All { .. } => Some(TreeFocus::All),
            Self::Folder { root, path, .. } => Some(TreeFocus::Folder {
                root: root.to_path_buf(),
                path: path.to_path_buf(),
            }),
            _ => None,
        }
    }

    fn matches(&self, target: &TreeFocus) -> bool {
        match (self, target) {
            (Self::All { .. }, TreeFocus::All) => true,
            (
                Self::Folder { root, path, .. },
                TreeFocus::Folder {
                    root: focused_root,
                    path: focused_path,
                },
            ) => *root == focused_root && *path == focused_path,
            _ => false,
        }
    }
}

enum Command {
    Focus(TreeFocus),
    Toggle(TreeFocus),
    Select(TreeFocus),
}

impl Libraries {
    fn rows(&self) -> Vec<TreeRow<'_>> {
        let count = self
            .groups
            .iter()
            .find(|bank| Some(bank.id) == self.active)
            .map(|bank| bank.count);
        let mut rows = vec![TreeRow::All { count }];
        let mut seen = std::collections::BTreeSet::new();
        for source in self
            .groups
            .iter()
            .filter(|bank| self.active.is_none() || Some(bank.id) == self.active)
            .flat_map(|bank| &bank.sources)
        {
            if !seen.insert(source.path.as_path()) {
                continue;
            }
            let parent = rows.len();
            rows.push(TreeRow::Folder {
                root: &source.path,
                path: &source.path,
                name: source
                    .path
                    .file_name()
                    .unwrap_or(source.path.as_os_str())
                    .to_string_lossy(),
                depth: 0,
                parent: None,
                expandable: true,
                source: Some(source),
            });
            self.append_children(&source.path, &source.path, 1, parent, &mut rows);
        }
        rows
    }

    fn append_children<'a>(
        &'a self,
        root: &'a Path,
        path: &'a Path,
        depth: usize,
        parent: usize,
        rows: &mut Vec<TreeRow<'a>>,
    ) {
        if !self.expanded.contains(path) || depth >= 64 {
            return;
        }
        if let Some(page) = self.pages.get(path) {
            for folder in &page.rows {
                let index = rows.len();
                rows.push(TreeRow::Folder {
                    root,
                    path: &folder.source,
                    name: Cow::Borrowed(&folder.name),
                    depth,
                    parent: Some(parent),
                    expandable: folder.has_children,
                    source: None,
                });
                self.append_children(root, &folder.source, depth + 1, index, rows);
            }
            if page.total > FOLDER_PAGE_SIZE {
                rows.push(TreeRow::Pager {
                    parent: path,
                    depth,
                    offset: page.offset,
                    total: page.total,
                });
            }
        } else {
            rows.push(TreeRow::Loading {
                parent: path,
                depth,
            });
        }
    }

    fn focus_id(&self, target: &TreeFocus) -> Id {
        match target {
            TreeFocus::All => Id::new(("sift-folder-all", self.active)),
            // The root distinguishes repeated appearances under overlapping sources.
            TreeFocus::Folder { root, path } => {
                Id::new(("sift-folder-node", self.active, root, path))
            }
        }
    }

    pub(in crate::desktop) fn tree_has_focus(&self, context: &egui::Context) -> bool {
        self.tree_focus.as_ref().is_some_and(|target| {
            context.memory(|memory| {
                memory.has_focus(self.focus_id(target))
                    || memory.has_focus(self.focus_id(target).with("expand"))
            })
        })
    }

    fn keyboard_command(&self, key: egui::Key) -> Option<Command> {
        let rows = self.rows();
        let current = rows.iter().position(|row| {
            self.tree_focus
                .as_ref()
                .is_some_and(|target| row.matches(target))
        })?;
        let row = &rows[current];
        let focus = |row: &TreeRow<'_>| row.target().map(Command::Focus);
        match key {
            egui::Key::ArrowDown => rows[current + 1..].iter().find_map(focus),
            egui::Key::ArrowUp => rows[..current].iter().rev().find_map(focus),
            egui::Key::Home => rows.first().and_then(focus),
            egui::Key::End => rows.iter().rev().find_map(focus),
            egui::Key::Enter => row.target().map(Command::Select),
            egui::Key::ArrowRight => match row {
                TreeRow::Folder {
                    path,
                    expandable: true,
                    ..
                } if !self.expanded.contains(*path) => row.target().map(Command::Toggle),
                TreeRow::Folder { .. } => rows.get(current + 1).and_then(|child| match child {
                    TreeRow::Folder {
                        parent: Some(parent),
                        ..
                    } if *parent == current => focus(child),
                    _ => None,
                }),
                _ => None,
            },
            egui::Key::ArrowLeft => match row {
                TreeRow::Folder { path, .. } if self.expanded.contains(*path) => {
                    row.target().map(Command::Toggle)
                }
                TreeRow::Folder {
                    parent: Some(parent),
                    ..
                } => focus(&rows[*parent]),
                _ => None,
            },
            _ => None,
        }
    }

    fn apply_command(&mut self, command: Command, context: &egui::Context) {
        let target = match command {
            Command::Focus(target) => target,
            Command::Select(target) => {
                self.select_folder(match &target {
                    TreeFocus::All => None,
                    TreeFocus::Folder { path, .. } => Some(path.clone()),
                });
                target
            }
            Command::Toggle(target) => {
                if let TreeFocus::Folder { path, .. } = &target {
                    self.toggle_folder(path);
                }
                target
            }
        };
        context.memory_mut(|memory| memory.request_focus(self.focus_id(&target)));
        self.tree_focus = Some(target);
        self.focus_scroll = true;
    }

    fn handle_tree_keys(&mut self, ui: &egui::Ui) {
        if !ui.is_enabled() || !self.tree_has_focus(ui.ctx()) {
            return;
        }
        let id = ui.memory(|memory| memory.focused());
        if let Some(id) = id {
            lock_arrows(ui, id);
        }
        let key = ui.input_mut(|input| {
            [
                egui::Key::ArrowDown,
                egui::Key::ArrowUp,
                egui::Key::ArrowRight,
                egui::Key::ArrowLeft,
                egui::Key::Home,
                egui::Key::End,
                egui::Key::Enter,
                egui::Key::Space,
            ]
            .into_iter()
            .find(|key| input.consume_key(egui::Modifiers::NONE, *key))
        });
        let command = if key == Some(egui::Key::Enter)
            && self
                .tree_focus
                .as_ref()
                .is_some_and(|target| id == Some(self.focus_id(target).with("expand")))
        {
            self.tree_focus.clone().map(Command::Toggle)
        } else {
            key.and_then(|key| self.keyboard_command(key))
        };
        if let Some(command) = command {
            self.apply_command(command, ui.ctx());
        }
    }

    pub(super) fn tree_ui(
        &mut self,
        ui: &mut egui::Ui,
        workers: &Workers,
        can_edit: bool,
    ) -> Option<Action> {
        self.repair_tree_focus(ui);
        self.handle_tree_keys(ui);
        let rows = self.rows();
        if rows.len() == 1 {
            ui.label(egui::RichText::new("Add a source folder to browse its sounds.").small());
        }
        let height = (ui.available_height() - 8.0).max(60.0);
        let tree_rect = Rect::from_min_size(
            ui.available_rect_before_wrap().min,
            egui::vec2(ui.available_width(), height),
        );
        let stride = ROW_HEIGHT + ui.spacing().item_spacing.y;
        let focus_index = self
            .tree_focus
            .as_ref()
            .and_then(|target| rows.iter().position(|row| row.matches(target)));
        let mut offset = self.tree_offset;
        if self.focus_scroll
            && let Some(index) = focus_index
        {
            offset = reveal_offset(offset, height, index, stride);
        }
        let mut command = None;
        let mut focused = None;
        let mut action = None;
        let mut dialog = None;
        let mut page_choice = None;
        let mut requests = Vec::new();
        let mut painted_focus = false;
        let output = egui::ScrollArea::vertical()
            .id_salt(("library-folders", self.active))
            .max_height(height)
            .auto_shrink([false, false])
            .vertical_scroll_offset(offset)
            .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                for index in range {
                    match &rows[index] {
                        TreeRow::All { count } => {
                            let target = TreeFocus::All;
                            let response = paint_row(
                                ui,
                                self.focus_id(&target),
                                Row {
                                    name: "All sounds",
                                    depth: 0,
                                    icon: Icon::All,
                                    expanded: None,
                                    selected: self.folder.is_none(),
                                    suffix: count.map(compact_count),
                                    status: false,
                                },
                            );
                            if response.body.clicked() {
                                command = Some(Command::Select(target.clone()));
                            }
                            if response.body.has_focus()
                                || response.toggle.as_ref().is_some_and(Response::has_focus)
                            {
                                if let Some(id) = ui.memory(|memory| memory.focused()) {
                                    lock_arrows(ui, id);
                                }
                                focused = Some(target);
                                painted_focus = true;
                            }
                            response
                                .body
                                .on_hover_text("All sounds in this library, including subfolders");
                        }
                        TreeRow::Folder {
                            root,
                            path,
                            name,
                            depth,
                            expandable,
                            source,
                            ..
                        } => {
                            let target = TreeFocus::Folder {
                                root: root.to_path_buf(),
                                path: path.to_path_buf(),
                            };
                            let open = self.expanded.contains(*path);
                            let (suffix, status) = match source.map(|source| &source.availability) {
                                Some(Availability::Offline) => (Some("Offline".into()), true),
                                Some(Availability::Unavailable(_)) => {
                                    (Some("Unavailable".into()), true)
                                }
                                _ => (source.map(|source| compact_count(source.count)), false),
                            };
                            let response = paint_row(
                                ui,
                                self.focus_id(&target),
                                Row {
                                    name,
                                    depth: *depth,
                                    icon: if source.is_some() {
                                        Icon::Source
                                    } else {
                                        Icon::Folder(open)
                                    },
                                    expanded: expandable.then_some(open),
                                    selected: self.folder.as_deref() == Some(*path),
                                    suffix,
                                    status,
                                },
                            );
                            if response.body.clicked() {
                                command = Some(Command::Select(target.clone()));
                            }
                            if response
                                .toggle
                                .as_ref()
                                .is_some_and(|toggle| toggle.clicked())
                            {
                                command = Some(Command::Toggle(target.clone()));
                            }
                            if response.body.has_focus()
                                || response.toggle.as_ref().is_some_and(Response::has_focus)
                            {
                                if let Some(id) = ui.memory(|memory| memory.focused()) {
                                    lock_arrows(ui, id);
                                }
                                focused = Some(target);
                                painted_focus = true;
                            }
                            if let (Some(library), Some(source)) = (self.active, source) {
                                response.body.context_menu(|ui| {
                                    if ui
                                        .add_enabled(
                                            can_edit,
                                            egui::Button::new("Rescan source folder"),
                                        )
                                        .clicked()
                                    {
                                        action = Some(Action::Scan(path.to_path_buf()));
                                        ui.close();
                                    }
                                    if ui
                                        .add_enabled(
                                            can_edit,
                                            egui::Button::new("Remove source folder..."),
                                        )
                                        .clicked()
                                    {
                                        dialog = Some(Dialog::Remove {
                                            edit: LibraryEdit::Detach(library, source.id),
                                            label: name.to_string(),
                                        });
                                        ui.close();
                                    }
                                });
                            }
                            let tooltip = source.map_or_else(
                                || path.display().to_string(),
                                |source| {
                                    format!(
                                        "{}\n{} cached sounds, including subfolders{}",
                                        path.display(),
                                        source.count,
                                        match &source.availability {
                                            Availability::Unavailable(error) =>
                                                format!("\n{error}"),
                                            _ => String::new(),
                                        }
                                    )
                                },
                            );
                            response.body.on_hover_text(tooltip);
                        }
                        TreeRow::Loading { parent, depth } => {
                            ui.horizontal(|ui| {
                                ui.add_space((*depth as f32 * INDENT).min(96.0) + TOGGLE_WIDTH);
                                ui.add_sized(
                                    [ui.available_width(), ROW_HEIGHT],
                                    egui::Label::new(
                                        egui::RichText::new("Loading folders…").weak(),
                                    ),
                                );
                            });
                            requests.push(parent.to_path_buf());
                        }
                        TreeRow::Pager {
                            parent,
                            depth,
                            offset,
                            total,
                        } => {
                            ui.push_id(("folder-pages", parent), |ui| {
                                ui.horizontal(|ui| {
                                    ui.set_min_height(ROW_HEIGHT);
                                    ui.add_space((*depth as f32 * INDENT).min(32.0));
                                    if ui
                                        .add_enabled(*offset > 0, egui::Button::new("‹"))
                                        .on_hover_text("Previous folders")
                                        .clicked()
                                    {
                                        page_choice = Some((
                                            parent.to_path_buf(),
                                            offset.saturating_sub(FOLDER_PAGE_SIZE),
                                        ));
                                    }
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{}–{} / {total}",
                                            offset + 1,
                                            (offset + FOLDER_PAGE_SIZE).min(*total)
                                        ))
                                        .small(),
                                    );
                                    if ui
                                        .add_enabled(
                                            offset + FOLDER_PAGE_SIZE < *total,
                                            egui::Button::new("›"),
                                        )
                                        .on_hover_text("Next folders")
                                        .clicked()
                                    {
                                        page_choice =
                                            Some((parent.to_path_buf(), offset + FOLDER_PAGE_SIZE));
                                    }
                                })
                            });
                        }
                    }
                }
            });
        if !painted_focus
            && self.tree_has_focus(ui.ctx())
            && let Some(id) = ui.memory(|memory| memory.focused())
        {
            // Virtualization and refresh may temporarily omit the focused row.
            // Register only its existing focus identity; never create a visible
            // phantom row or let an unrelated widget inherit it.
            ui.interact(
                Rect::from_min_size(ui.min_rect().min, egui::Vec2::ZERO),
                id,
                egui::Sense::focusable_noninteractive(),
            );
        }
        let loaded = self
            .expanded
            .iter()
            .all(|path| self.pages.contains_key(path));
        let interacted = ui.input(|input| {
            input
                .pointer
                .hover_pos()
                .is_some_and(|position| tree_rect.contains(position))
                && (input.pointer.any_down()
                    || input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::MouseWheel { .. })))
        });
        // Loading placeholders can be shorter than the restored tree. Keep its
        // requested position until the rows arrive, unless the user navigates.
        if loaded || interacted || (self.focus_scroll && painted_focus) {
            self.tree_offset = output.state.offset.y;
        }
        if painted_focus {
            self.focus_scroll = false;
        }
        if let Some(target) = focused {
            self.tree_focus = Some(target);
        }
        if let Some(command) = command {
            self.apply_command(command, ui.ctx());
        }
        if let Some(dialog) = dialog {
            self.dialog = Some(dialog);
        }
        if let Some((parent, offset)) = page_choice {
            self.offsets.insert(parent.clone(), offset);
            self.pages.remove(&parent);
            self.pending.remove(&parent);
        }
        for parent in requests {
            if self.expanded.contains(&parent)
                && self.error.is_none()
                && !self.pending.contains(&parent)
                && workers.folders(
                    self.generation,
                    parent.clone(),
                    self.offsets.get(&parent).copied().unwrap_or(0),
                )
            {
                self.pending.insert(parent);
            }
        }
        action
    }

    fn repair_tree_focus(&mut self, ui: &egui::Ui) {
        if !self.tree_has_focus(ui.ctx()) {
            return;
        }
        let rows = self.rows();
        let Some(target) = &self.tree_focus else {
            return;
        };
        if rows.iter().any(|row| row.matches(target))
            || self
                .expanded
                .iter()
                .any(|path| !self.pages.contains_key(path))
        {
            return;
        }
        let fallback = match target {
            TreeFocus::Folder { root, path } => rows.iter().rfind(|row| matches!(row, TreeRow::Folder { root: candidate_root, path: candidate, .. } if *candidate_root == root && path.starts_with(candidate))).and_then(TreeRow::target).unwrap_or(TreeFocus::All),
            TreeFocus::All => TreeFocus::All,
        };
        self.apply_command(Command::Focus(fallback), ui.ctx());
    }
}

fn lock_arrows(ui: &egui::Ui, id: Id) {
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            id,
            egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            },
        )
    });
}

fn reveal_offset(current: f32, height: f32, index: usize, stride: f32) -> f32 {
    let top = index as f32 * stride;
    if top < current {
        top
    } else if top + stride > current + height {
        (top + stride - height).max(0.0)
    } else {
        current
    }
}

fn compact_count(count: usize) -> String {
    if count >= 1_000_000 {
        format!("{:.1}m", count as f64 / 1_000_000.0)
    } else if count >= 1000 {
        format!("{:.1}k", count as f64 / 1000.0)
    } else {
        count.to_string()
    }
}

pub(super) fn secondary_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_gray(166)
    } else {
        Color32::from_gray(86)
    }
}

#[derive(Clone, Copy)]
enum Icon {
    All,
    Source,
    Folder(bool),
}

struct Row<'a> {
    name: &'a str,
    depth: usize,
    icon: Icon,
    expanded: Option<bool>,
    selected: bool,
    suffix: Option<String>,
    status: bool,
}
struct RowResponse {
    body: Response,
    toggle: Option<Response>,
}

fn paint_row(ui: &mut egui::Ui, id: Id, row: Row<'_>) -> RowResponse {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::hover(),
    );
    let indent = (row.depth as f32 * INDENT).min((rect.width() * 0.4).min(96.0));
    let toggle_rect = Rect::from_min_size(
        rect.min + egui::vec2(indent, 0.0),
        egui::vec2(TOGGLE_WIDTH, ROW_HEIGHT),
    );
    let body_rect = Rect::from_min_max(egui::pos2(toggle_rect.right(), rect.top()), rect.max);
    let body = ui.interact(body_rect, id, egui::Sense::click());
    body.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            row.selected,
            row.name,
        )
    });
    let toggle = row.expanded.map(|open| {
        let response = ui.interact(toggle_rect, id.with("expand"), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                format!("{} {}", if open { "Collapse" } else { "Expand" }, row.name),
            )
        });
        response.on_hover_text(format!(
            "{} {}",
            if open { "Collapse" } else { "Expand" },
            row.name
        ))
    });
    let painter = ui.painter();
    if row.selected {
        painter.rect_filled(rect, 3.0, ui.visuals().selection.bg_fill);
    } else if body.hovered() || toggle.as_ref().is_some_and(|toggle| toggle.hovered()) {
        painter.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    for level in 0..row.depth.min((indent / INDENT) as usize) {
        let x = rect.left() + level as f32 * INDENT + TOGGLE_WIDTH / 2.0;
        painter.line_segment(
            [
                egui::pos2(x, rect.top()),
                egui::pos2(x, rect.bottom() + ui.spacing().item_spacing.y),
            ],
            (1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
    }
    if row.selected {
        painter.line_segment(
            [
                rect.left_center() + egui::vec2(2.0, -9.0),
                rect.left_center() + egui::vec2(2.0, 9.0),
            ],
            (2.0, Color32::from_rgb(135, 222, 194)),
        );
    }
    if body.has_focus() || toggle.as_ref().is_some_and(Response::has_focus) {
        painter.rect_stroke(
            rect.shrink(1.0),
            3.0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let color = if row.selected {
        ui.visuals().selection.stroke.color
    } else if ui.visuals().dark_mode {
        Color32::from_gray(218)
    } else {
        ui.visuals().text_color()
    };
    let muted = secondary_color(ui);
    if let Some(open) = row.expanded {
        let center = toggle_rect.center();
        let points = if open {
            [
                center + egui::vec2(-4.0, -2.0),
                center + egui::vec2(0.0, 2.0),
                center + egui::vec2(4.0, -2.0),
            ]
        } else {
            [
                center + egui::vec2(-2.0, -4.0),
                center + egui::vec2(2.0, 0.0),
                center + egui::vec2(-2.0, 4.0),
            ]
        };
        painter.add(egui::Shape::line(
            points.to_vec(),
            egui::Stroke::new(1.4, muted),
        ));
    }
    paint_icon(
        ui,
        row.icon,
        body_rect.left_center() + egui::vec2(8.0, 0.0),
        if row.selected {
            Color32::from_rgb(135, 222, 194)
        } else {
            muted
        },
    );
    let suffix = row.suffix.map(|text| {
        painter.layout_no_wrap(
            text,
            egui::FontId::proportional(12.0),
            if row.status {
                if ui.visuals().dark_mode {
                    Color32::from_rgb(225, 182, 112)
                } else {
                    Color32::from_rgb(117, 76, 17)
                }
            } else {
                muted
            },
        )
    });
    let right = rect.right() - 7.0;
    let name_right = right - suffix.as_ref().map_or(0.0, |suffix| suffix.size().x + 8.0);
    let name_rect = Rect::from_min_max(
        egui::pos2(body_rect.left() + 22.0, rect.top()),
        egui::pos2(name_right.max(body_rect.left() + 22.0), rect.bottom()),
    );
    let mut job = egui::text::LayoutJob::simple(
        row.name.to_owned(),
        egui::FontId::proportional(14.0),
        color,
        name_rect.width(),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let name = painter.layout_job(job);
    painter.with_clip_rect(name_rect).galley(
        egui::pos2(name_rect.left(), rect.center().y - name.size().y / 2.0),
        name,
        color,
    );
    if let Some(suffix) = suffix {
        painter.galley(
            egui::pos2(
                right - suffix.size().x,
                rect.center().y - suffix.size().y / 2.0,
            ),
            suffix,
            muted,
        );
    }
    RowResponse { body, toggle }
}

fn paint_icon(ui: &egui::Ui, icon: Icon, center: egui::Pos2, color: Color32) {
    let painter = ui.painter();
    let stroke = egui::Stroke::new(1.2, color);
    match icon {
        Icon::Folder(open) => {
            let points = [
                (-7.0, -5.0),
                (-2.0, -5.0),
                (0.0, -3.0),
                (7.0, -3.0),
                (7.0, 5.0),
                (-7.0, 5.0),
            ]
            .map(|(x, y)| center + egui::vec2(x, y));
            painter.add(egui::Shape::closed_line(points.to_vec(), stroke));
            if open {
                painter.line_segment(
                    [
                        center + egui::vec2(-5.0, -1.0),
                        center + egui::vec2(8.0, -1.0),
                    ],
                    stroke,
                );
            }
        }
        Icon::Source => {
            let points = [
                (-5.0, -6.0),
                (5.0, -6.0),
                (8.0, 3.0),
                (7.0, 6.0),
                (-7.0, 6.0),
                (-8.0, 3.0),
            ]
            .map(|(x, y)| center + egui::vec2(x, y));
            painter.add(egui::Shape::closed_line(points.to_vec(), stroke));
            painter.line_segment(
                [
                    center + egui::vec2(-7.0, 2.0),
                    center + egui::vec2(7.0, 2.0),
                ],
                stroke,
            );
            painter.circle_filled(center + egui::vec2(5.0, 4.0), 0.9, color);
        }
        Icon::All => {
            for offset in [-4.0, 0.0, 4.0] {
                let points = [(-7.0, offset), (0.0, offset + 3.0), (7.0, offset)]
                    .map(|(x, y)| center + egui::vec2(x, y));
                painter.add(egui::Shape::line(points.to_vec(), stroke));
            }
        }
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/libraries/tree.rs"
    ));
}
