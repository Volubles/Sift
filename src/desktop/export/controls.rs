use crate::export::{Channels, Depth, Format, Mp3Quality, SAMPLE_RATES, Settings};
use eframe::egui;

pub(super) fn settings(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    id: impl std::hash::Hash + std::fmt::Debug,
) -> bool {
    let before = settings.clone();
    ui.push_id(id, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label("Format");
            egui::ComboBox::from_id_salt("format")
                .selected_text(settings.format.label())
                .show_ui(ui, |ui| {
                    for format in Format::ALL {
                        ui.selectable_value(&mut settings.format, format, format.label());
                    }
                });
            if settings.format == Format::Flac && settings.depth == Depth::Float32 {
                settings.depth = Depth::Int24;
            }
            ui.add_enabled_ui(settings.format != Format::Original, |ui| {
                egui::ComboBox::from_id_salt("channels")
                    .selected_text(settings.channels.label())
                    .show_ui(ui, |ui| {
                        for channels in [Channels::Original, Channels::Mono, Channels::Stereo] {
                            ui.selectable_value(&mut settings.channels, channels, channels.label());
                        }
                    });
                egui::ComboBox::from_id_salt("rate")
                    .selected_text(settings.sample_rate.map_or("Original rate".into(), |r| format!("{r} Hz")))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut settings.sample_rate, None, "Original rate");
                        for rate in SAMPLE_RATES {
                            ui.selectable_value(&mut settings.sample_rate, Some(rate), format!("{rate} Hz"));
                        }
                    });
                if matches!(settings.format, Format::Wav | Format::Aiff | Format::Flac) {
                    egui::ComboBox::from_id_salt("depth")
                        .selected_text(settings.depth.label())
                        .show_ui(ui, |ui| {
                            for depth in [Depth::Int16, Depth::Int24, Depth::Float32] {
                                if settings.format != Format::Flac || depth != Depth::Float32 {
                                    ui.selectable_value(&mut settings.depth, depth, depth.label());
                                }
                            }
                        });
                }
            });
        });
        match settings.format {
            Format::Original => {
                ui.weak("Copy original bytes and metadata. Encoding controls are unused.");
            }
            Format::Flac => {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Compression");
                    ui.add(egui::Slider::new(&mut settings.flac_compression, 0..=12));
                    ui.weak("Size / encoding time; audio stays lossless at the selected bit depth.");
                });
            }
            Format::Mp3 => {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Quality");
                    egui::ComboBox::from_id_salt("mp3-quality")
                        .selected_text(mp3_label(settings.mp3))
                        .show_ui(ui, |ui| {
                            for rate in [64, 96, 128, 160, 192, 224, 256, 320] {
                                ui.selectable_value(&mut settings.mp3, Mp3Quality::Bitrate(rate), format!("{rate} kbps"));
                            }
                            for q in 0..=9 {
                                ui.selectable_value(&mut settings.mp3, Mp3Quality::Vbr(q), format!("VBR {q}{}", if q == 0 { " (highest)" } else { "" }));
                            }
                        });
                    ui.weak("Lossy encoding; converting to a higher bitrate cannot restore lost detail.");
                });
            }
            Format::Ogg => {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Vorbis quality");
                    ui.add(egui::Slider::new(&mut settings.vorbis_quality, 0..=10));
                    ui.weak("Higher quality uses larger files. Lossy encoding.");
                });
            }
            _ => {}
        }
    });
    before != *settings
}
fn mp3_label(quality: Mp3Quality) -> String {
    match quality {
        Mp3Quality::Bitrate(rate) => format!("{rate} kbps"),
        Mp3Quality::Vbr(q) => format!("VBR {q}"),
    }
}
