use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::collections::{Reference, ReferenceSelection};
use eframe::egui::{self, Event, Key, Modifiers};

use super::SoundKey;
use crate::search::{
    SelectionSources,
    selection::{MAX_SELECTION_BYTES, MAX_SELECTION_KEYS},
};

#[derive(Clone, Copy, Debug)]
pub(super) enum Gesture {
    Replace,
    Toggle,
    Extend { additive: bool },
}

impl Gesture {
    pub(super) fn from_modifiers(modifiers: Modifiers) -> Self {
        if modifiers.shift {
            Self::Extend {
                additive: modifiers.command || modifiers.ctrl,
            }
        } else if modifiers.command || modifiers.ctrl {
            Self::Toggle
        } else {
            Self::Replace
        }
    }
}

#[derive(Clone)]
struct Cursor {
    key: SoundKey,
    source: PathBuf,
    index: usize,
}

#[derive(Clone)]
pub(super) struct RangeRequest {
    pub generation: u64,
    pub anchor: PathBuf,
    pub target: PathBuf,
    sent: bool,
    additive: bool,
    first: usize,
    last: usize,
}

#[derive(Default)]
pub(super) struct Selection {
    // Ctrl+A is a view-wide intent, not a million eagerly loaded references.
    all: bool,
    keys: HashSet<PathBuf>,
    keys_bytes: usize,
    cursor: Option<Cursor>,
    anchor: Option<Cursor>,
    navigation: Option<(usize, Gesture)>,
    range: Option<RangeRequest>,
    generation: u64,
    scroll: Option<usize>,
    error: Option<String>,
    focus_id: Option<egui::Id>,
    active_reference: Option<Reference>,
}

#[derive(Clone)]
pub(super) struct SoundDrag {
    pub primary: Reference,
    pub selection: ReferenceSelection,
    pub count: usize,
}

impl Selection {
    pub(super) fn count(&self, total: usize) -> usize {
        if self.all {
            total.saturating_sub(self.keys.len())
        } else {
            self.keys.len()
        }
    }

