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
use crate::library::ScanReport;
use crate::platform::{DragError, DragOutcome, DragWorker};
use crate::search::{PAGE_SIZE, SearchQuery};
use crate::waveform::{Waveform, Waveforms};
use crate::worker::{Event, Workers};

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
            Ok(Box::new(Browser::new(
                workers,
                waveforms,
                player,
                drags,
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
    drag_notice: Option<DragNotice>,
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
    selected: Option<Sample>,
    folder: Option<PathBuf>,
    smoke: Option<PathBuf>,
    screenshot_requested: bool,
    capture_writer: Option<JoinHandle<std::io::Result<()>>>,
    capture_passed: Arc<AtomicBool>,
    started: Instant,
    rendered_frames: u64,
    input_events: u64,
}

enum DragNotice {
    Message(&'static str),
    Error(String),
}

impl Browser {
    fn new(
        workers: Workers,
        waveforms: Waveforms,
        player: Player,
        drags: DragWorker,
        options: Options,
        capture_passed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            workers,
            waveforms,
            player,
            drags,
            drag_notice: None,
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
            && self
                .workers
                .query(self.generation, self.query.clone(), offset)
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
        while let Some(result) = self.drags.poll() {
            // OLE receives the final mouse release on its own thread. Reset egui
            // so the next click/drag cannot inherit a stuck pointer or widget ID.
            ctx.stop_dragging();
            ctx.input_mut(|input| input.pointer = Default::default());
            self.drag_notice = Some(match result {
                Ok(DragOutcome::Accepted) => DragNotice::Message(
                    "Copy handed to the destination. Check its progress and any conflict prompts.",
                ),
                Ok(DragOutcome::Cancelled) => DragNotice::Message("File drag cancelled."),
                Ok(DragOutcome::Rejected) => DragNotice::Message(
                    "The destination did not accept the file. Try a writable folder in Explorer.",
                ),
                Err(error) => {
                    if matches!(error, DragError::Native(_) | DragError::Disconnected) {
                        eprintln!("Native file drag failed: {error}");
                    }
                    DragNotice::Error(format!("Could not drag the file: {error}"))
                }
            });
        }
        while let Some(event) = self.workers.try_event() {
            match event {
                Event::Ready => {
                    self.ready = true;
                    self.request_page(0);
                    if let Some(folder) = self.folder.take() {
                        self.begin_scan(folder);
                    }
                }
                Event::Page { generation, result } if generation == self.generation => match result
                {
                    Ok(page) => {
                        self.total = page.total;
                        self.pending.remove(&page.offset);
                        if let Some(selected) = &self.selected {
                            if let Some(updated) =
                                page.rows.iter().find(|row| row.id == selected.id)
                            {
                                self.selected = Some(updated.clone());
                            }
                        } else {
                            if let Some(sample) = page.rows.first().cloned() {
                                self.select(sample);
                            }
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

    fn select(&mut self, sample: Sample) {
        let autoplay = matches!(
            self.player.snapshot().phase,
            transport::PLAYING | transport::LOADING
        );
        self.wave_generation += 1;
        self.waveform = None;
        self.wave_error = None;
        self.audio_error = None;
        self.wave_pending = sample.metadata_error.is_none();
        if self.wave_pending
            && let Err(error) = self
                .waveforms
                .request(self.wave_generation, sample.source.clone())
        {
            self.wave_error = Some(error.to_string());
            self.wave_pending = false;
        }
        let result = if autoplay && sample.metadata_error.is_none() {
            self.player.play(sample.source.clone(), 0.0)
        } else {
            self.player.stop()
        };
        if let Err(error) = result {
            self.audio_error = Some(error.to_string());
        }
        self.selected = Some(sample);
    }

    fn begin_drag(&mut self, sample: Sample) {
        if self.selected.as_ref().map(|selected| selected.id) != Some(sample.id) {
            self.select(sample.clone());
        }
        self.drag_notice = match self.drags.request(vec![sample.source]) {
            Ok(()) => None,
            Err(error) => {
                if matches!(error, DragError::Native(_) | DragError::Disconnected) {
                    eprintln!("Native file drag could not start: {error}");
                }
                Some(DragNotice::Error(format!(
                    "Could not drag the file: {error}"
                )))
            }
        };
    }

    fn toggle_preview(&mut self) {
        let Some(sample) = &self.selected else {
            return;
        };
        if sample.metadata_error.is_some() {
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
            .is_some_and(|sample| sample.metadata_error.is_none());
        ui.horizontal(|ui| {
            let label = if matches!(snapshot.phase, transport::PLAYING | transport::LOADING) {
                "Pause"
            } else {
                "Play"
            };
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
                if ui
                    .add_enabled(
                        cfg!(windows) && self.ready && self.scan.is_none(),
                        egui::Button::new("Add folder"),
                    )
                    .clicked()
                {
                    #[cfg(windows)]
                    if let Some(folder) = rfd::FileDialog::new()
                        .set_title("Choose a sample library")
                        .pick_folder()
                    {
                        self.begin_scan(folder);
                    }
                }
                if self.scan.is_some() && ui.button("Cancel scan").clicked() {
                    self.workers.cancel_scan();
                }
            });
        });
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
            ui.label("Dragging a copy. Release in a folder, or press Esc to cancel.");
        } else if let Some(notice) = &self.drag_notice {
            match notice {
                DragNotice::Message(message) => {
                    ui.label(*message);
                }
                DragNotice::Error(message) => {
                    ui.colored_label(Color32::from_rgb(245, 143, 140), message);
                }
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
                self.begin_drag(sample);
            }
        } else {
            ui.label("Choose a sound to inspect its metadata.");
        }
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Library");
            ui.label(RichText::new("LOCAL LIBRARY").small().weak());
        });
        ui.separator();
        if DragWorker::supported() {
            ui.label(
                RichText::new("Drag a sound to an Explorer folder to copy it.")
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
                ui.label("Add a sample folder to get started. Source files stay in place.");
            } else {
                ui.label("No matching sounds.");
            }
            return;
        }
        let mut needed = BTreeSet::new();
        let selected_id: Option<SampleId> = self.selected.as_ref().map(|sample| sample.id);
        let mut drag_sample = None;
        egui::ScrollArea::vertical()
            .id_salt(("samples", self.generation))
            .auto_shrink([false, false])
            .show_rows(ui, 32.0, self.total, |ui, range| {
                for index in range {
                    let offset = index / PAGE_SIZE * PAGE_SIZE;
                    needed.insert(offset);
                    if let Some(sample) = self
                        .pages
                        .iter()
                        .find(|page| page.offset == offset)
                        .and_then(|page| page.rows.get(index - offset))
                    {
                        let label = format!(
                            "{}    {}    {}",
                            sample.name,
                            sample.format.to_uppercase(),
                            sample
                                .audio
                                .duration_seconds
                                .map(|value| format!("{value:.2} s"))
                                .unwrap_or_else(|| "Unknown duration".into())
                        );
                        let response = ui.add_sized(
                            [ui.available_width(), 32.0],
                            sample_button(label, selected_id == Some(sample.id)),
                        );
                        let clicked = response.clicked().then(|| sample.clone());
                        if DragWorker::supported()
                            && !self.drags.busy()
                            && response.drag_started_by(egui::PointerButton::Primary)
                        {
                            drag_sample = Some(sample.clone());
                        }
                        response.on_hover_text(format!(
                            "{}\nDrag to a folder to copy the source file.",
                            sample.display_path
                        ));
                        if let Some(sample) = clicked {
                            self.select(sample);
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
            self.begin_drag(sample);
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
        egui::Panel::top("header").show(ui, |ui| self.header(ui));
        egui::Panel::bottom("footer").show(ui, |ui| {
            ui.label(
                RichText::new(
                    "Source files stay in their original folders. Your library is saved locally.",
                )
                .small()
                .weak(),
            );
        });
        egui::Panel::bottom("preview").show(ui, |ui| self.preview(ui));
        egui::Panel::right("details")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| self.details(ui));
        egui::CentralPanel::default().show(ui, |ui| self.list(ui));
        if !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        {
            self.toggle_preview();
        }
        self.smoke_check(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
