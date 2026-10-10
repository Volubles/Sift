use eframe::egui::{self, Color32};

use crate::catalog::Sample;
use crate::collections::{Availability, Reference};

fn sample_button(label: String, selected: bool) -> egui::Button<'static> {
    egui::Button::new(label)
        .selected(selected)
        .sense(egui::Sense::click_and_drag())
}

pub(super) fn table_heading(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
    let mut rect = rect;
    rect.min.x += 34.0;
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

struct SoundRow<'a> {
    name: &'a str,
    format: &'a str,
    audio: &'a crate::library::AudioMetadata,
}

pub(super) fn table_row(
    ui: &mut egui::Ui,
    sound: &Sample,
    selected: bool,
    index: usize,
) -> egui::Response {
    sound_row(
        ui,
        SoundRow {
            name: &sound.name,
            format: &sound.format,
            audio: &sound.audio,
        },
        &Availability::Ready,
        selected,
        index,
    )
}

pub(super) fn reference_row(
    ui: &mut egui::Ui,
    sound: &Reference,
    availability: &Availability,
    selected: bool,
    index: usize,
) -> egui::Response {
    sound_row(
        ui,
        SoundRow {
            name: &sound.name,
            format: &sound.format,
            audio: &sound.audio,
        },
        availability,
        selected,
        index,
    )
}

fn sound_row(
    ui: &mut egui::Ui,
    sound: SoundRow<'_>,
    availability: &Availability,
    selected: bool,
    index: usize,
) -> egui::Response {
    let response = ui.add_sized(
        [ui.available_width(), 28.0],
        sample_button(String::new(), selected),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            selected,
            sound.name,
        )
    });
    if !selected && !response.hovered() && index.is_multiple_of(2) {
        ui.painter()
            .rect_filled(response.rect, 0.0, Color32::from_rgb(37, 39, 40));
    }
    let columns = [
        format!(
            "{}{}",
            match availability {
                Availability::Ready => "",
                Availability::Changed => "Changed · ",
                Availability::Missing => "Missing · ",
                Availability::Unavailable(_) => "Unavailable · ",
            },
            sound.name
        ),
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

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/sound_table.rs"
    ));
}