    pub(super) fn snapshot(&self, total: usize) -> Result<crate::search::SourceSelection, String> {
        if self.range.is_some() || self.navigation.is_some() {
            return Err("Wait for the selection to finish, then try again".into());
        }
        let count = self.count(total);
        if count == 0 || count > crate::search::MAX_ACTION_SOUNDS {
            return Err(crate::search::SelectionError::InvalidSize.to_string());
        }
        let paths = self.keys.iter().cloned().collect();
        Ok(if self.all {
            crate::search::SourceSelection::AllExcept(paths)
        } else {
            crate::search::SourceSelection::Only(paths)
        })
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn forget(&mut self, source: &Path) {
        self.refresh();
        if self.keys.remove(source) {
            self.keys_bytes -= source.as_os_str().len();
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.source == source)
        {
            self.cursor = None;
        }
        if self
            .anchor
            .as_ref()
            .is_some_and(|anchor| anchor.source == source)
        {
            self.anchor = None;
        }
    }
    pub(super) fn reset(&mut self) {
        let generation = self.generation + 1;
        *self = Self {
            generation,
            ..Self::default()
        };
    }

    pub(super) fn refresh(&mut self) {
        if self.range.is_some() {
            self.error = Some("Results changed; select the range again.".into());
        }
        self.generation += 1;
        self.range = None;
        self.navigation = None;
        self.scroll = None;
    }

    pub(super) fn seed(&mut self, key: SoundKey, index: usize, source: &Path) {
        if self.cursor.is_none()
            && !self.all
            && self.keys.is_empty()
            && self.range.is_none()
            && self.navigation.is_none()
        {
            self.choose(key, index, source, Gesture::Replace);
        }
    }

    pub(super) fn observe(&mut self, key: SoundKey, index: usize, source: &Path) {
        if let Some(cursor) = &mut self.cursor
            && cursor.source == source
        {
            cursor.index = index;
            cursor.key = key;
        }
        if let Some(anchor) = &mut self.anchor
            && anchor.source == source
        {
            anchor.index = index;
            anchor.key = key;
        }
    }

    pub(super) fn active(&self) -> Option<SoundKey> {
        self.cursor.as_ref().map(|cursor| cursor.key)
    }

    pub(super) fn active_source(&self) -> Option<&Path> {
        self.cursor.as_ref().map(|cursor| cursor.source.as_path())
    }

    pub(super) fn remember(&mut self, reference: Reference) {
        if self.active_source() == Some(reference.source.as_path()) {
            self.active_reference = Some(reference);
        }
    }

    pub(super) fn reference(&self) -> Option<&Reference> {
        self.active_reference
            .as_ref()
            .filter(|reference| self.active_source() == Some(reference.source.as_path()))
    }

    pub(super) fn has_focus(&self, context: &egui::Context) -> bool {
        self.focus_id
            .is_some_and(|id| context.memory(|memory| memory.has_focus(id)))
    }

    pub(super) fn activate(&mut self, key: SoundKey, index: usize, source: &Path) {
        self.cursor = Some(Cursor {
            key,
            index,
            source: source.to_path_buf(),
        });
        self.navigation = None;
    }

    pub(super) fn contains(&self, source: &Path, index: usize) -> bool {
        if let Some(range) = &self.range {
            let in_range = (range.first..=range.last).contains(&index);
            if !range.additive {
                return in_range;
            }
            if in_range {
                return true;
            }
        }
        self.all != self.keys.contains(source)
    }

    pub(super) fn choose(&mut self, key: SoundKey, index: usize, source: &Path, gesture: Gesture) {
        self.generation += 1;
        self.range = None;
        self.navigation = None;
        self.error = None;
        let cursor = Cursor {
            key,
            index,
            source: source.to_path_buf(),
        };
        match gesture {
            Gesture::Replace => {
                self.all = false;
                self.keys.clear();
                self.keys_bytes = source.as_os_str().len();
                self.keys.insert(source.to_path_buf());
                self.anchor = Some(cursor.clone());
            }
            Gesture::Toggle => {
                if self.keys.remove(source) {
                    self.keys_bytes -= source.as_os_str().len();
                } else if self.keys_bytes + source.as_os_str().len() <= MAX_SELECTION_BYTES
                    && self.keys.len() < MAX_SELECTION_KEYS
                {
                    self.keys_bytes += source.as_os_str().len();
                    self.keys.insert(source.to_path_buf());
                } else {
                    self.error =
                        Some("Selection is too large. Use Ctrl+A to mark the whole view.".into());
                }
                self.anchor = Some(cursor.clone());
            }
            Gesture::Extend { additive } => {
                let anchor = self
                    .anchor
                    .get_or_insert_with(|| self.cursor.clone().unwrap_or_else(|| cursor.clone()));
                self.range = Some(RangeRequest {
                    generation: self.generation,
                    anchor: anchor.source.clone(),
                    target: source.to_path_buf(),
                    sent: false,
                    additive,
                    first: anchor.index.min(index),
                    last: anchor.index.max(index),
                });
            }
        }
        self.cursor = Some(cursor);
    }

    pub(super) fn request(&self) -> Option<RangeRequest> {
        self.range.as_ref().filter(|range| !range.sent).cloned()
    }

    pub(super) fn sent(&mut self) {
        if let Some(range) = &mut self.range {
            range.sent = true;
        }
    }

    pub(super) fn resolved(&mut self, generation: u64, result: Result<SelectionSources, String>) {
        let Some(range) = self
            .range
            .as_ref()
            .filter(|range| range.generation == generation)
            .cloned()
        else {
            return;
        };
        self.range = None;
        match result {
            Ok(SelectionSources::MissingEndpoint) => {
                self.error = Some("A range endpoint changed. Select the sounds again.".into())
            }
            Ok(SelectionSources::TooLarge) => {
                self.error =
                    Some("This range is too large. Use Ctrl+A to mark the whole view.".into())
            }
            Ok(SelectionSources::Ready(sources)) => {
                let (bytes, count) = sources
                    .iter()
                    .filter(|source| !range.additive || !self.keys.contains(*source))
                    .fold((0, 0), |(bytes, count), source| {
                        (bytes + source.as_os_str().len(), count + 1)
                    });
                let base_bytes = if range.additive { self.keys_bytes } else { 0 };
                let base_count = if range.additive { self.keys.len() } else { 0 };
                if !(range.additive && self.all)
                    && (base_bytes + bytes > MAX_SELECTION_BYTES
                        || base_count + count > MAX_SELECTION_KEYS)
                {
                    self.error =
                        Some("Selection is too large. Use Ctrl+A to mark the whole view.".into());
                    return;
                }
                if !range.additive {
                    self.keys.clear();
                    self.keys_bytes = 0;
                    self.all = false;
                }
                if !self.all {
                    for source in sources {
                        let bytes = source.as_os_str().len();
                        if self.keys.insert(source) {
                            self.keys_bytes += bytes;
                        }
                    }
                } else {
                    for source in sources {
                        if self.keys.remove(&source) {
                            self.keys_bytes -= source.as_os_str().len();
                        }
                    }
                }
            }
            Err(error) => {
                eprintln!("Sound selection failed: {error}");
                self.error = Some(format!("Could not mark sounds: {error}"));
            }
        }
    }

    pub(super) fn target(&self) -> Option<(usize, Gesture)> {
        self.navigation
    }

    pub(super) fn scroll_area(
        &mut self,
        ui: &egui::Ui,
        area: egui::ScrollArea,
        origin: usize,
    ) -> egui::ScrollArea {
        if let Some(index) = self
            .scroll
            .take()
            .or_else(|| self.navigation.map(|target| target.0))
        {
            let offset = index.saturating_sub(origin) as f32 * (28.0 + ui.spacing().item_spacing.y)
                - ui.available_height() * 0.5;
            area.vertical_scroll_offset(offset.max(0.0))
        } else {
            area
        }
    }

    /// One focus target survives virtualized rows and yields to text fields,
    /// menus, the folder tree, and ordinary controls.
    pub(super) fn keyboard(&mut self, ui: &mut egui::Ui, id: egui::Id, total: usize) -> bool {
        self.focus_id = Some(id);
        let response = ui.interact(
            ui.available_rect_before_wrap(),
            id,
            egui::Sense::focusable_noninteractive(),
        );
        if !response.has_focus() || egui::Popup::is_any_open(ui.ctx()) {
            return false;
        }
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        let mut favorite = false;
        ui.input_mut(|input| {
            input.events.retain(|event| {
                let Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    repeat,
                    ..
                } = event
                else {
                    return true;
                };
                if (modifiers.command || modifiers.ctrl)
                    && *key == Key::A
                    && !modifiers.alt
                    && !modifiers.shift
                {
                    if !repeat {
                        self.refresh();
                        self.all = true;
                        self.keys.clear();
                        self.keys_bytes = 0;
                        self.error = None;
                    }
                    return false;
                }
                if modifiers.alt || modifiers.command || modifiers.ctrl {
                    return true;
                }
                match key {
                    Key::W | Key::ArrowUp | Key::S | Key::ArrowDown if total > 0 => {
                        let current = self
                            .navigation
                            .map(|target| target.0)
                            .or_else(|| self.cursor.as_ref().map(|cursor| cursor.index));
                        let next = match (current, key) {
                            (None, _) => 0,
                            (Some(index), Key::W | Key::ArrowUp) => {
                                index.saturating_sub(1).min(total - 1)
                            }
                            (Some(index), _) => index.saturating_add(1).min(total - 1),
                        };
                        if current != Some(next) {
                            self.navigation = Some((
                                next,
                                if modifiers.shift {
                                    Gesture::Extend { additive: false }
                                } else {
                                    Gesture::Replace
                                },
                            ));
                            self.scroll = Some(next);
                        }
                        false
                    }
                    Key::F if !modifiers.shift => {
                        favorite |= !repeat;
                        false
                    }
                    _ => true,
                }
            })
        });
        favorite
    }

    pub(super) fn feedback(&self, ui: &mut egui::Ui, total: usize) {
        if self.range.is_some() {
            ui.label(egui::RichText::new("Marking sounds...").small().weak());
        } else {
            let count = self.count(total);
            if count > 1 {
                ui.label(
                    egui::RichText::new(format!("{count} sounds selected"))
                        .small()
                        .weak(),
                );
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(245, 143, 140), error);
        }
    }
}

pub(super) fn focus(ui: &egui::Ui, id: egui::Id) {
    ui.memory_mut(|memory| memory.request_focus(id));
}

pub(super) fn active_outline(ui: &egui::Ui, response: &egui::Response, active: bool) {
    if active {
        ui.painter().rect_stroke(
            response.rect.shrink(0.5),
            2.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(135, 222, 194)),
            egui::StrokeKind::Inside,
        );
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/selection.rs"
    ));
}
