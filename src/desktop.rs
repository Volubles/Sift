use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::audio::player::Player;
use crate::catalog::{Sample, SampleId};
use crate::collections::favorites::{Favorite, FavoriteId};
use crate::collections::{
    Availability, Entry, EntryId, Reference, worker::Worker as CollectionWorker,
};
use crate::library::ScanReport;
use crate::platform::{DragError, DragWorker};
use crate::search::PAGE_SIZE;
use crate::waveform::Waveforms;
use crate::worker::{Event, Workers};

mod diagnostics;
mod export;
mod filters;
mod tags;
use diagnostics::{CaptureStatus, Diagnostics};
mod collections;
use collections::Action as CollectionAction;
mod saved;
use saved::SavedSounds;
mod favorites;
use favorites::{Action as FavoriteAction, star_button};
mod libraries;
mod preview;
use preview::Preview;
mod library_view;
mod navigation;
mod pages;
mod selection;
use library_view::LibraryView;
mod sound_table;
use libraries::{Action as LibraryAction, Libraries};
use selection::{Gesture, SoundDrag};
use sound_table::{table_heading, table_row};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Sidebar {
    #[default]
    Libraries,
    Collections,
    Favorites,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SoundKey {
    Library(SampleId),
    Collection(EntryId),
    Favorite(FavoriteId),
}

pub struct Options {
    pub database: PathBuf,
    pub folder: Option<PathBuf>,
    pub smoke_screenshot: Option<PathBuf>,
}

pub fn run(options: Options) -> eframe::Result {
    let expects_capture = options.smoke_screenshot.is_some();
    let capture_passed = Arc::new(AtomicBool::new(false));
    let reported_capture = capture_passed.clone();
    let renderer = selected_renderer()?;
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([800.0, 500.0]),
        renderer,
        ..Default::default()
    };
    eframe::run_native(
        "Sift",
        native,
        Box::new(move |creation| {
            let context = creation.egui_ctx.clone();
            context.set_theme(egui::ThemePreference::Dark);
            let wake_context = context.clone();
            let workers = Workers::start(options.database.clone(), move || {
                wake_context.request_repaint()
            })?;
            let wave_context = context.clone();
            let waveforms = Waveforms::start(options.database.clone(), move || {
                wave_context.request_repaint()
            })?;
            let audio_context = context.clone();
            let player = Player::start(move || audio_context.request_repaint())?;
            let drag_context = context.clone();
            let drags = DragWorker::start_for_catalog(options.database.clone(), move || {
                drag_context.request_repaint()
            })?;
            let collection_context = context.clone();
            let collection_worker = CollectionWorker::start_with_catalog(
                crate::collections::data_path(&options.database),
                Some(options.database.clone()),
                move || collection_context.request_repaint(),
            )?;
            let tag_context = context.clone();
            let tag_worker =
                crate::tags::worker::Worker::start(options.database.clone(), move || {
                    tag_context.request_repaint()
                })?;
            Ok(Box::new(Browser::new(
                workers,
                waveforms,
                player,
                drags,
                SavedSounds::new(collection_worker, tag_worker),
                options,
                capture_passed,
            )))
        }),
    )?;
    if expects_capture && !reported_capture.load(Ordering::Acquire) {
        return Err(eframe::Error::AppCreation(Box::new(std::io::Error::other(
            "Native window smoke capture did not complete successfully",
        ))));
    }
    Ok(())
}

fn selected_renderer() -> eframe::Result<eframe::Renderer> {
    match std::env::var("SIFT_RENDERER").as_deref() {
        Ok("glow") | Err(std::env::VarError::NotPresent) => Ok(eframe::Renderer::Glow),
        #[cfg(feature = "wgpu-renderer")]
        Ok("wgpu") => Ok(eframe::Renderer::Wgpu),
        _ => Err(eframe::Error::AppCreation(Box::new(std::io::Error::other(
            "Unsupported SIFT_RENDERER; WGPU requires the wgpu-renderer Cargo feature",
        )))),
    }
}

struct Browser {
    workers: Workers,
    preview: Preview,
    library_view: LibraryView,
    drags: DragWorker,
    saved: SavedSounds,
    favorite_paths_dirty: bool,
    libraries: Libraries,
    sidebar: Sidebar,
    marked_view: Sidebar,
    startup_focus_pending: bool,
    drag_error: Option<String>,
    ready: bool,
    error: Option<String>,
    scan: Option<ScanReport>,
    scan_summary: Option<ScanReport>,
    show_details: bool,
    live_updates: bool,
    watch_message: String,
    automatic_scan: bool,
    last_index_refresh: Instant,
    folder: Option<PathBuf>,
    diagnostics: Diagnostics,
    export: export::Export,
}

impl Browser {
    fn new(
        workers: Workers,
        waveforms: Waveforms,
        player: Player,
        drags: DragWorker,
        saved: SavedSounds,
        options: Options,
        capture_passed: Arc<AtomicBool>,
    ) -> Self {
        let diagnostics = Diagnostics::new(options.smoke_screenshot, capture_passed);
        Self {
            export: export::Export::new(options.database.clone()),
            workers,
            preview: Preview::new(waveforms, player),
            library_view: LibraryView::default(),
            drags,
            saved,
            libraries: Libraries::default(),
            marked_view: Sidebar::Libraries,
            startup_focus_pending: true,
            sidebar: diagnostics.sidebar(),
            favorite_paths_dirty: true,
            drag_error: None,
            ready: false,
            error: None,
            scan: None,
            scan_summary: None,
            show_details: false,
            live_updates: cfg!(windows),
            watch_message: "Starting live updates...".into(),
            automatic_scan: false,
            last_index_refresh: Instant::now(),
            folder: options.folder,
            diagnostics,
        }
    }

