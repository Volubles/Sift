use eframe::egui::{self, Color32};

use crate::catalog::Sample;
use crate::collections::{Availability, Reference};

fn sample_button(label: String, selected: bool) -> egui::Button<'static> {
    egui::Button::new(label)
        .selected(selected)
        .sense(egui::Sense::click_and_drag())
}

pub(super) fn table_heading(ui: &mut egui::Ui, order: &mut crate::search::SortOrder) -> bool {
    use crate::search::{SortDirection, SortField};
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
    let mut rect = rect;
    rect.min.x += 34.0;
    let mut x = rect.left();
    let mut changed = false;
    for ((name, field), width) in [
        ("Filename", SortField::Filename),
        ("Format", SortField::Format),
        ("Length", SortField::Length),
        ("Rate", SortField::Rate),
        ("Channels", SortField::Channels),
    ]
    .into_iter()
    .zip(column_widths(rect.width()))
    {
        let cell =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(width, rect.height()))
                .intersect(rect);
        if cell.width() > 0.0 {
            let direction = if order.direction == SortDirection::Ascending {
                "ascending"
            } else {
                "descending"
            };
            let label = if order.field == field {
                format!("{name}, {direction}")
            } else {
                name.to_owned()
            };
            let response = ui.interact(
                cell,
                ui.id().with(("sort-column", name)),
                egui::Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    order.field == field,
                    &label,
                )
            });
            if response.hovered() {
                ui.painter().rect_filled(cell, 2.0, Color32::from_gray(48));
            }
            let painter = ui
                .painter()
                .with_clip_rect(cell.shrink2(egui::vec2(6.0, 0.0)));
            let text = painter.text(
                egui::pos2(x + 8.0, cell.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(13.0),
                if order.field == field {
                    Color32::from_rgb(135, 222, 194)
                } else {
                    Color32::GRAY
                },
            );
            if order.field == field {
                paint_sort_direction(
                    &painter,
                    egui::pos2(text.right() + 7.0, cell.center().y),
                    order.direction,
                    Color32::from_rgb(135, 222, 194),
                );
            }
            if response
                .on_hover_text(format!("Sort by {name}; click again to reverse"))
                .clicked()
            {
                order.toggle(field);
                changed = true;
            }
        }
        x += width;
        if field == SortField::Filename {
            let tags = tag_rect(rect);
            if tags.width() > 0.0 {
                ui.painter().text(
                    egui::pos2(tags.left() + 8.0, tags.center().y),
                    egui::Align2::LEFT_CENTER,
                    "Tags",
                    egui::FontId::proportional(13.0),
                    Color32::GRAY,
                );
                x += tags.width();
            }
        }
    }
    ui.separator();
    changed
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
    let widths = column_widths(rect.width());
    let mut x = rect.left();
    for (index, (value, width)) in values.iter().zip(widths).enumerate() {
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
        if index == 0 {
            x += tag_rect(rect).width();
        }
    }
}

fn column_widths(width: f32) -> [f32; 5] {
    [
        (width - 330.0 - tag_width(width)).max(80.0),
        70.0,
        80.0,
        90.0,
        90.0,
    ]
}

fn tag_width(width: f32) -> f32 {
    if width >= 640.0 { 140.0 } else { 0.0 }
}
pub(super) fn tag_rect(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(rect.left() + column_widths(rect.width())[0], rect.top()),
        egui::vec2(tag_width(rect.width()), rect.height()),
    )
}

pub(super) fn paint_sort_direction(
    painter: &egui::Painter,
    center: egui::Pos2,
    direction: crate::search::SortDirection,
    color: Color32,
) {
    let sign = if direction == crate::search::SortDirection::Ascending {
        -1.0
    } else {
        1.0
    };
    let points = vec![
        center + egui::vec2(0.0, 3.0 * sign),
        center + egui::vec2(-3.5, -2.5 * sign),
        center + egui::vec2(3.5, -2.5 * sign),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/sound_table.rs"
    ));
}
