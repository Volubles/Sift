use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::audio::player::Player;
use crate::catalog::{Sample, SampleId, SearchPage};
use crate::collections::favorites::{Favorite, FavoriteId};
use crate::collections::{
    Availability, Entry, EntryId, Reference, worker::Worker as CollectionWorker,
};
use crate::library::ScanReport;
use crate::platform::{DragError, DragWorker};
use crate::search::{PAGE_SIZE, SearchQuery};
use crate::waveform::Waveforms;
use crate::worker::{Event, Workers};

mod collections;
use collections::{Action as CollectionAction, Collections};
mod favorites;
use favorites::{Action as FavoriteAction, star_button};
mod libraries;
mod preview;
use preview::Preview;
mod library_view;
mod navigation;
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
            let drags = DragWorker::start(move || drag_context.request_repaint())?;
            let collection_context = context.clone();
            let collection_worker = CollectionWorker::start_with_catalog(
                crate::collections::data_path(&options.database),
                Some(options.database.clone()),
                move || collection_context.request_repaint(),
            )?;
            Ok(Box::new(Browser::new(
                workers,
                waveforms,
                player,
                drags,
                Collections::new(collection_worker),
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
    collections: Collections,
    favorite_paths_dirty: bool,
    libraries: Libraries,
    sidebar: Sidebar,
    marked_view: Sidebar,
    drag_error: Option<String>,
    generation: u64,
    view_generation: u64,
    pages: VecDeque<SearchPage>,
    pending: BTreeSet<usize>,
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
    smoke: Option<PathBuf>,
    smoke_subfolder: Option<PathBuf>,
    screenshot_requested: bool,
    capture_writer: Option<JoinHandle<std::io::Result<()>>>,
    capture_passed: Arc<AtomicBool>,
    started: Instant,
    rendered_frames: u64,
    input_events: u64,
}

impl Browser {
    fn new(
        workers: Workers,
        waveforms: Waveforms,
        player: Player,
        drags: DragWorker,
        collections: Collections,
        options: Options,
        capture_passed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            workers,
            preview: Preview::new(waveforms, player),
            library_view: LibraryView::default(),
            drags,
            collections,
            libraries: Libraries::default(),
            marked_view: Sidebar::Libraries,
            sidebar: if options.smoke_screenshot.is_some() {
                match std::env::var("SIFT_SMOKE_VIEW").as_deref() {
                    Ok("favorites") => Sidebar::Favorites,
                    Ok("collections") => Sidebar::Collections,
                    _ => Sidebar::default(),
                }
            } else {
                Sidebar::default()
            },
            favorite_paths_dirty: true,
            drag_error: None,
            generation: 0,
            view_generation: 0,
            pages: VecDeque::new(),
            pending: BTreeSet::new(),
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
            smoke_subfolder: options
                .smoke_screenshot
                .as_ref()
                .and_then(|_| std::env::var_os("SIFT_SMOKE_SUBFOLDER").map(PathBuf::from)),
            smoke: options.smoke_screenshot,
            screenshot_requested: false,
            capture_writer: None,
            capture_passed,
            started: Instant::now(),
            rendered_frames: 0,
            input_events: 0,
        }
    }

    fn reset_query(&mut self) {
        self.library_view.selection.reset();
        self.view_generation += 1;
        self.library_view.total = 0;
        self.refresh_query();
    }

    fn marked_count(&self) -> usize {
        match self.marked_view {
            Sidebar::Libraries => self.library_view.selection.count(self.library_view.total),
            Sidebar::Collections => self.collections.marked_count(),
            Sidebar::Favorites => self.collections.favorites.selected_count(),
        }
    }

    fn change_library_location(&mut self) {
        if self.library_view.change(self.libraries.location()) {
            self.view_generation += 1;
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
        if self.sidebar == Sidebar::Collections {
            self.collections.suspend_view();
        }
        if mode == Sidebar::Favorites {
            self.library_view.selection.suspend();
            self.workers
                .invalidate_selection(self.library_view.selection.generation());
        }
        self.sidebar = mode;
        self.collections
            .favorites
            .set_active(mode == Sidebar::Favorites);
        match mode {
            Sidebar::Libraries => {
                self.marked_view = Sidebar::Libraries;
                self.restore_library_preview();
                self.refresh_query();
            }
            Sidebar::Favorites => {
                self.marked_view = Sidebar::Favorites;
                self.restore_preview(self.collections.favorites.active_reference());
            }
            Sidebar::Collections => {
                let saved = self.collections.resume_view();
                self.restore_preview(saved);
            }
        }
    }

    fn marked_selection(&self) -> Result<crate::collections::ReferenceSelection, String> {
        match self.marked_view {
            Sidebar::Libraries => Ok(crate::collections::ReferenceSelection {
                origin: crate::collections::SelectionOrigin::Library {
                    query: self.library_view.query.clone(),
                    scope: self.libraries.scope(),
                },
                sources: self
                    .library_view
                    .selection
                    .snapshot(self.library_view.total)?,
            }),
            Sidebar::Collections => self.collections.selected_references(),
            Sidebar::Favorites => self.collections.favorites.selected_references(),
        }
    }

    fn sound_drag(&mut self, context: &egui::Context, primary: Reference, view: Sidebar) {
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
        self.library_view.selection.refresh();
        self.workers
            .invalidate_selection(self.library_view.selection.generation());
        self.favorite_paths_dirty = true;
        self.generation += 1;
        self.pages.clear();
        self.pending.clear();
        match SearchQuery::parse(&self.library_view.query_text) {
            Ok(query) => {
                self.library_view.query = query;
                self.error = None;
                self.request_page(
                    self.library_view.selection.first_visible() / PAGE_SIZE * PAGE_SIZE,
                );
            }
            Err(error) => {
                self.error = Some(error.to_string());
            }
        }
    }

    fn request_page(&mut self, offset: usize) {
        if self.ready
            && self.error.is_none()
            && !self.pending.contains(&offset)
            && !self.pages.iter().any(|page| page.offset == offset)
            && self.workers.query_scoped(
                self.generation,
                self.library_view.query.clone(),
                self.libraries.scope(),
                offset,
            )
        {
            self.pending.insert(offset);
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
        self.collections.poll();
        if self.sidebar == Sidebar::Collections
            && let Some(saved) = self.collections.take_preview_restore()
        {
            self.restore_preview(saved);
        }
        if let Some(SoundKey::Collection(id)) = self.preview.key()
            && let Some(entry) = self.collections.selected_update(id)
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
                    let scoped = if let Some(relative) = self.smoke_subfolder.take() {
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
                Event::Page { generation, result } if generation == self.generation => match result
                {
                    Ok(page) => {
                        self.library_view.total = page.total;
                        self.pending.remove(&page.offset);
                        if self.sidebar != Sidebar::Favorites {
                            self.preview.refresh_catalog(&page.rows);
                        }
                        for (position, sample) in page.rows.iter().enumerate() {
                            self.library_view.selection.observe(
                                SoundKey::Library(sample.id),
                                page.offset + position,
                                &sample.source,
                            );
                            if self.library_view.selection.active_source()
                                == Some(sample.source.as_path())
                            {
                                self.library_view
                                    .selection
                                    .remember(Reference::from(sample));
                            }
                        }
                        insert_page(&mut self.pages, page);
                        self.favorite_paths_dirty = true;
                    }
                    Err(message) => {
                        self.pending.clear();
                        eprintln!("Sift search failed: {message}");
                        self.error = Some(format!("Search failed: {message}"));
                    }
                },
                Event::Page { .. } => {}
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
                    self.collections.sources_changed(
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
        if self.favorite_paths_dirty || self.collections.favorite_paths_dirty {
            let mut paths: BTreeSet<_> = self
                .pages
                .iter()
                .flat_map(|page| &page.rows)
                .map(|sample| sample.source.clone())
                .collect();
            if let Some(selected) = self.preview.selected() {
                paths.insert(selected.source.clone());
            }
            self.collections.track_favorites(paths);
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
        self.drag_error = match self.drags.request(vec![sample.source]) {
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
        let query_text = if favorites {
            &mut self.collections.favorites.query_text
        } else {
            &mut self.library_view.query_text
        };
        let response = ui.add_sized(
            [ui.available_width(), 34.0],
            egui::TextEdit::singleline(query_text).hint_text("Search sounds by name or folder..."),
        );
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            response.request_focus();
        }
        if response.changed() {
            if favorites {
                self.collections.favorites.reset_query();
            } else {
                self.reset_query();
            }
        }
        ui.horizontal(|ui| {
            let total = if favorites {
                self.collections.favorites.total
            } else {
                self.library_view.total
            };
            let count_label = if favorites {
                "favorites"
            } else if self.library_view.query_text.is_empty() {
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
        self.collections.favorite_feedback(ui);
        if self.drags.busy() {
            ui.horizontal(|ui| {
                ui.label(if self.drags.native_active() {
                    "Dragging a copy. Release in a folder, or press Esc to cancel."
                } else {
                    "Preparing file copy. Keep the left mouse button held; Esc cancels."
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
        if !self.ready {
            ui.label(if self.error.is_some() {
                "Catalog unavailable."
            } else {
                "Opening catalog..."
            });
            return;
        }
        if self.library_view.total == 0 && self.pages.is_empty() {
            self.request_page(0);
        }
        if self.library_view.total == 0 {
            if !self.pending.is_empty() {
                ui.label("Searching...");
            } else if self.library_view.query_text.is_empty() {
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
        let favorite_key =
            self.library_view
                .selection
                .keyboard(ui, focus_id, self.library_view.total);
        let target = self.library_view.selection.target();
        let mut choice = target.and_then(|(index, gesture)| {
            self.pages
                .iter()
                .find(|page| index >= page.offset && index < page.offset + page.rows.len())
                .and_then(|page| page.rows.get(index - page.offset))
                .map(|sample| (sample.clone(), index, gesture))
        });
        let mut drag_sample = None;
        let mut favorite = None;
        self.library_view
            .selection
            .feedback(ui, self.library_view.total);
        table_heading(ui);
        let scroll = self
            .library_view
            .selection
            .scroll_area(
                ui,
                egui::ScrollArea::vertical()
                    .id_salt(("samples", self.view_generation))
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
                        let response = ui
                            .horizontal(|ui| {
                                if star_button(
                                    ui,
                                    self.collections.favorites.status(&sample.source),
                                    self.collections.favorites.can_edit(),
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
                            .inner;
                        selection::active_outline(
                            ui,
                            &response,
                            self.library_view.selection.active_source()
                                == Some(sample.source.as_path()),
                        );
                        if response.clicked() {
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
        if let Some(reference) = favorite {
            self.collections.toggle_favorite(&reference);
        }
        if let Some((sample, index, gesture)) = choice {
            self.marked_view = Sidebar::Libraries;
            self.library_view.selection.choose(
                SoundKey::Library(sample.id),
                index,
                &sample.source,
                gesture,
            );
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
            self.collections.toggle_favorite(reference);
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
            && self.workers.selection(
                request.generation,
                self.library_view.query.clone(),
                self.libraries.scope(),
                request.anchor,
                request.target,
            )
        {
            self.library_view.selection.sent();
        }
    }

    fn smoke_check(&mut self, ctx: &egui::Context) {
        let Some(path) = self.smoke.clone() else {
            return;
        };
        if self.ready
            && self.collections.settled()
            && self.collections.favorites.settled()
            && (self.sidebar != Sidebar::Libraries || self.libraries.settled())
            && self.scan.is_none()
            && !self.preview.waveform_pending()
            && !self.pages.is_empty()
            && !self.screenshot_requested
        {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            match std::thread::Builder::new()
                .name("sift-smoke-capture".into())
                .spawn(move || save_capture(path, &image))
            {
                Ok(writer) => {
                    self.capture_writer = Some(writer);
                }
                Err(error) => {
                    eprintln!("Sift capture worker failed: {error}");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if self
            .capture_writer
            .as_ref()
            .is_some_and(|writer| writer.is_finished())
            && let Some(writer) = self.capture_writer.take()
        {
            match writer.join() {
                Ok(Ok(())) => {
                    self.capture_passed.store(true, Ordering::Release);
                    eprintln!(
                        "Sift native window smoke capture complete; {} results, {} cached records",
                        self.library_view.total,
                        self.pages.iter().map(|page| page.rows.len()).sum::<usize>()
                    );
                }
                Ok(Err(error)) => eprintln!("Sift smoke capture failed: {error}"),
                Err(_) => eprintln!("Sift smoke capture worker panicked"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if self.started.elapsed() > Duration::from_secs(30) {
            eprintln!("Sift smoke capture timed out: {:?}", self.error);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
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

fn save_capture(path: PathBuf, image: &egui::ColorImage) -> std::io::Result<()> {
    use std::io::Write;
    let mut bytes = format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
    bytes.reserve(image.width() * image.height() * 3);
    for pixel in &image.pixels {
        let rgba = pixel.to_array();
        bytes.extend_from_slice(&rgba[..3]);
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&bytes)
}

impl Drop for Browser {
    fn drop(&mut self) {
        if let Some(writer) = self.capture_writer.take() {
            match writer.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("Sift capture failed during shutdown: {error}"),
                Err(_) => eprintln!("Sift capture worker panicked during shutdown"),
            }
        }
    }
}

fn insert_page(pages: &mut VecDeque<SearchPage>, page: SearchPage) {
    pages.retain(|cached| cached.offset != page.offset);
    pages.push_back(page);
    // Keep recent requested pages, not the numerically largest offsets. Scrolling
    // back to an earlier page must not evict that page immediately.
    while pages.len() > 4 {
        pages.pop_front();
    }
}

impl eframe::App for Browser {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(path) = std::env::var_os("SIFT_FRAME_REPORT") {
            let report = format!(
                "frames={} input_events={} phase={}\n",
                self.rendered_frames,
                self.input_events,
                self.preview.playback_phase()
            );
            use std::io::Write;
            let result = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .and_then(|mut file| file.write_all(report.as_bytes()));
            if let Err(error) = result {
                eprintln!("Frame diagnostic failed: {error}");
            }
        }
    }
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.accept_events(context);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.rendered_frames += 1;
        self.collections
            .favorites
            .set_active(self.sidebar == Sidebar::Favorites);
        self.input_events += ui.input(|input| input.events.len()) as u64;
        let ctx = ui.ctx().clone();
        // Keep the most recently used sound list when a sidebar/button takes
        // focus. Opening Collections must not substitute its first cached row.
        if self.library_view.selection.has_focus(&ctx) {
            self.marked_view = Sidebar::Libraries;
        } else if self.collections.favorites.has_focus(&ctx) {
            self.marked_view = Sidebar::Favorites;
        } else if self.collections.sound_has_focus(&ctx) {
            self.marked_view = Sidebar::Collections;
        }
        if self.drags.busy()
            && (ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                || ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary)))
        {
            self.drags.cancel();
        }
        egui::Panel::bottom("footer").show(ui, |ui| {
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
                    ui.label("Select sounds, then switch to Collections to save them together. Drag one sound out to copy it.");
                    if ui.button("Refresh favorites").clicked() { self.collections.favorites.reset_query(); }
                    return;
                }
                let selected_entry = match self.preview.key() {
                    Some(SoundKey::Collection(id)) => Some(id),
                    _ => None,
                };
                let action = self.collections.ui(ui, self.preview.selected(), selected_entry, self.marked_count());
                if let Some(saved) = self.collections.take_preview_restore() { self.restore_preview(saved); }
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
                            self.collections.add_selected(collection, selection);
                        }
                    }
                }
            });
        egui::Panel::top("header").show(ui, |ui| self.header(ui));
        egui::Panel::top("preview").show(ui, |ui| self.preview.ui(ui));
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
                    self.collections
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
                    }
                }
            } else {
                self.list(ui);
            }
        });
        if !self.drags.busy()
            && DragWorker::supported()
            && ctx.input(external_drag_requested)
            && egui::DragAndDrop::has_payload_of_type::<SoundDrag>(&ctx)
            && let Some(drag) = egui::DragAndDrop::take_payload::<SoundDrag>(&ctx)
        {
            if drag.count == 1 {
                self.begin_drag(drag.primary.clone());
            } else {
                self.drag_error = Some(
                    "Drag one sound to copy outside Sift. Groups can be dropped into collections."
                        .into(),
                );
            }
        }
        if !self.drags.busy()
            && DragWorker::supported()
            && ctx.input(external_drag_requested)
            && egui::DragAndDrop::has_payload_of_type::<Reference>(&ctx)
            && let Some(reference) = egui::DragAndDrop::take_payload::<Reference>(&ctx)
        {
            self.begin_drag((*reference).clone());
        }
        let drag_label = egui::DragAndDrop::payload::<SoundDrag>(&ctx)
            .map(|drag| {
                if drag.count == 1 {
                    drag.primary.name.clone()
                } else {
                    format!("{} sounds · collections", drag.count)
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
        if !egui::Popup::is_any_open(&ctx)
            && (!ctx.egui_wants_keyboard_input()
                || self.library_view.selection.has_focus(&ctx)
                || self.collections.sound_has_focus(&ctx))
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

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop.rs"
    ));
}