    fn reset_query(&mut self) {
        self.favorite_paths_dirty = true;
        self.error = None;
        if let Some(offset) = self.library_view.reset_query(&self.workers) {
            self.request_page(offset);
        }
    }

    fn apply_library_filters(&mut self, filters: crate::search::filters::MetadataFilters) {
        match self.library_view.apply_filters(&self.workers, filters) {
            Ok(offset) => {
                self.error = None;
                self.favorite_paths_dirty = true;
                if let Some(offset) = offset {
                    self.request_page(offset);
                }
            }
            Err(error) => self.library_view.error = Some(error.to_string()),
        }
    }

    fn handle_tag_action(&mut self) {
        let Some(action) = self.saved.tags.take_action() else {
            return;
        };
        let mut filter = self.library_view.query.tags.clone();
        let mut label = None;
        match action {
            tags::Action::Toggle(tag, browse) => {
                if let Err(error) = filter.set(tag.id, !filter.contains(tag.id)) {
                    self.error = Some(error.to_string());
                    return;
                }
                if browse == tags::Browse::AllLibraries {
                    self.switch_sidebar(Sidebar::Libraries);
                    self.libraries.browse_all();
                    self.change_library_location();
                    self.library_view.query_text.clear();
                    self.library_view.query.filters = Default::default();
                }
                label = Some(tag);
            }
            tags::Action::RemoveFilter(id) => {
                if let Err(error) = filter.set(id, false) {
                    self.error = Some(error.to_string());
                    return;
                }
            }
            tags::Action::ClearFilters => filter = Default::default(),
        }
        self.library_view.query.tags = filter;
        self.saved.tags.set_filter(&self.library_view.query.tags);
        if let Some(tag) = label {
            self.saved.tags.remember_filter_label(&tag);
        }
        self.reset_query();
    }

    fn prepare_tag_target(&mut self) {
        self.saved.tags.set_filter(&self.library_view.query.tags);
        let count = self.marked_count();
        let preview = self.preview.selected();
        let key = if count == 0 {
            preview.map_or(tags::TargetKey::None, |sound| {
                tags::TargetKey::Sound(crate::path_codec::encode(&sound.source))
            })
        } else {
            let (selection_generation, view_generation, pending) = match self.marked_view {
                Sidebar::Libraries => (
                    self.library_view.selection.generation(),
                    self.library_view.generation,
                    self.library_view.selection.snapshot_pending(),
                ),
                Sidebar::Favorites => self.saved.favorites.tag_selection_key(),
                Sidebar::Collections => self.saved.collections.tag_selection_key(),
            };
            tags::TargetKey::Marks {
                view: self.marked_view,
                selection_generation,
                view_generation,
                pending,
                count,
                preview: preview.map(|sound| crate::path_codec::encode(&sound.source)),
            }
        };
        if self.saved.tags.target_key() == &key {
            return;
        }
        let (target, caption, browse) = if count > 0 {
            let target = if count > crate::search::MAX_ACTION_SOUNDS {
                Err(format!(
                    "Choose at most {} sounds for tag edits",
                    crate::search::MAX_ACTION_SOUNDS
                ))
            } else {
                self.marked_selection()
                    .map(|selection| Some(crate::tags::worker::Target::Selection(selection)))
            };
            (
                target,
                format!(
                    "{count} selected sound{}",
                    if count == 1 { "" } else { "s" }
                ),
                if self.marked_view == Sidebar::Libraries {
                    tags::Browse::CurrentLibrary
                } else {
                    tags::Browse::AllLibraries
                },
            )
        } else if let Some(sound) = preview {
            (
                Ok(Some(crate::tags::worker::Target::Sources(vec![
                    sound.source.clone(),
                ]))),
                sound.name.clone(),
                if matches!(self.preview.key(), Some(SoundKey::Library(_))) {
                    tags::Browse::CurrentLibrary
                } else {
                    tags::Browse::AllLibraries
                },
            )
        } else {
            (
                Ok(None),
                "Select a sound to edit tags".into(),
                tags::Browse::CurrentLibrary,
            )
        };
        self.saved.tags.set_target(key, target, caption, browse);
    }

    fn marked_count(&self) -> usize {
        match self.marked_view {
            Sidebar::Libraries => self.library_view.selection.count(self.library_view.total),
            Sidebar::Collections => self.saved.collections.marked_count(),
            Sidebar::Favorites => self.saved.favorites.selected_count(),
        }
    }

    fn change_library_location(&mut self) {
        if self.library_view.change(self.libraries.location()) {
            self.marked_view = Sidebar::Libraries;
            self.restore_library_preview();
        }
        self.refresh_query();
    }

    fn restore_library_preview(&mut self) {
        let saved = self
            .library_view
            .selection
            .active()
            .zip(self.library_view.selection.reference().cloned());
        self.restore_preview(saved);
    }

    fn restore_preview(&mut self, saved: Option<(SoundKey, Reference)>) {
        if let Some((key, reference)) = saved {
            self.preview.restore(reference, key);
        } else {
            self.preview.clear();
        }
        self.favorite_paths_dirty = true;
    }

