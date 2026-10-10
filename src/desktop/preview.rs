use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};

use crate::audio::{player::Player, transport};
use crate::catalog::Sample;
use crate::collections::{Availability, Reference};
use crate::library::FileStamp;
use crate::waveform::{Waveform, Waveforms};
use crate::worker::{SourceRefresh, Workers};

use super::SoundKey;

/// Owns selection and preview generations; workers perform source inspection,
/// waveform analysis, and playback outside the UI thread.
pub(super) struct Preview {
    waveforms: Waveforms,
    player: Player,
    waveform: Option<Arc<Waveform>>,
    wave_generation: u64,
    wave_pending: bool,
    wave_error: Option<String>,
    audio_error: Option<String>,
    output_name: Option<String>,
    volume: f32,
    selected: Option<Reference>,
    selected_key: Option<SoundKey>,
    selected_availability: Availability,
    selected_stamp: Option<FileStamp>,
    source_generation: u64,
    source_check: Option<SourceRefresh>,
    auto_play: bool,
}

impl Preview {
    pub(super) fn new(waveforms: Waveforms, player: Player) -> Self {
        Self {
            waveforms,
            player,
            waveform: None,
            wave_generation: 0,
            wave_pending: false,
            wave_error: None,
            audio_error: None,
            output_name: None,
            volume: 0.4,
            selected: None,
            selected_key: None,
            selected_availability: Availability::Ready,
            selected_stamp: None,
            source_generation: 0,
            source_check: None,
            auto_play: false,
        }
    }

    pub(super) fn selected(&self) -> Option<&Reference> {
        self.selected.as_ref()
    }

    pub(super) fn key(&self) -> Option<SoundKey> {
        self.selected_key
    }

    pub(super) fn waveform_pending(&self) -> bool {
        self.wave_pending
    }

    pub(super) fn playback_phase(&self) -> u8 {
        self.player.snapshot().phase
    }

    pub(super) fn update_availability(&mut self, availability: &Availability) {
        if self.selected_availability != *availability {
            self.selected_availability = availability.clone();
        }
    }

    pub(super) fn refresh_catalog(&mut self, rows: &[Sample]) {
        if let Some(SoundKey::Library(id)) = self.selected_key {
            if let Some(sample) = rows.iter().find(|sample| sample.id == id) {
                self.selected = Some(Reference::from(sample));
            }
        } else if self.selected.is_none()
            && let Some(sample) = rows.first()
        {
            self.select(
                Reference::from(sample),
                SoundKey::Library(sample.id),
                Availability::Ready,
                sample.stamp.clone(),
            );
        }
    }

    pub(super) fn select(
        &mut self,
        sample: Reference,
        key: SoundKey,
        availability: Availability,
        stamp: Option<FileStamp>,
    ) {
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
        self.selected_stamp = stamp;
        self.source_generation += 1;
        self.source_check = Some(SourceRefresh::Verify);
    }

    pub(super) fn sources_changed(
        &mut self,
        touched: &[PathBuf],
        full_roots: &[PathBuf],
        forced_roots: &[PathBuf],
    ) {
        if self.selected.as_ref().is_some_and(|sound| {
            touched
                .iter()
                .chain(full_roots)
                .any(|path| sound.source.starts_with(path))
        }) {
            let observed_write = self.selected.as_ref().is_some_and(|sound| {
                touched.contains(&sound.source)
                    || forced_roots
                        .iter()
                        .any(|root| sound.source.starts_with(root))
            });
            self.source_generation += 1;
            self.source_check = Some(if observed_write {
                SourceRefresh::ObservedWrite
            } else {
                SourceRefresh::Verify
            });
        }
    }

    /// Returns whether a current reply refreshed the tracked source reference.
    pub(super) fn source_ready(
        &mut self,
        generation: u64,
        refresh: SourceRefresh,
        sample: Option<Sample>,
        stamp: Option<FileStamp>,
        availability: Availability,
    ) -> bool {
        if generation != self.source_generation {
            return false;
        }
        let mut reference_changed = false;
        let metadata_changed =
            sample
                .as_ref()
                .zip(self.selected.as_ref())
                .is_some_and(|(current, previous)| {
                    current.audio != previous.audio
                        || current.size_bytes != previous.size_bytes
                        || current.source != previous.source
                        || current.metadata_error != previous.metadata_error
                });
        let changed = metadata_changed
            || matches!(refresh, SourceRefresh::ObservedWrite)
            || self
                .selected_stamp
                .as_ref()
                .is_some_and(|previous| Some(previous) != stamp.as_ref());
        self.selected_stamp = stamp;
        self.selected_availability = availability.clone();
        if let Some(sample) = sample {
            if matches!(self.selected_key, Some(SoundKey::Library(_))) {
                self.selected_key = Some(SoundKey::Library(sample.id));
            }
            self.selected = Some(Reference::from(&sample));
            reference_changed = true;
        }
        if !availability.usable() {
            self.wave_generation += 1;
            self.wave_pending = false;
            self.waveform = None;
            self.wave_error = Some(match availability {
                Availability::Missing => {
                    "Source file is missing; saved references are retained".into()
                }
                Availability::Unavailable(message) => message,
                _ => "Source unavailable".into(),
            });
            if let Err(error) = self.player.stop() {
                self.audio_error = Some(error.to_string());
            }
        } else if changed {
            if let Err(error) = self.player.stop() {
                self.audio_error = Some(error.to_string());
            }
            self.wave_generation += 1;
            self.waveform = None;
            self.wave_error = None;
            if let Some(sound) = &self.selected {
                self.wave_pending = sound.metadata_error.is_none();
                if self.wave_pending
                    && let Err(error) = self
                        .waveforms
                        .rebuild(self.wave_generation, sound.source.clone())
                {
                    self.wave_error = Some(error.to_string());
                    self.wave_pending = false;
                }
            }
        }
        reference_changed
    }

    pub(super) fn queue_source_check(&mut self, workers: &Workers) {
        if let Some(refresh) = self.source_check
            && let Some(reference) = &self.selected
            && workers.check_source(self.source_generation, reference.clone(), refresh)
        {
            self.source_check = None;
        }
    }

    pub(super) fn poll(&mut self) {
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

    pub(super) fn toggle_preview(&mut self) {
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

    pub(super) fn ui(&mut self, ui: &mut egui::Ui) {
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

    pub(super) fn details(&self, ui: &mut egui::Ui, drag_enabled: bool) {
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
                drag_enabled,
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
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/preview.rs"
    ));
}
