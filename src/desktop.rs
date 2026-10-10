use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::audio::{player::Player, transport};
use crate::catalog::{Sample, SampleId, SearchPage};
use crate::collections::{
    Availability, Entry, EntryId, Reference, worker::Worker as CollectionWorker,
};
use crate::library::ScanReport;
use crate::platform::{DragError, DragWorker};
use crate::search::{PAGE_SIZE, SearchQuery};
use crate::waveform::{Waveform, Waveforms};
use crate::worker::{Event, Workers};

mod collections;
use collections::{Action as CollectionAction, Collections};
mod libraries;
use libraries::{Action as LibraryAction, Libraries};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Sidebar {
    #[default]
    Libraries,
    Collections,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SoundKey {
    Library(SampleId),
    Collection(EntryId),
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
            let collection_worker = CollectionWorker::start(
                crate::collections::data_path(&options.database),
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
    waveforms: Waveforms,
    player: Player,
    drags: DragWorker,
    collections: Collections,
    libraries: Libraries,
    sidebar: Sidebar,
    drag_error: Option<String>,
    waveform: Option<Arc<Waveform>>,
    wave_generation: u64,
    wave_pending: bool,
    wave_error: Option<String>,
    audio_error: Option<String>,
    output_name: Option<String>,
    volume: f32,
    query_text: String,
    query: SearchQuery,
    generation: u64,
    pages: VecDeque<SearchPage>,
    pending: BTreeSet<usize>,
    total: usize,
    ready: bool,
    error: Option<String>,
    scan: Option<ScanReport>,
    scan_summary: Option<ScanReport>,
    selected: Option<Reference>,
    selected_key: Option<SoundKey>,
    selected_availability: Availability,
    show_details: bool,
    auto_play: bool,
    folder: Option<PathBuf>,
    smoke: Option<PathBuf>,
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
            waveforms,
            player,
            drags,
            collections,
            libraries: Libraries::default(),
            sidebar: Sidebar::default(),
            drag_error: None,
            waveform: None,
            wave_generation: 0,
            wave_pending: false,
            wave_error: None,
            audio_error: None,
            output_name: None,
            volume: 0.4,
            query_text: String::new(),
            query: SearchQuery::All,
            generation: 0,
            pages: VecDeque::new(),
            pending: BTreeSet::new(),
            total: 0,
            ready: false,
            error: None,
            scan: None,
            scan_summary: None,
            selected: None,
            selected_key: None,
            selected_availability: Availability::Ready,
            show_details: false,
            auto_play: false,
            folder: options.folder,
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
        self.generation += 1;
        self.pages.clear();
        self.pending.clear();
        self.total = 0;
        match SearchQuery::parse(&self.query_text) {
            Ok(query) => {
                self.query = query;
                self.error = None;
                self.request_page(0);
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
                self.query.clone(),
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
        if let Some(SoundKey::Collection(id)) = self.selected_key
            && let Some(entry) = self.collections.selected_update(id)
            && self.selected_availability != entry.availability
        {
            self.selected_availability = entry.availability.clone();
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
                Event::Ready => {
                    self.ready = true;
                    self.request_page(0);
                    if let Some(folder) = self.folder.take() {
                        self.libraries.add_initial_folder(&self.workers, folder);
                    }
                }
                Event::Libraries(result) => {
                    if self.libraries.snapshot(result) {
                        self.reset_query();
                    }
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
                        self.total = page.total;
                        self.pending.remove(&page.offset);
                        if let Some(SoundKey::Library(id)) = self.selected_key {
                            if let Some(updated) = page.rows.iter().find(|row| row.id == id) {
                                self.selected = Some(Reference::from(updated));
                            }
                        } else if self.selected.is_none()
                            && let Some(sample) = page.rows.first().cloned()
                        {
                            self.select_sample(sample);
                        }
                        insert_page(&mut self.pages, page);
                    }
                    Err(message) => {
                        self.pending.clear();
                        eprintln!("Sift search failed: {message}");
                        self.error = Some(format!("Search failed: {message}"));
                    }
                },
                Event::Page { .. } => {}
                Event::Progress(report) => {
                    self.scan = Some(report);
                }
                Event::ScanFinished(result) => {
                    self.workers.finish_scan();
                    self.scan = None;
                    match result {
                        Ok(report) => {
                            self.scan_summary = Some(report);
                            self.reset_query();
                        }
                        Err(message) => {
                            eprintln!("Sift indexing failed: {message}");
                            self.error = Some(format!("Indexing failed: {message}"));
                        }
                    }
                }
                Event::StartupFailed(message) => {
                    eprintln!("Sift catalog startup failed: {message}");
                    self.error = Some(format!("Catalog could not open: {message}"));
                }
            }
        }
        while let Some(ready) = self.waveforms.poll() {
            if ready.generation != self.wave_generation {
                continue;
            }
            self.wave_pending = false;
            match ready.result {
                Ok(waveform) => {
                    self.waveform = Some(waveform);
                    self.wave_error = None;
                }
                Err(error) => {
                    eprintln!("Waveform analysis failed: {error}");
                    self.wave_error = Some(error.to_string());
                }
            }
        }
        while let Some(message) = self.player.poll() {
            if message.generation != self.player.generation() {
                continue;
            }
            if let Some(error) = message.error {
                eprintln!("Preview failed: {error}");
                self.audio_error = Some(error);
            }
            if let Some(output) = message.output {
                self.output_name = Some(output);
                self.audio_error = None;
            }
        }
    }

    fn select_sample(&mut self, sample: Sample) {
        self.select(
            Reference::from(&sample),
            SoundKey::Library(sample.id),
            Availability::Ready,
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
        let autoplay = self.auto_play;
        self.wave_generation += 1;
        self.waveform = None;
        self.wave_error = match &availability {
            Availability::Missing => {
                Some("Source file is missing; the saved reference is retained".into())
            }
            Availability::Unavailable(error) => Some(format!("Source unavailable: {error}")),
            _ => None,
        };
        self.audio_error = None;
        self.wave_pending = sample.metadata_error.is_none() && availability.usable();
        if self.wave_pending
            && let Err(error) = self
                .waveforms
                .request(self.wave_generation, sample.source.clone())
        {
            self.wave_error = Some(error.to_string());
            self.wave_pending = false;
        }
        let result = if autoplay && sample.metadata_error.is_none() && availability.usable() {
            self.player.play(sample.source.clone(), 0.0)
        } else {
            self.player.stop()
        };
        if let Err(error) = result {
            self.audio_error = Some(error.to_string());
        }
        self.selected = Some(sample);
        self.selected_key = Some(key);
        self.selected_availability = availability;
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

    fn toggle_preview(&mut self) {
        let Some(sample) = &self.selected else {
            return;
        };
        if sample.metadata_error.is_some() || !self.selected_availability.usable() {
            return;
        }
        let result = match self.player.snapshot().phase {
            transport::PLAYING | transport::LOADING => self.player.pause(),
            transport::PAUSED => self.player.resume(),
            _ => self.player.play(sample.source.clone(), 0.0),
        };
        if let Err(error) = result {
            self.audio_error = Some(error.to_string());
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.player.snapshot();
        let available = self
            .selected
            .as_ref()
            .is_some_and(|sample| sample.metadata_error.is_none())
            && self.selected_availability.usable();
        ui.horizontal(|ui| {
            let label = if matches!(snapshot.phase, transport::PLAYING | transport::LOADING) {
                "Pause"
            } else {
                "Play"
            };
            ui.checkbox(&mut self.auto_play, "Auto-play");
            if ui
                .add_enabled(available, egui::Button::new(label))
                .clicked()
            {
                self.toggle_preview();
            }
            if ui.button("Stop").clicked()
                && let Err(error) = self.player.stop()
            {
                self.audio_error = Some(error.to_string());
            }
            if ui
                .add(egui::Slider::new(&mut self.volume, 0.0..=1.0).text("Volume"))
                .changed()
                && let Err(error) = self.player.volume(self.volume)
            {
                self.audio_error = Some(error.to_string());
            }
            let duration = self
                .waveform
                .as_ref()
                .map(|waveform| waveform.duration())
                .or_else(|| {
                    self.selected
                        .as_ref()
                        .and_then(|sample| sample.audio.duration_seconds)
                });
            ui.label(format!(
                "{:.2} / {} s",
                snapshot.seconds,
                duration
                    .map(|value| format!("{value:.2}"))
                    .unwrap_or_else(|| "Unknown".into())
            ));
            ui.label(RichText::new("Space: play/pause").small().weak());
        });
        let (response, painter) = ui.allocate_painter(
            egui::vec2(ui.available_width(), 105.0),
            egui::Sense::click(),
        );
        let rect = response.rect;
        painter.rect_filled(rect, 3.0, Color32::from_rgb(15, 19, 20));
        let middle = rect.center().y;
        painter.line_segment(
            [
                egui::pos2(rect.left(), middle),
                egui::pos2(rect.right(), middle),
            ],
            egui::Stroke::new(1.0, Color32::from_gray(45)),
        );
        if let Some(waveform) = &self.waveform {
            for (index, peak) in waveform.peaks.iter().enumerate() {
                let x =
                    rect.left() + (index as f32 + 0.5) / waveform.peaks.len() as f32 * rect.width();
                painter.line_segment(
                    [
                        egui::pos2(x, middle - peak[1].clamp(-1.0, 1.0) * rect.height() * 0.45),
                        egui::pos2(x, middle - peak[0].clamp(-1.0, 1.0) * rect.height() * 0.45),
                    ],
                    egui::Stroke::new(1.0, Color32::from_rgb(135, 222, 194)),
                );
            }
            let duration = waveform.duration();
            let x =
                rect.left() + (snapshot.seconds / duration).clamp(0.0, 1.0) as f32 * rect.width();
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(1.5, Color32::WHITE),
            );
            if response.clicked()
                && let Some(position) = response.interact_pointer_pos()
                && let Some(sample) = &self.selected
            {
                let seconds =
                    ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * duration;
                if let Err(error) = self.player.play(sample.source.clone(), seconds) {
                    self.audio_error = Some(error.to_string());
                }
            }
        } else {
            let text = if self.wave_pending {
                "Loading waveform..."
            } else {
                "Select a sound to see its waveform"
            };
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(13.0),
                Color32::GRAY,
            );
        }
        if let Some(error) = &self.wave_error {
            ui.colored_label(
                Color32::from_rgb(245, 143, 140),
                format!("Waveform unavailable: {error}"),
            );
        }
        if let Some(error) = &self.audio_error {
            ui.colored_label(
                Color32::from_rgb(245, 143, 140),
                format!("Preview unavailable: {error}"),
            );
            if ui.button("Reconnect audio output").clicked()
                && let Some(sample) = &self.selected
                && let Err(error) = self
                    .player
                    .reconnect(sample.source.clone(), snapshot.seconds)
            {
                self.audio_error = Some(error.to_string());
            }
        }
        if let Some(output) = &self.output_name {
            ui.label(RichText::new(format!("Output: {output}")).small().weak());
        }
        if matches!(snapshot.phase, transport::PLAYING | transport::LOADING) {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
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
                if self.scan.is_some() && ui.button("Cancel scan").clicked() {
                    self.workers.cancel_scan();
                }
            });
        });
        ui.label(
            RichText::new(format!("Browsing: {}", self.libraries.label()))
                .small()
                .color(Color32::from_rgb(135, 222, 194)),
        );
        ui.add_space(12.0);
        let response = ui.add_sized(
            [ui.available_width(), 34.0],
            egui::TextEdit::singleline(&mut self.query_text)
                .hint_text("Search sounds by name or folder..."),
        );
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            response.request_focus();
        }
        if response.changed() {
            self.reset_query();
        }
        ui.horizontal(|ui| {
            let count_label = if self.query_text.is_empty() {
                "indexed sounds"
            } else {
                "matching sounds"
            };
            ui.label(
                RichText::new(format!("{} {count_label}", self.total))
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
                "Indexing: {} files saved, {} issues",
                scan.indexed, scan.errors
            ));
        }
        if let Some(report) = &self.scan_summary {
            let outcome = if report.cancelled {
                "Scan cancelled"
            } else {
                "Scan finished"
            };
            ui.label(format!(
                "{outcome}: {} files saved, {} issues, {} links skipped",
                report.indexed, report.errors, report.skipped_links
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

    fn details(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sound details");
        ui.add_space(12.0);
        if let Some(sample) = &self.selected {
            ui.label(RichText::new(&sample.name).strong().size(17.0));
            ui.add_space(10.0);
            egui::Grid::new("metadata")
                .spacing([16.0, 9.0])
                .show(ui, |ui| {
                    for (label, value) in [
                        ("Format", sample.format.to_uppercase()),
                        (
                            "Duration",
                            sample
                                .audio
                                .duration_seconds
                                .map(|value| format!("{value:.3} s"))
                                .unwrap_or_else(|| "Unknown".into()),
                        ),
                        (
                            "Sample rate",
                            sample
                                .audio
                                .sample_rate
                                .map(|value| format!("{value} Hz"))
                                .unwrap_or_else(|| "Unknown".into()),
                        ),
                        (
                            "Channels",
                            sample
                                .audio
                                .channels
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "Unknown".into()),
                        ),
                        (
                            "Bit depth",
                            sample
                                .audio
                                .bit_depth
                                .map(|value| format!("{value}-bit"))
                                .unwrap_or_else(|| "Unknown".into()),
                        ),
                        (
                            "Size",
                            format!("{:.1} KiB", sample.size_bytes as f64 / 1024.0),
                        ),
                    ] {
                        ui.label(RichText::new(label).weak());
                        ui.label(value);
                        ui.end_row();
                    }
                });
            ui.add_space(18.0);
            ui.label(RichText::new("SOURCE").small().weak());
            ui.label(sample.source.display().to_string());
            match &self.selected_availability {
                Availability::Changed => {
                    ui.colored_label(
                        Color32::from_rgb(230, 184, 115),
                        "Source changed. Metadata is from the saved reference.",
                    );
                }
                Availability::Missing => {
                    ui.colored_label(
                        Color32::from_rgb(245, 143, 140),
                        "Source file is missing. The saved reference is retained.",
                    );
                }
                Availability::Unavailable(error) => {
                    ui.colored_label(
                        Color32::from_rgb(245, 143, 140),
                        format!("Source unavailable: {error}"),
                    );
                }
                Availability::Ready => {}
            }
            if let Some(error) = &sample.metadata_error {
                ui.add_space(12.0);
                ui.colored_label(
                    Color32::from_rgb(245, 143, 140),
                    format!("Metadata unavailable: {error}"),
                );
            }
            ui.add_space(12.0);
            let response = ui.add_enabled(
                DragWorker::supported() && !self.drags.busy(),
                egui::Button::new("Drag copy to folder").sense(egui::Sense::click_and_drag()),
            );
            let dragged = response
                .drag_started_by(egui::PointerButton::Primary)
                .then(|| sample.clone());
            response.on_hover_text("Hold the left mouse button and drag to an Explorer folder. The source stays in its library.");
            if let Some(sample) = dragged {
                egui::DragAndDrop::set_payload(ui.ctx(), sample);
            }
        } else {
            ui.label("Choose a sound to inspect its metadata.");
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
        if self.total == 0 && self.pages.is_empty() {
            self.request_page(0);
        }
        if self.total == 0 {
            if !self.pending.is_empty() {
                ui.label("Searching...");
            } else if self.query_text.is_empty() {
                ui.label(
                    "No indexed sounds in this view. Add or rescan a source folder in Libraries.",
                );
            } else {
                ui.label("No matching sounds.");
            }
            return;
        }
        let mut needed = BTreeSet::new();
        let selected_id = match self.selected_key {
            Some(SoundKey::Library(id)) => Some(id),
            _ => None,
        };
        let mut drag_sample = None;
        table_heading(ui);
        egui::ScrollArea::vertical()
            .id_salt(("samples", self.generation))
            .auto_shrink([false, false])
            .show_rows(ui, 28.0, self.total, |ui, range| {
                for index in range {
                    let offset = index / PAGE_SIZE * PAGE_SIZE;
                    needed.insert(offset);
                    if let Some(sample) = self
                        .pages
                        .iter()
                        .find(|page| page.offset == offset)
                        .and_then(|page| page.rows.get(index - offset))
                    {
                        let response = table_row(ui, sample, selected_id == Some(sample.id), index);
                        let clicked = response.clicked().then(|| sample.clone());
                        if DragWorker::supported()
                            && !self.drags.busy()
                            && response.drag_started_by(egui::PointerButton::Primary)
                        {
                            drag_sample = Some(sample.clone());
                        }
                        response.on_hover_text(format!(
                            "{}\nDrag to a collection, or out to copy the source file.",
                            sample.display_path
                        ));
                        if let Some(sample) = clicked {
                            self.select_sample(sample);
                        }
                    } else {
                        ui.add_sized(
                            [ui.available_width(), 32.0],
                            egui::Label::new("Loading sounds..."),
                        );
                    }
                }
            });
        if let Some(sample) = drag_sample {
            let reference = Reference::from(&sample);
            self.select_sample(sample);
            egui::DragAndDrop::set_payload(ui.ctx(), reference);
        }
        for offset in needed {
            self.request_page(offset);
        }
    }

    fn smoke_check(&mut self, ctx: &egui::Context) {
        let Some(path) = self.smoke.clone() else {
            return;
        };
        if self.ready
            && self.collections.settled()
            && self.libraries.settled()
            && self.scan.is_none()
            && !self.wave_pending
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
                        self.total,
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

fn sample_button(label: String, selected: bool) -> egui::Button<'static> {
    egui::Button::new(label)
        .selected(selected)
        .sense(egui::Sense::click_and_drag())
}

fn navigation_button(ui: &mut egui::Ui, mode: Sidebar, selected: bool) -> egui::Response {
    let name = match mode {
        Sidebar::Libraries => "Libraries",
        Sidebar::Collections => "Collections",
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

fn table_heading(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
    paint_columns(
        ui,
        rect,
        &[
            "Filename".into(),
            "Format".into(),
            "Length".into(),
            "Rate".into(),
            "Ch".into(),
        ],
        Color32::GRAY,
    );
    ui.separator();
}

fn table_row(ui: &mut egui::Ui, sound: &Sample, selected: bool, index: usize) -> egui::Response {
    let response = ui.add_sized(
        [ui.available_width(), 28.0],
        sample_button(String::new(), selected),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            selected,
            &sound.name,
        )
    });
    if !selected && !response.hovered() && index.is_multiple_of(2) {
        ui.painter()
            .rect_filled(response.rect, 0.0, Color32::from_rgb(37, 39, 40));
    }
    let columns = [
        sound.name.clone(),
        sound.format.to_uppercase(),
        sound
            .audio
            .duration_seconds
            .map(|seconds| format!("{seconds:.2} s"))
            .unwrap_or_else(|| "—".into()),
        sound
            .audio
            .sample_rate
            .map(|rate| rate.to_string())
            .unwrap_or_else(|| "—".into()),
        sound
            .audio
            .channels
            .map(|channels| channels.to_string())
            .unwrap_or_else(|| "—".into()),
    ];
    paint_columns(
        ui,
        response.rect,
        &columns,
        if selected {
            Color32::WHITE
        } else {
            Color32::from_gray(210)
        },
    );
    response
}

fn paint_columns(ui: &egui::Ui, rect: egui::Rect, values: &[String; 5], color: Color32) {
    let widths = [(rect.width() - 280.0).max(80.0), 60.0, 80.0, 90.0, 50.0];
    let mut x = rect.left();
    for (value, width) in values.iter().zip(widths) {
        let cell =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(width, rect.height()))
                .intersect(rect);
        ui.painter()
            .with_clip_rect(cell.shrink2(egui::vec2(6.0, 0.0)))
            .text(
                egui::pos2(x + 8.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                value,
                egui::FontId::proportional(13.0),
                color,
            );
        x += width;
    }
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
                self.player.snapshot().phase
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
        self.input_events += ui.input(|input| input.events.len()) as u64;
        let ctx = ui.ctx().clone();
        if self.drags.busy()
            && (ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                || ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary)))
        {
            self.drags.cancel();
        }
        egui::Panel::bottom("footer").show(ui, |ui| {
            ui.label(
                RichText::new(
                    "Source files stay in place. Collections save references automatically.",
                )
                .small()
                .weak(),
            );
        });
        egui::Panel::left("activity-rail")
            .resizable(false)
            .exact_size(44.0)
            .show(ui, |ui| {
                for mode in [Sidebar::Libraries, Sidebar::Collections] {
                    if navigation_button(ui, mode, self.sidebar == mode).clicked() {
                        self.sidebar = mode;
                    }
                }
            });
        egui::Panel::left("browser-sidebar")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                if self.sidebar == Sidebar::Libraries {
                    if let Some(action) = self.libraries.ui(ui, &self.workers, self.scan.is_some())
                    {
                        match action {
                            LibraryAction::ScopeChanged => self.reset_query(),
                            LibraryAction::Scan(folder) => self.begin_scan(folder),
                        }
                    }
                    return;
                }
                let selected_entry = match self.selected_key {
                    Some(SoundKey::Collection(id)) => Some(id),
                    _ => None,
                };
                if let Some(action) =
                    self.collections
                        .ui(ui, self.selected.as_ref(), selected_entry)
                {
                    match action {
                        CollectionAction::Select(entry) => self.select_entry(entry),
                        CollectionAction::Drag(entry) => {
                            let reference = entry.reference.clone();
                            self.select_entry(entry);
                            egui::DragAndDrop::set_payload(&ctx, reference);
                        }
                    }
                }
            });
        egui::Panel::top("header").show(ui, |ui| self.header(ui));
        egui::Panel::top("preview").show(ui, |ui| self.preview(ui));
        if self.show_details {
            egui::Panel::right("details")
                .resizable(true)
                .default_size(300.0)
                .show(ui, |ui| self.details(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| self.list(ui));
        if !self.drags.busy()
            && DragWorker::supported()
            && ctx.input(external_drag_requested)
            && let Some(reference) = egui::DragAndDrop::take_payload::<Reference>(&ctx)
        {
            self.begin_drag((*reference).clone());
        }
        if let Some(reference) = egui::DragAndDrop::payload::<Reference>(&ctx)
            && let Some(position) = ctx.pointer_interact_pos()
        {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("sound-drag-label"),
            ));
            painter.text(
                position + egui::vec2(16.0, 18.0),
                egui::Align2::LEFT_TOP,
                &reference.name,
                egui::FontId::proportional(13.0),
                Color32::from_rgb(135, 222, 194),
            );
        }
        if !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        {
            self.toggle_preview();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_sound_drop_reaches_the_collection_before_payload_cleanup() {
        let ctx = egui::Context::default();
        let reference = Reference {
            source: PathBuf::from("fixture.wav"),
            name: "fixture.wav".into(),
            display_path: "fixture.wav".into(),
            format: "wav".into(),
            size_bytes: 7,
            audio: Default::default(),
            metadata_error: None,
        };
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 0.02;
            let mut geometry = None;
            let mut received = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let source =
                            ui.add_sized([400.0, 28.0], sample_button("Sound".into(), false));
                        if source.drag_started_by(egui::PointerButton::Primary) {
                            source.dnd_set_drag_payload(reference.clone());
                        }
                        let target = ui.add_sized([400.0, 28.0], egui::Button::new("Collection"));
                        received = target.dnd_release_payload::<Reference>();
                        geometry = Some((source.rect.center(), target.rect.center()));
                    });
                },
            );
            output.textures_delta.clear();
            (geometry.unwrap(), received)
        };
        let ((source, target), _) = frame(vec![]);
        let button = |position, pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![
            egui::Event::PointerMoved(source),
            button(source, true),
        ]);
        frame(vec![egui::Event::PointerMoved(
            source + egui::vec2(30.0, 0.0),
        )]);
        assert!(egui::DragAndDrop::payload::<Reference>(&ctx).is_some());
        assert!(!ctx.input(external_drag_requested));
        frame(vec![egui::Event::PointerMoved(target)]);
        let (_, received) = frame(vec![button(target, false)]);
        assert_eq!(received.unwrap().source, reference.source);
        assert!(egui::DragAndDrop::payload::<Reference>(&ctx).is_none());
        frame(vec![
            egui::Event::PointerMoved(source),
            button(source, true),
        ]);
        frame(vec![egui::Event::PointerMoved(egui::pos2(810.0, 100.0))]);
        assert!(ctx.input(external_drag_requested));
    }

    #[test]
    fn sound_row_separates_click_selection_from_a_primary_drag_and_can_be_reused() {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 0.02;
            let mut response = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let row =
                            ui.add_sized([400.0, 32.0], sample_button("Fixture.wav".into(), false));
                        response = Some(row);
                    });
                },
            );
            // This is an interaction test without a renderer.
            output.textures_delta.clear();
            response.unwrap()
        };
        let first = frame(vec![]);
        let origin = first.rect.center();
        let button = |pos, pressed, button| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![
            egui::Event::PointerMoved(origin),
            button(origin, true, egui::PointerButton::Primary),
        ]);
        let click = frame(vec![button(origin, false, egui::PointerButton::Primary)]);
        assert!(click.clicked());
        assert!(!click.drag_started());
        frame(vec![button(origin, true, egui::PointerButton::Primary)]);
        let moved = origin + egui::vec2(30.0, 0.0);
        let drag = frame(vec![egui::Event::PointerMoved(moved)]);
        assert!(drag.drag_started_by(egui::PointerButton::Primary));
        assert!(!drag.clicked());
        // OLE owns the final release; no native pointer-release event arrives in
        // this simulation. The same reset used by the desktop clears that state.
        ctx.stop_dragging();
        ctx.input_mut(|input| input.pointer = Default::default());
        frame(vec![
            egui::Event::PointerMoved(origin),
            button(origin, true, egui::PointerButton::Primary),
        ]);
        assert!(frame(vec![button(origin, false, egui::PointerButton::Primary)]).clicked());
        frame(vec![button(origin, true, egui::PointerButton::Secondary)]);
        assert!(
            !frame(vec![egui::Event::PointerMoved(moved)])
                .drag_started_by(egui::PointerButton::Primary)
        );
    }

    #[test]
    fn scrolling_back_retains_the_newly_requested_page_and_bounds_the_cache() {
        let mut pages = VecDeque::new();
        for offset in [0, 128, 256, 384, 1024, 0] {
            insert_page(
                &mut pages,
                SearchPage {
                    total: 100_000,
                    offset,
                    rows: vec![],
                },
            );
        }
        assert_eq!(pages.len(), 4);
        assert_eq!(pages.back().unwrap().offset, 0);
        assert!(pages.iter().any(|page| page.offset == 0));
    }
}