    fn switch_sidebar(&mut self, mode: Sidebar) {
        if self.sidebar == mode {
            return;
        }
        self.startup_focus_pending = false;
        if self.sidebar == Sidebar::Collections {
            self.saved.collections.suspend_view(&self.saved.storage);
        }
        if mode == Sidebar::Favorites {
            self.library_view.selection.suspend();
            self.workers
                .invalidate_selection(self.library_view.selection.generation());
        }
        self.sidebar = mode;
        self.saved.favorites.set_active(mode == Sidebar::Favorites);
        match mode {
            Sidebar::Libraries => {
                self.marked_view = Sidebar::Libraries;
                self.restore_library_preview();
                self.refresh_query();
            }
            Sidebar::Favorites => {
                self.marked_view = Sidebar::Favorites;
                self.restore_preview(self.saved.favorites.active_reference());
            }
            Sidebar::Collections => {
                let saved = self.saved.collections.resume_view(&self.saved.storage);
                self.restore_preview(saved);
            }
        }
    }

    fn marked_selection(&self) -> Result<crate::collections::ReferenceSelection, String> {
        match self.marked_view {
            Sidebar::Libraries => Ok(crate::collections::ReferenceSelection {
                order: self.library_view.order,
                origin: crate::collections::SelectionOrigin::Library {
                    query: self.library_view.query.clone(),
                    scope: self.libraries.scope(),
                },
                sources: self
                    .library_view
                    .selection
                    .snapshot(self.library_view.total)?,
            }),
            Sidebar::Collections => self.saved.collections.selected_references(),
            Sidebar::Favorites => self.saved.favorites.selected_references(),
        }
    }

    fn sound_drag(&mut self, context: &egui::Context, primary: Reference, view: Sidebar) {
        if self.drags.busy() {
            return;
        }
        self.marked_view = view;
        match self.marked_selection() {
            Ok(selection) => {
                self.drag_error = None;
                egui::DragAndDrop::set_payload(
                    context,
                    SoundDrag {
                        primary,
                        selection,
                        count: self.marked_count(),
                    },
                );
            }
            Err(error) => self.drag_error = Some(error),
        }
    }

    fn refresh_query(&mut self) {
        self.favorite_paths_dirty = true;
        self.error = None;
        if let Some(offset) = self.library_view.refresh(&self.workers) {
            self.request_page(offset);
        }
    }

    fn reorder_list(&mut self) {
        self.library_view.reorder(&self.workers);
        self.request_list_positions();
        self.request_page(0);
    }

    fn request_list_positions(&mut self) {
        if self.ready {
            self.library_view
                .request_positions(&self.workers, self.libraries.scope());
        }
    }

    fn request_page(&mut self, offset: usize) {
        if self.ready && self.error.is_none() {
            self.library_view
                .request_page(&self.workers, self.libraries.scope(), offset);
        }
    }

    fn begin_scan(&mut self, folder: PathBuf) {
        match self.workers.scan(folder) {
            Ok(()) => {
                self.automatic_scan = false;
                self.scan = Some(ScanReport::default());
                self.scan_summary = None;
                self.error = None;
            }
            Err(message) => {
                self.error = Some(message);
            }
        }
    }

