use crate::search::filters::{
    Channels, Format, Formats, Length, MetadataFilters, SampleRate, UnknownLengths,
};
use eframe::egui::{self, Color32};

pub(super) const BUTTON_WIDTH: f32 = 88.0;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum LengthMode {
    #[default]
    Any,
    Range,
    Unknown,
}

#[derive(Default)]
pub(super) struct Editor {
    draft: Draft,
}

#[derive(Default)]
struct Draft {
    length: LengthMode,
    min: String,
    max: String,
    include_unknown: bool,
    formats: Formats,
    channels: Channels,
    rate: SampleRate,
}

impl From<MetadataFilters> for Draft {
    fn from(filters: MetadataFilters) -> Self {
        let mut draft = Self {
            formats: filters.formats,
            channels: filters.channels,
            rate: filters.sample_rate,
            ..Default::default()
        };
        match filters.length {
            Length::Any => {}
            Length::Unknown => draft.length = LengthMode::Unknown,
            Length::Range { min, max, unknown } => {
                draft.length = LengthMode::Range;
                draft.min = min.map(|value| value.to_string()).unwrap_or_default();
                draft.max = max.map(|value| value.to_string()).unwrap_or_default();
                draft.include_unknown = unknown == UnknownLengths::Include;
            }
        }
        draft
    }
}

impl Draft {
    fn filters(&self) -> Result<MetadataFilters, String> {
        let number = |text: &str| {
            if text.trim().is_empty() {
                Ok(None)
            } else {
                text.trim()
                    .parse::<f64>()
                    .map(Some)
                    .map_err(|_| "Use decimal seconds, for example 0.5".to_owned())
            }
        };
        let length = match self.length {
            LengthMode::Any => Length::Any,
            LengthMode::Unknown => Length::Unknown,
            LengthMode::Range if self.min.trim().is_empty() && self.max.trim().is_empty() => {
                Length::Any
            }
            LengthMode::Range => Length::Range {
                min: number(&self.min)?,
                max: number(&self.max)?,
                unknown: if self.include_unknown {
                    UnknownLengths::Include
                } else {
                    UnknownLengths::Exclude
                },
            },
        };
        let filters = MetadataFilters {
            length,
            formats: self.formats,
            channels: self.channels,
            sample_rate: self.rate,
        };
        filters.validate().map_err(|error| error.to_string())?;
        Ok(filters)
    }
}

impl Editor {
    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        current: MetadataFilters,
    ) -> Option<MetadataFilters> {
        let count = current.active_count();
        let response = ui.add_sized(
            [BUTTON_WIDTH, 34.0],
            egui::Button::new(if count == 0 {
                "Filters".into()
            } else {
                format!("Filters ({count})")
            }),
        );
        if response.clicked() {
            self.draft = current.into();
        }
        let mut changed = None;
        let mut close_submenu = None;
        egui::Popup::menu(&response)
            .width(340.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.heading("Metadata filters");
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Length");
                    // Submenus share the panel's root; ComboBox would replace it.
                    close_submenu = choice_menu(
                        ui,
                        match self.draft.length {
                            LengthMode::Any => "Any",
                            LengthMode::Range => "Range",
                            LengthMode::Unknown => "Unknown",
                        },
                        |ui| {
                            let mut chosen = false;
                            for (mode, label) in [
                                (LengthMode::Any, "Any"),
                                (LengthMode::Range, "Range"),
                                (LengthMode::Unknown, "Unknown"),
                            ] {
                                chosen |= ui
                                    .selectable_value(&mut self.draft.length, mode, label)
                                    .clicked();
                            }
                            chosen
                        },
                    )
                    .or(close_submenu);
                });
                if self.draft.length == LengthMode::Range {
                    ui.horizontal(|ui| {
                        ui.label("Min");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.draft.min)
                                .char_limit(16)
                                .desired_width(74.0)
                                .hint_text("Any"),
                        );
                        ui.label("Max");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.draft.max)
                                .char_limit(16)
                                .desired_width(74.0)
                                .hint_text("Any"),
                        );
                        ui.label("seconds");
                    });
                    ui.add_enabled(
                        !self.draft.min.trim().is_empty() || !self.draft.max.trim().is_empty(),
                        egui::Checkbox::new(
                            &mut self.draft.include_unknown,
                            "Include unknown lengths",
                        ),
                    );
                }
                ui.separator();
                ui.label("Format (no selection means any)");
                ui.horizontal(|ui| {
                    for format in Format::ALL {
                        let mut selected = self.draft.formats.contains(format);
                        if ui.checkbox(&mut selected, format.label()).changed() {
                            self.draft.formats.set(format, selected);
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Channels");
                    close_submenu = choice_menu(ui, channel_label(self.draft.channels), |ui| {
                        let mut chosen = false;
                        for channels in [
                            Channels::Any,
                            Channels::Mono,
                            Channels::Stereo,
                            Channels::Unknown,
                        ] {
                            chosen |= ui
                                .selectable_value(
                                    &mut self.draft.channels,
                                    channels,
                                    channel_label(channels),
                                )
                                .clicked();
                        }
                        chosen
                    })
                    .or(close_submenu);
                });
                ui.horizontal(|ui| {
                    ui.label("Sample rate");
                    close_submenu = choice_menu(ui, &rate_label(self.draft.rate), |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .show(ui, |ui| {
                                let mut chosen = ui
                                    .selectable_value(&mut self.draft.rate, SampleRate::Any, "Any")
                                    .clicked();
                                for hz in [
                                    8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000,
                                    176400, 192000, 384000,
                                ] {
                                    chosen |= ui
                                        .selectable_value(
                                            &mut self.draft.rate,
                                            SampleRate::Hz(hz),
                                            rate_label(SampleRate::Hz(hz)),
                                        )
                                        .clicked();
                                }
                                chosen |= ui
                                    .selectable_value(
                                        &mut self.draft.rate,
                                        SampleRate::Unknown,
                                        "Unknown",
                                    )
                                    .clicked();
                                chosen
                            })
                            .inner
                    })
                    .or(close_submenu);
                });
                let result = self.draft.filters();
                match result {
                    Ok(filters) if filters != current => changed = Some(filters),
                    Ok(_) => {}
                    Err(error) => {
                        ui.colored_label(Color32::from_rgb(245, 143, 140), error);
                    }
                }
                ui.separator();
                if ui.button("Close").clicked() {
                    ui.close();
                }
            });
        if let Some(id) = close_submenu {
            // Keep the submenu registered until the panel has handled its click.
            egui::containers::menu::MenuState::from_id(ui.ctx(), id, |state| {
                state.open_item = None
            });
        }
        changed
    }
}

