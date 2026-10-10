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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::desktop::external_drag_requested;

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
}