    fn accept_events(&mut self, ctx: &egui::Context) {
        self.export.poll();
        if self.drags.busy() && ctx.input_mut(cancel_file_drag_requested) {
            self.drags.cancel();
        }
        let tags_changed = self.saved.poll();
        let definition_changed = self
            .saved
            .tags
            .take_management_change()
            .is_some_and(|change| self.library_view.tags_changed(&change));
        if definition_changed || (tags_changed && !self.library_view.query.tags.is_empty()) {
            self.reset_query();
        }
        if self.sidebar == Sidebar::Collections
            && let Some(saved) = self.saved.collections.take_preview_restore()
        {
            self.restore_preview(saved);
        }
        if let Some(SoundKey::Collection(id)) = self.preview.key()
            && let Some(entry) = self.saved.collections.selected_update(id)
        {
            self.preview.update_availability(&entry.availability);
        }
        while let Some(result) = self.drags.poll() {
            // OLE consumes the final mouse release in its modal loop. Reset egui
            // so the next click/drag cannot inherit a stuck pointer or widget ID.
            ctx.stop_dragging();
            egui::DragAndDrop::clear_payload(ctx);
            ctx.input_mut(|input| input.pointer = Default::default());
            self.drag_error = match result {
                // Acceptance, Escape, and unsuitable drop locations are ordinary
                // gesture completion; only actual failures need a notification.
                Ok(_) => None,
                Err(error) => {
                    if matches!(
                        error,
                        DragError::Native(_) | DragError::Disconnected | DragError::WrongThread
                    ) {
                        eprintln!("Native file drag failed: {error}");
                    }
                    Some(format!("Could not drag the file: {error}"))
                }
            };
        }
        while let Some(event) = self.workers.try_event() {
            match event {
                Event::Positions { generation, result } => {
                    if let Some(index) = self
                        .library_view
                        .selection
                        .positions_resolved(generation, result)
                    {
                        self.request_page(index / PAGE_SIZE * PAGE_SIZE);
                    }
                }
                Event::Selection { generation, result } => {
                    self.library_view.selection.resolved(generation, result)
                }
                Event::Ready => {
                    self.ready = true;
                    self.request_page(0);
                    if let Some(folder) = self.folder.take() {
                        self.libraries.add_initial_folder(&self.workers, folder);
                    }
                }
                Event::Libraries(result) => {
                    let changed = self.libraries.snapshot(result);
                    let scoped = if let Some(relative) = self.diagnostics.take_subfolder() {
                        if self.libraries.select_smoke_subfolder(relative) {
                            true
                        } else {
                            eprintln!(
                                "Smoke folder must be a relative subfolder of the active library's first source"
                            );
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            false
                        }
                    } else {
                        false
                    };
                    if changed || scoped {
                        self.change_library_location();
                    }
                    self.library_view
                        .retain(|location| self.libraries.valid_location(location));
                }
                Event::LibraryEdited { token, result } => {
                    if let Some(folder) = self.libraries.edited(&self.workers, token, result) {
                        self.begin_scan(folder);
                    }
                }
                Event::Folders { generation, result } => {
                    self.libraries.folder_page(generation, result)
                }
                Event::Page { generation, result } => {
                    if let Some(rows) = self.library_view.page(generation, result) {
                        if self.sidebar != Sidebar::Favorites {
                            self.preview.refresh_catalog(rows);
                        }
                        self.favorite_paths_dirty = true;
                    }
                }
                Event::Progress(report) => {
                    if report.indexed > 0
                        && self.last_index_refresh.elapsed() >= Duration::from_millis(250)
                    {
                        self.refresh_query();
                        self.last_index_refresh = Instant::now();
                    }
                    self.scan = Some(report);
                }
                Event::ScanFinished(result) => {
                    self.workers.finish_scan();
                    self.scan = None;
                    match result {
                        Ok(report) => {
                            self.scan_summary = Some(report);
                            self.refresh_query();
                        }
                        Err(message) => {
                            eprintln!("Sift indexing failed: {message}");
                            self.error = Some(format!("Indexing failed: {message}"));
                        }
                    }
                }
                Event::SyncStarted => {
                    self.scan = Some(ScanReport::default());
                    self.automatic_scan = true;
                }
                Event::SyncFinished {
                    report,
                    touched,
                    full_roots,
                    forced_roots,
                } => {
                    self.scan = None;
                    self.automatic_scan = false;
                    if report.changed() {
                        self.refresh_query();
                    }
                    self.saved.sources_changed(
                        &touched
                            .iter()
                            .chain(&full_roots)
                            .cloned()
                            .collect::<Vec<_>>(),
                    );
                    self.preview
                        .sources_changed(&touched, &full_roots, &forced_roots);
                    if report.errors > 0 || report.incomplete {
                        self.scan_summary = Some(report);
                    }
                }
                Event::WatchStatus { enabled, message } => {
                    self.live_updates = enabled;
                    self.watch_message = message;
                }
                Event::SourceReady {
                    generation,
                    refresh,
                    sample,
                    stamp,
                    availability,
                } => {
                    self.favorite_paths_dirty |=
                        self.preview
                            .source_ready(generation, refresh, sample, stamp, availability);
                }
                Event::StartupFailed(message) => {
                    eprintln!("Sift catalog startup failed: {message}");
                    self.error = Some(format!("Catalog could not open: {message}"));
                }
            }
        }
        self.preview.queue_source_check(&self.workers);
        if self.favorite_paths_dirty || self.saved.collections.favorite_paths_dirty {
            let mut paths: BTreeSet<_> = self
                .library_view
                .pages
                .iter()
                .flat_map(|page| &page.rows)
                .map(|sample| sample.source.clone())
                .collect();
            if let Some(selected) = self.preview.selected() {
                paths.insert(selected.source.clone());
            }
            self.saved.track_favorites(paths);
            self.favorite_paths_dirty = false;
        }
        self.preview.poll();
    }

    fn select_sample(&mut self, sample: Sample) {
        let stamp = sample.stamp.clone();
        self.preview.select(
            Reference::from(&sample),
            SoundKey::Library(sample.id),
            Availability::Ready,
            stamp,
        );
        self.favorite_paths_dirty = true;
    }

    fn select_favorite(&mut self, favorite: Favorite) {
        self.select(
            favorite.reference,
            SoundKey::Favorite(favorite.id),
            favorite.availability,
        );
    }

    fn select_entry(&mut self, entry: Entry) {
        self.select(
            entry.reference,
            SoundKey::Collection(entry.id),
            entry.availability,
        );
    }

    fn select(&mut self, sample: Reference, key: SoundKey, availability: Availability) {
        self.preview.select(sample, key, availability, None);
        self.favorite_paths_dirty = true;
    }

    fn begin_drag(&mut self, sample: Reference) {
        let result = self.drags.request(vec![sample.source]);
        self.drag_requested(result);
    }

    fn begin_group_drag(&mut self, selection: crate::collections::ReferenceSelection) {
        let result = self.drags.request_selection(selection);
        self.drag_requested(result);
    }

    fn drag_requested(&mut self, result: Result<(), DragError>) {
        self.drag_error = match result {
            Ok(()) => None,
            Err(error) => {
                if matches!(
                    error,
                    DragError::Native(_) | DragError::Disconnected | DragError::WrongThread
                ) {
                    eprintln!("Native file drag could not start: {error}");
                }
                Some(format!("Could not drag the file: {error}"))
            }
        };
    }

