use std::time::Duration;

use eframe::egui::{self, Color32, RichText};

use crate::audio::transport;
use crate::collections::Availability;
use crate::platform::DragWorker;

use super::Browser;

impl Browser {
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

    pub(super) fn preview(&mut self, ui: &mut egui::Ui) {
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

    pub(super) fn details(&mut self, ui: &mut egui::Ui) {
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
}