fn choice_menu(
    ui: &mut egui::Ui,
    label: &str,
    choices: impl FnOnce(&mut egui::Ui) -> bool,
) -> Option<egui::Id> {
    (ui.menu_button(label, choices).inner == Some(true))
        .then(|| egui::containers::menu::find_menu_root(ui).id)
}

fn channel_label(value: Channels) -> &'static str {
    match value {
        Channels::Any => "Any",
        Channels::Mono => "Mono",
        Channels::Stereo => "Stereo",
        Channels::Unknown => "Unknown",
    }
}
fn rate_label(value: SampleRate) -> String {
    match value {
        SampleRate::Any => "Any".into(),
        SampleRate::Unknown => "Unknown".into(),
        SampleRate::Hz(hz) => format!("{} kHz", f64::from(hz) / 1000.0),
    }
}

pub(super) fn active(ui: &mut egui::Ui, current: MetadataFilters) -> Option<MetadataFilters> {
    if current.active_count() == 0 {
        return None;
    }
    let mut next = current;
    ui.horizontal_wrapped(|ui| {
        let length = match current.length {
            Length::Any => None,
            Length::Unknown => Some("Length: Unknown".into()),
            Length::Range { min, max, unknown } => Some(format!(
                "Length: {}–{} s{}",
                min.map(|value| value.to_string())
                    .unwrap_or_else(|| "0".into()),
                max.map(|value| value.to_string())
                    .unwrap_or_else(|| "Any".into()),
                if unknown == UnknownLengths::Include {
                    " + unknown"
                } else {
                    ""
                }
            )),
        };
        if let Some(label) = length
            && ui.small_button(format!("{label} ×")).clicked()
        {
            next.length = Length::Any;
        }
        if !current.formats.is_empty() {
            let label = Format::ALL
                .into_iter()
                .filter(|format| current.formats.contains(*format))
                .map(Format::label)
                .collect::<Vec<_>>()
                .join(", ");
            if ui.small_button(format!("{label} ×")).clicked() {
                next.formats = Formats::default();
            }
        }
        if current.channels != Channels::Any
            && ui
                .small_button(format!("{} ×", channel_label(current.channels)))
                .clicked()
        {
            next.channels = Channels::Any;
        }
        if current.sample_rate != SampleRate::Any
            && ui
                .small_button(format!("{} ×", rate_label(current.sample_rate)))
                .clicked()
        {
            next.sample_rate = SampleRate::Any;
        }
        if ui.small_button("Clear filters").clicked() {
            next = MetadataFilters::default();
        }
    });
    (next != current).then_some(next)
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/filters.rs"
    ));
}