    fn handoff_file_drag(&mut self, context: &egui::Context) {
        if self.drags.busy() || !DragWorker::supported() || !context.input(external_drag_requested)
        {
            return;
        }
        if egui::DragAndDrop::has_payload_of_type::<SoundDrag>(context) {
            if let Some(drag) = egui::DragAndDrop::take_payload::<SoundDrag>(context) {
                self.begin_group_drag(Arc::unwrap_or_clone(drag).selection);
            }
        } else if egui::DragAndDrop::has_payload_of_type::<Reference>(context)
            && let Some(reference) = egui::DragAndDrop::take_payload::<Reference>(context)
        {
            self.begin_drag(Arc::unwrap_or_clone(reference));
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("SIFT")
                    .size(25.0)
                    .strong()
                    .color(Color32::from_rgb(135, 222, 194)),
            );
            ui.add_space(12.0);
            ui.label(RichText::new("Your sounds, within reach.").weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut self.show_details, "Details");
                let previous = self.live_updates;
                if ui
                    .checkbox(&mut self.live_updates, "Live updates")
                    .on_hover_text(&self.watch_message)
                    .changed()
                    && let Err(error) = self.workers.set_live_updates(self.live_updates)
                {
                    self.live_updates = previous;
                    self.watch_message = error;
                }
                if self.scan.is_some()
                    && ui
                        .button(if self.automatic_scan {
                            "Cancel update"
                        } else {
                            "Cancel scan"
                        })
                        .clicked()
                {
                    self.workers.cancel_scan();
                }
            });
        });
        if self.sidebar == Sidebar::Favorites {
            ui.label(RichText::new("Favorites").small().strong());
        } else if self.libraries.breadcrumb_ui(ui) {
            self.change_library_location();
        }
        if !self.live_updates {
            ui.label(RichText::new(&self.watch_message).small().weak());
        }
        ui.add_space(12.0);
        let favorites = self.sidebar == Sidebar::Favorites;
        let (response, filter_change) = ui
            .horizontal(|ui| {
                let query_text = if favorites {
                    &mut self.saved.favorites.query_text
                } else {
                    &mut self.library_view.query_text
                };
                let width = if favorites {
                    ui.available_width()
                } else {
                    (ui.available_width()
                        - filters::BUTTON_WIDTH
                        - tags::BUTTON_WIDTH
                        - 2.0 * ui.spacing().item_spacing.x)
                        .max(80.0)
                };
                let response = ui.add_sized(
                    [width, 34.0],
                    egui::TextEdit::singleline(query_text)
                        .hint_text("Search sounds by name or folder..."),
                );
                let changed = if favorites {
                    None
                } else {
                    self.library_view
                        .filter_editor
                        .ui(ui, self.library_view.query.filters)
                };
                if !favorites {
                    self.saved.tags.filter_button(ui);
                }
                (response, changed)
            })
            .inner;
        if let Some(filters) = filter_change {
            self.apply_library_filters(filters);
        }
        if !favorites && let Some(filters) = filters::active(ui, self.library_view.query.filters) {
            self.apply_library_filters(filters);
        }
        if !favorites {
            self.saved.tags.active_filters(ui);
            self.handle_tag_action();
        }
        if ui.is_enabled()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::F))
        {
            response.request_focus();
        }
        if response.changed() {
            if favorites {
                self.saved.favorites.reset_query();
            } else {
                self.reset_query();
            }
        }
        ui.horizontal(|ui| {
            let total = if favorites {
                self.saved.favorites.total
            } else {
                self.library_view.total
            };
            let count_label = if favorites {
                "favorites"
            } else if self.library_view.query_text.is_empty()
                && self.library_view.query.filters.active_count() == 0
                && self.library_view.query.tags.is_empty()
            {
                "indexed sounds"
            } else {
                "matching sounds"
            };
            ui.label(
                RichText::new(format!("{total} {count_label}"))
                    .small()
                    .weak(),
            );
            ui.separator();
            ui.label(
                RichText::new("3+ characters: contains words  /  1-2 characters: filename prefix")
                    .small()
                    .weak(),
            );
        });
        if let Some(message) = &self.error {
            ui.colored_label(Color32::from_rgb(245, 143, 140), message);
        }
        if let Some(message) = &self.library_view.error {
            ui.colored_label(Color32::from_rgb(245, 143, 140), message);
        }
        self.saved.favorite_feedback(ui);
        if self.drags.busy() {
            ui.horizontal(|ui| {
                ui.label(if self.drags.native_active() {
                    "Dragging copies. Release in a folder, or press Esc to cancel."
                } else {
                    "Preparing copies. Keep the left mouse button held; Esc cancels."
                });
                if ui.button("Cancel file drag").clicked() {
                    self.drags.cancel();
                }
            });
        } else if let Some(message) = &self.drag_error {
            let dismiss = ui
                .horizontal(|ui| {
                    ui.colored_label(Color32::from_rgb(245, 143, 140), message);
                    ui.small_button("Dismiss").clicked()
                })
                .inner;
            if dismiss {
                self.drag_error = None;
            }
        }
        if let Some(scan) = &self.scan {
            ui.label(format!(
                "Indexing: {} checked, {} updated, {} unchanged, {} issues",
                scan.visited, scan.indexed, scan.unchanged, scan.errors
            ));
        }
        if let Some(report) = &self.scan_summary {
            let outcome = if report.cancelled {
                "Scan cancelled"
            } else if report.incomplete || report.deferred > 0 {
                "Scan incomplete"
            } else {
                "Scan finished"
            };
            ui.label(format!(
                "{outcome}: {} updated, {} unchanged, {} removed, {} deferred, {} issues",
                report.indexed, report.unchanged, report.removed, report.deferred, report.errors
            ));
            if !report.issues.is_empty() {
                egui::CollapsingHeader::new("Scan issues").show(ui, |ui| {
                    for issue in &report.issues {
                        ui.label(format!("{}: {}", issue.path.display(), issue.message));
                    }
                });
            }
        }
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Sounds");
        });
        ui.separator();
        if DragWorker::supported() {
            ui.label(
                RichText::new("Drag to a collection to save it, or out to a folder to copy it.")
                    .small()
                    .weak(),
            );
        }
        self.request_list_positions();
        if !self.ready {
            ui.label(if self.error.is_some() {
                "Catalog unavailable."
            } else {
                "Opening catalog..."
            });
            return;
        }
        if self.library_view.total == 0 && self.library_view.pages.is_empty() {
            self.request_page(0);
        }
        if self.library_view.total == 0 {
            if self.library_view.pages.has_pending() {
                ui.label("Searching...");
            } else if self.library_view.query_text.is_empty()
                && self.library_view.query.filters.active_count() == 0
                && self.library_view.query.tags.is_empty()
            {
                ui.label(
                    "No indexed sounds in this view. Add or rescan a source folder in Libraries.",
                );
            } else {
                ui.label("No matching sounds.");
            }
            return;
        }
        let mut needed = BTreeSet::new();
        let focus_id = ui.make_persistent_id("library-sound-focus");
        if std::mem::take(&mut self.startup_focus_pending)
            && self.sidebar == Sidebar::Libraries
            && ui.memory(|memory| memory.focused().is_none())
            && !egui::Popup::is_any_open(ui.ctx())
        {
            selection::focus(ui, focus_id);
        }
        let favorite_key =
            self.library_view
                .selection
                .keyboard(ui, focus_id, self.library_view.total);
        let target = self.library_view.selection.target();
        let mut choice = target.and_then(|(index, gesture)| {
            self.library_view
                .pages
                .iter()
                .find(|page| index >= page.offset && index < page.offset + page.rows.len())
                .and_then(|page| page.rows.get(index - page.offset))
                .map(|sample| (sample.clone(), index, gesture))
        });
        let mut drag_sample = None;
        let mut favorite = None;
        let mut export_requested = false;
        self.library_view
            .selection
            .feedback(ui, self.library_view.total);
        if table_heading(ui, &mut self.library_view.order) {
            self.reorder_list();
            selection::focus(ui, focus_id);
            return;
        }
        let scroll = self
            .library_view
            .selection
            .scroll_area(
                ui,
                egui::ScrollArea::vertical()
                    .id_salt(("samples", self.library_view.view_generation))
                    .auto_shrink([false, false]),
                0,
                ui.available_height(),
            )
            .show_rows(ui, 28.0, self.library_view.total, |ui, range| {
                let first_visible = range.start;
                for index in range {
                    let offset = index / PAGE_SIZE * PAGE_SIZE;
                    needed.insert(offset);
                    if let Some(sample) = self
                        .library_view
                        .pages
                        .iter()
                        .find(|page| page.offset == offset)
                        .and_then(|page| page.rows.get(index - offset))
                    {
                        let key = SoundKey::Library(sample.id);
                        if self.preview.key() == Some(key) {
                            self.library_view.selection.seed(key, index, &sample.source);
                            if self.library_view.selection.reference().is_none() {
                                self.library_view
                                    .selection
                                    .remember(Reference::from(sample));
                            }
                        }
                        let row_id = ui.make_persistent_id(("library-row", sample.id.0));
                        let response = ui
                            .scope_builder(egui::UiBuilder::new().id(row_id), |ui| {
                                ui.horizontal(|ui| {
                                    if star_button(
                                        ui,
                                        self.saved.favorites.status(&sample.source),
                                        self.saved.favorites.can_edit(),
                                        &sample.name,
                                    )
                                    .clicked()
                                    {
                                        favorite = Some(Reference::from(sample));
                                    }
                                    table_row(
                                        ui,
                                        sample,
                                        self.library_view.selection.contains(&sample.source, index),
                                        index,
                                    )
                                })
                                .inner
                            })
                            .inner;
                        let tag_clicked = self.saved.tags.row(
                            ui,
                            &sample.source,
                            sound_table::tag_rect(response.rect),
                            tags::Browse::CurrentLibrary,
                        );
                        selection::active_outline(
                            ui,
                            &response,
                            self.library_view.selection.active_source()
                                == Some(sample.source.as_path()),
                        );
                        if response.secondary_clicked() {
                            self.marked_view = Sidebar::Libraries;
                            if !self.library_view.selection.contains(&sample.source, index) {
                                self.library_view.selection.choose(
                                    key,
                                    index,
                                    &sample.source,
                                    Gesture::Replace,
                                );
                                self.library_view
                                    .selection
                                    .remember(Reference::from(sample));
                            }
                        }
                        response.context_menu(|ui| {
                            if ui.button("Export selected...").clicked() {
                                export_requested = true;
                                ui.close();
                            }
                        });
                        if response.clicked() && !tag_clicked {
                            choice = Some((
                                sample.clone(),
                                index,
                                Gesture::from_modifiers(ui.input(|input| input.modifiers)),
                            ));
                            selection::focus(ui, focus_id);
                        }
                        if DragWorker::supported()
                            && !self.drags.busy()
                            && response.drag_started_by(egui::PointerButton::Primary)
                        {
                            drag_sample = Some((sample.clone(), index));
                        }
                        response.on_hover_text(format!(
                            "{}\nDrag to a collection, or out to copy the source file.",
                            sample.display_path
                        ));
                    } else {
                        ui.add_sized(
                            [ui.available_width(), 32.0],
                            egui::Label::new("Loading sounds..."),
                        );
                    }
                }
                first_visible
            });
        self.library_view
            .selection
            .remember_scroll(scroll.state.offset.y, scroll.inner);
        if export_requested {
            self.marked_view = Sidebar::Libraries;
            self.export.open(
                self.marked_selection()
                    .map(crate::export::worker::Input::Selection),
                ui.ctx(),
            );
        }
        if let Some(reference) = favorite {
            self.saved.toggle_favorite(&reference);
        }
        if let Some((sample, index, gesture)) = choice {
            self.marked_view = Sidebar::Libraries;
            if !self.library_view.selection.choose(
                SoundKey::Library(sample.id),
                index,
                &sample.source,
                gesture,
            ) {
                return;
            }
            self.library_view
                .selection
                .remember(Reference::from(&sample));
            self.select_sample(sample);
        }
        if favorite_key
            && let Some(reference) = self
                .preview
                .selected()
                .filter(|reference| {
                    self.library_view.selection.active_source() == Some(reference.source.as_path())
                })
                .or_else(|| self.library_view.selection.reference())
        {
            self.saved.toggle_favorite(reference);
        }
        if let Some((sample, index)) = drag_sample {
            let key = SoundKey::Library(sample.id);
            if !self.library_view.selection.contains(&sample.source, index) {
                self.library_view
                    .selection
                    .choose(key, index, &sample.source, Gesture::Replace);
            }
            self.library_view
                .selection
                .activate(key, index, &sample.source);
            selection::focus(ui, focus_id);
            let reference = Reference::from(&sample);
            self.library_view.selection.remember(reference.clone());
            self.select_sample(sample);
            self.sound_drag(ui.ctx(), reference, Sidebar::Libraries);
        }
        for offset in needed {
            self.request_page(offset);
        }
        if let Some((index, _)) = target {
            self.request_page(index / PAGE_SIZE * PAGE_SIZE);
        }
        if let Some(request) = self.library_view.selection.request()
            && self.workers.selection_ordered(
                request.generation,
                self.library_view.query.clone(),
                self.libraries.scope(),
                self.library_view.order,
                request.anchor,
                request.target,
            )
        {
            self.library_view.selection.sent();
        }
    }

    fn smoke_check(&mut self, context: &egui::Context) {
        if !self.diagnostics.capturing() {
            return;
        }
        let ready = self.ready
            && self.saved.settled()
            && self.saved.tags.settled()
            && (self.sidebar != Sidebar::Libraries || self.libraries.settled())
            && self.scan.is_none()
            && !self.preview.waveform_pending()
            && !self.library_view.pages.is_empty();
        self.diagnostics.poll_capture(
            context,
            CaptureStatus {
                ready,
                results: self.library_view.total,
                cached_records: self
                    .library_view
                    .pages
                    .iter()
                    .map(|page| page.rows.len())
                    .sum(),
                error: self.error.as_deref().or(self.library_view.error.as_deref()),
            },
        );
    }
}

