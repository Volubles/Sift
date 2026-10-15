use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use eframe::egui;

use super::Sidebar;

pub(super) struct Diagnostics {
    capture: Option<Capture>,
    report: Option<FrameReport>,
    report_writer: Option<JoinHandle<std::io::Result<()>>>,
}

struct Capture {
    path: PathBuf,
    subfolder: Option<PathBuf>,
    sidebar: Sidebar,
    requested: bool,
    writer: Option<JoinHandle<std::io::Result<()>>>,
    passed: Arc<AtomicBool>,
    started: Instant,
}

struct FrameReport {
    path: PathBuf,
    frames: u64,
    input_events: u64,
}

pub(super) struct CaptureStatus<'a> {
    pub ready: bool,
    pub results: usize,
    pub cached_records: usize,
    pub error: Option<&'a str>,
}

impl Diagnostics {
    pub(super) fn new(path: Option<PathBuf>, passed: Arc<AtomicBool>) -> Self {
        Self {
            capture: path.map(|path| Capture {
                path,
                subfolder: std::env::var_os("SIFT_SMOKE_SUBFOLDER").map(PathBuf::from),
                sidebar: match std::env::var("SIFT_SMOKE_VIEW").as_deref() {
                    Ok("favorites") => Sidebar::Favorites,
                    Ok("collections") => Sidebar::Collections,
                    _ => Sidebar::default(),
                },
                requested: false,
                writer: None,
                passed,
                started: Instant::now(),
            }),
            report: std::env::var_os("SIFT_FRAME_REPORT").map(|path| FrameReport {
                path: PathBuf::from(path),
                frames: 0,
                input_events: 0,
            }),
            report_writer: None,
        }
    }

    pub(super) fn sidebar(&self) -> Sidebar {
        self.capture
            .as_ref()
            .map_or(Sidebar::default(), |capture| capture.sidebar)
    }

    pub(super) fn take_subfolder(&mut self) -> Option<PathBuf> {
        self.capture.as_mut()?.subfolder.take()
    }

    pub(super) fn capturing(&self) -> bool {
        self.capture.is_some()
    }

    pub(super) fn frame(&mut self, context: &egui::Context) {
        if let Some(report) = &mut self.report {
            report.frames += 1;
            report.input_events += context.input(|input| input.events.len()) as u64;
        }
    }

    pub(super) fn poll_capture(&mut self, context: &egui::Context, status: CaptureStatus<'_>) {
        let Some(capture) = &mut self.capture else {
            return;
        };
        if status.ready && !capture.requested {
            capture.requested = true;
            context.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        if capture.requested && capture.writer.is_none() {
            let image = context.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = image {
                let path = capture.path.clone();
                match thread::Builder::new()
                    .name("sift-smoke-capture".into())
                    .spawn(move || save_capture(path, &image))
                {
                    Ok(writer) => capture.writer = Some(writer),
                    Err(error) => {
                        eprintln!("Sift capture worker failed: {error}");
                        context.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        if capture
            .writer
            .as_ref()
            .is_some_and(|writer| writer.is_finished())
            && let Some(writer) = capture.writer.take()
        {
            match writer.join() {
                Ok(Ok(())) => {
                    capture.passed.store(true, Ordering::Release);
                    eprintln!(
                        "Sift native window smoke capture complete; {} results, {} cached records",
                        status.results, status.cached_records
                    );
                }
                Ok(Err(error)) => eprintln!("Sift smoke capture failed: {error}"),
                Err(_) => eprintln!("Sift smoke capture worker panicked"),
            }
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if capture.started.elapsed() > Duration::from_secs(30) {
            eprintln!("Sift smoke capture timed out: {:?}", status.error);
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            context.request_repaint_after(Duration::from_millis(50));
        }
    }

    pub(super) fn finish(&mut self, phase: u8) {
        let Some(report) = self.report.take() else {
            return;
        };
        let text = format!(
            "frames={} input_events={} phase={phase}\n",
            report.frames, report.input_events
        );
        match thread::Builder::new()
            .name("sift-frame-report".into())
            .spawn(move || write_new(report.path, text.as_bytes()))
        {
            Ok(writer) => self.report_writer = Some(writer),
            Err(error) => eprintln!("Frame diagnostic worker could not start: {error}"),
        }
    }
}

fn save_capture(path: PathBuf, image: &egui::ColorImage) -> std::io::Result<()> {
    let mut bytes = format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
    bytes.reserve(image.width() * image.height() * 3);
    for pixel in &image.pixels {
        bytes.extend_from_slice(&pixel.to_array()[..3]);
    }
    write_new(path, &bytes)
}

fn write_new(path: PathBuf, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)
}

impl Drop for Diagnostics {
    fn drop(&mut self) {
        if let Some(capture) = &mut self.capture
            && let Some(writer) = capture.writer.take()
        {
            match writer.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("Sift capture failed during shutdown: {error}"),
                Err(_) => eprintln!("Sift capture worker panicked during shutdown"),
            }
        }
        if let Some(writer) = self.report_writer.take() {
            match writer.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("Frame diagnostic failed: {error}"),
                Err(_) => eprintln!("Frame diagnostic worker panicked during shutdown"),
            }
        }
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/diagnostics.rs"
    ));
}
