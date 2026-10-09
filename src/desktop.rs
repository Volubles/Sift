use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::catalog::{Sample, SampleId, SearchPage};
use crate::library::ScanReport;
use crate::search::{PAGE_SIZE, SearchQuery};
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
            Ok(Box::new(Browser::new(workers, options, capture_passed)))
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
}

impl Browser {
    fn new(workers: Workers, options: Options, capture_passed: Arc<AtomicBool>) -> Self {
        Self {
            workers,
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

    fn accept_events(&mut self) {
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
                            self.selected = page.rows.first().cloned();
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

    fn details(&self, ui: &mut egui::Ui) {
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
        } else {
            ui.label("Choose a sound to inspect its metadata.");
        }
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Library");
            ui.label(RichText::new("METADATA PROTOTYPE").small().weak());
        });
        ui.separator();
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
                            egui::Button::new(label).selected(selected_id == Some(sample.id)),
                        );
                        if response.clicked() {
                            self.selected = Some(sample.clone());
                        }
                        response.on_hover_text(&sample.display_path);
                    } else {
                        ui.add_sized(
                            [ui.available_width(), 32.0],
                            egui::Label::new("Loading sounds..."),
                        );
                    }
                }
            });
        for offset in needed {
            self.request_page(offset);
        }
    }

    fn smoke_check(&mut self, ctx: &egui::Context) {
        let Some(path) = self.smoke.clone() else {
            return;
        };
        if self.ready && self.scan.is_none() && !self.pages.is_empty() && !self.screenshot_requested
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
    fn logic(&mut self, _context: &egui::Context, _frame: &mut eframe::Frame) {
        self.accept_events();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
        egui::Panel::right("details")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| self.details(ui));
        egui::CentralPanel::default().show(ui, |ui| self.list(ui));
        self.smoke_check(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