fn navigation_button(ui: &mut egui::Ui, mode: Sidebar, selected: bool) -> egui::Response {
    let name = match mode {
        Sidebar::Libraries => "Libraries",
        Sidebar::Collections => "Collections",
        Sidebar::Favorites => "Favorites",
    };
    let response = ui
        .add_sized([32.0, 38.0], egui::Button::new("").selected(selected))
        .on_hover_text(name);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, name)
    });
    let centre = response.rect.center();
    let color = if selected {
        Color32::from_rgb(135, 222, 194)
    } else {
        Color32::from_gray(155)
    };
    let painter = ui.painter();
    match mode {
        Sidebar::Libraries => {
            let points = vec![
                centre + egui::vec2(-7.0, -9.0),
                centre + egui::vec2(7.0, -9.0),
                centre + egui::vec2(10.0, 6.0),
                centre + egui::vec2(-10.0, 6.0),
            ];
            painter.add(egui::Shape::convex_polygon(
                points,
                color.gamma_multiply(0.25),
                egui::Stroke::new(1.5, color),
            ));
            painter.line_segment(
                [
                    centre + egui::vec2(-9.0, 10.0),
                    centre + egui::vec2(9.0, 10.0),
                ],
                (2.0, color),
            );
        }
        Sidebar::Favorites => {
            let points: Vec<_> = (0..10)
                .map(|point| {
                    let angle =
                        std::f32::consts::TAU * point as f32 / 10.0 - std::f32::consts::FRAC_PI_2;
                    let radius = if point % 2 == 0 { 11.0 } else { 5.0 };
                    centre + egui::vec2(angle.cos(), angle.sin()) * radius
                })
                .collect();
            painter.add(egui::Shape::closed_line(
                points,
                egui::Stroke::new(1.5, color),
            ));
        }
        Sidebar::Collections => {
            for row in 0..3 {
                for column in 0..3 {
                    let min =
                        centre + egui::vec2(-9.0 + column as f32 * 7.0, -9.0 + row as f32 * 7.0);
                    painter.rect_filled(
                        egui::Rect::from_min_size(min, egui::vec2(5.0, 5.0)),
                        0.0,
                        color,
                    );
                }
            }
        }
    }
    response
}

impl eframe::App for Browser {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.diagnostics.finish(self.preview.playback_phase());
    }
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.accept_events(context);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.saved.tags.begin_frame();
        if self.saved.tags.management_open() || self.export.is_open() {
            ui.disable();
        }
        self.diagnostics.frame(ui.ctx());
        self.saved
            .favorites
            .set_active(self.sidebar == Sidebar::Favorites);
        let ctx = ui.ctx().clone();
        // Keep the most recently used sound list when a sidebar/button takes
        // focus. Opening Collections must not substitute its first cached row.
        if self.library_view.selection.has_focus(&ctx) {
            self.marked_view = Sidebar::Libraries;
        } else if self.saved.favorites.has_focus(&ctx) {
            self.marked_view = Sidebar::Favorites;
        } else if self.saved.sound_has_focus(&ctx) {
            self.marked_view = Sidebar::Collections;
        }
        if self.drags.busy() && ctx.input_mut(cancel_file_drag_requested) {
            self.drags.cancel();
        }
        egui::Panel::bottom("footer").show(ui, |ui| {
            self.export.status(ui);
            ui.label(
                RichText::new(
                    "Source files stay in place. Collections and favorites save automatically.",
                )
                .small()
                .weak(),
            );
        });
        egui::Panel::left("activity-rail")
            .resizable(false)
            .exact_size(44.0)
            .show(ui, |ui| {
                for mode in [Sidebar::Libraries, Sidebar::Collections, Sidebar::Favorites] {
                    if navigation_button(ui, mode, self.sidebar == mode).clicked() {
                        self.switch_sidebar(mode);
                    }
                }
            });
        egui::Panel::left("browser-sidebar")
            .resizable(true)
            .min_size(220.0)
            .default_size(260.0)
            .show(ui, |ui| {
                if self.sidebar == Sidebar::Libraries {
                    if let Some(action) = self.libraries.ui(ui, &self.workers, self.scan.is_some())
                    {
                        match action {
                            LibraryAction::ScopeChanged => self.change_library_location(),
                            LibraryAction::Scan(folder) => self.begin_scan(folder),
                        }
                    }
                    return;
                }
                if self.sidebar == Sidebar::Favorites {
                    ui.heading("Favorites");
                    ui.separator();
                    ui.label("Starred sounds from all libraries and collections.");
                    ui.label("Select sounds, then drag the group out to copy it or switch to Collections to save references.");
                    if ui.button("Refresh favorites").clicked() { self.saved.favorites.reset_query(); }
                    return;
                }
                let selected_entry = match self.preview.key() {
                    Some(SoundKey::Collection(id)) => Some(id),
                    _ => None,
                };
                let action = self.saved.ui(ui, self.preview.selected(), selected_entry, self.marked_count());
                if let Some(saved) = self.saved.collections.take_preview_restore() { self.restore_preview(saved); }
                if let Some(action) = action {
                    match action {
                        CollectionAction::Select(entry) => {
                            self.marked_view = Sidebar::Collections;
                            self.select_entry(entry);
                        }
                        CollectionAction::Drag(entry) => {
                            let reference = entry.reference.clone();
                            self.select_entry(entry);
                            self.sound_drag(&ctx, reference, Sidebar::Collections);
                        }
                        CollectionAction::AddMarked(collection) => {
                            let selection = self.marked_selection();
                            self.saved.collections.add_selected(&mut self.saved.storage, collection, selection);
                        }
                        CollectionAction::Export(selection) => {
                            self.marked_view = Sidebar::Collections;
                            self.export.open(selection.map(crate::export::worker::Input::Selection), &ctx);
                        }
                    }
                }
            });
        egui::Panel::top("header").show(ui, |ui| self.header(ui));
        egui::Panel::top("preview").show(ui, |ui| self.preview.ui(ui));
        self.prepare_tag_target();
        egui::Panel::top("tag-editor").show(ui, |ui| self.saved.tags.panel(ui));
        if self.show_details {
            egui::Panel::right("details")
                .resizable(true)
                .default_size(300.0)
                .show(ui, |ui| {
                    self.preview
                        .details(ui, DragWorker::supported() && !self.drags.busy())
                });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if self.sidebar == Sidebar::Favorites {
                let selected = match self.preview.key() {
                    Some(SoundKey::Favorite(id)) => Some(id),
                    _ => None,
                };
                if let Some(action) =
                    self.saved
                        .favorite_list(ui, selected, self.preview.selected())
                {
                    match action {
                        FavoriteAction::Select(sound) => {
                            self.marked_view = Sidebar::Favorites;
                            self.select_favorite(sound);
                        }
                        FavoriteAction::Drag(sound) => {
                            let reference = sound.reference.clone();
                            self.select_favorite(sound);
                            self.sound_drag(&ctx, reference, Sidebar::Favorites);
                        }
                        FavoriteAction::Export(selection) => {
                            self.marked_view = Sidebar::Favorites;
                            self.export
                                .open(selection.map(crate::export::worker::Input::Selection), &ctx);
                        }
                    }
                }
            } else {
                self.list(ui);
            }
        });
        self.handle_tag_action();
        self.saved.tags.management_ui(&ctx);
        self.export.ui(&ctx);
        self.saved.tags.request();
        self.handoff_file_drag(&ctx);
        let drag_label = egui::DragAndDrop::payload::<SoundDrag>(&ctx)
            .map(|drag| {
                if drag.count == 1 {
                    drag.primary.name.clone()
                } else {
                    format!("{} sounds", drag.count)
                }
            })
            .or_else(|| {
                egui::DragAndDrop::payload::<Reference>(&ctx)
                    .map(|reference| reference.name.clone())
            });
        if let Some(label) = drag_label
            && let Some(position) = ctx.pointer_interact_pos()
        {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("sound-drag-label"),
            ));
            painter.text(
                position + egui::vec2(16.0, 18.0),
                egui::Align2::LEFT_TOP,
                &label,
                egui::FontId::proportional(13.0),
                Color32::from_rgb(135, 222, 194),
            );
        }
        if !self.saved.tags.management_open()
            && !self.export.is_open()
            && !egui::Popup::is_any_open(&ctx)
            && (!ctx.egui_wants_keyboard_input()
                || self.library_view.selection.has_focus(&ctx)
                || self.saved.sound_has_focus(&ctx))
            && (self.sidebar != Sidebar::Libraries || !self.libraries.tree_has_focus(&ctx))
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        {
            self.preview.toggle_preview();
        }
        self.smoke_check(&ctx);
    }
}

fn external_drag_requested(input: &egui::InputState) -> bool {
    input.pointer.button_down(egui::PointerButton::Primary)
        && (input
            .pointer
            .latest_pos()
            .is_some_and(|position| !input.content_rect().contains(position))
            || input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::PointerGone)))
}

fn cancel_file_drag_requested(input: &mut egui::InputState) -> bool {
    let mut cancelled = input.pointer.button_released(egui::PointerButton::Primary);
    input.events.retain(|event| {
        if matches!(
            event,
            egui::Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                ..
            }
        ) {
            cancelled = true;
            false
        } else {
            true
        }
    });
    cancelled
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop.rs"
    ));
}
