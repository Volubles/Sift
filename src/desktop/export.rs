use crate::{
    collections::Reference,
    export::{
        self, Conflict, Item, Outcome, PreviewRow, Request, Settings,
        worker::{Event, Input, Worker},
    },
};
use eframe::egui::{self, Color32, RichText};
use std::{path::PathBuf, sync::atomic::Ordering, thread::JoinHandle};
mod controls;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Loading,
    Editing,
    Reviewing,
    Ready,
    Running,
    Finished,
}
struct Row {
    reference: Reference,
    stem: String,
    custom: Option<Settings>,
    preview: Option<PreviewRow>,
    outcome: Option<Outcome>,
}
pub(super) struct Export {
    catalog: PathBuf,
    worker: Option<Worker>,
    visible: bool,
    notice: bool,
    phase: Phase,
    revision: u64,
    rows: Vec<Row>,
    selected: usize,
    common: Settings,
    prefix: String,
    suffix: String,
    numbering: bool,
    directory: PathBuf,
    conflict: Conflict,
    replace_confirmed: bool,
    error: Option<String>,
    running: Option<usize>,
    folder_open: Option<JoinHandle<std::io::Result<()>>>,
}
impl Export {
    pub fn new(catalog: PathBuf) -> Self {
        Self {
            catalog,
            worker: None,
            visible: false,
            notice: false,
            phase: Phase::Editing,
            revision: 0,
            rows: Vec::new(),
            selected: 0,
            common: Settings::default(),
            prefix: String::new(),
            suffix: String::new(),
            numbering: false,
            directory: PathBuf::new(),
            conflict: Conflict::KeepBoth,
            replace_confirmed: false,
            error: None,
            running: None,
            folder_open: None,
        }
    }
    pub fn is_open(&self) -> bool {
        self.visible
    }
    pub fn can_open(&self) -> bool {
        !matches!(
            self.phase,
            Phase::Loading | Phase::Reviewing | Phase::Running
        )
    }
    pub fn open(&mut self, input: Result<Input, String>, ctx: &egui::Context) {
        if !self.can_open() {
            self.visible = true;
            return;
        }
        self.visible = true;
        self.revision += 1;
        self.rows.clear();
        self.selected = 0;
        self.error = None;
        self.notice = false;
        self.running = None;
        self.replace_confirmed = false;
        let result = (|| {
            if self.worker.is_none() {
                let context = ctx.clone();
                self.worker = Some(
                    Worker::start(self.catalog.clone(), move || context.request_repaint())
                        .map_err(|e| e.to_string())?,
                );
            }
            self.worker
                .as_mut()
                .ok_or("Export worker is unavailable")?
                .load(self.revision, input?)
        })();
        match result {
            Ok(()) => self.phase = Phase::Loading,
            Err(error) => {
                self.phase = Phase::Editing;
                self.error = Some(error);
            }
        }
    }
    fn invalidate(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel();
        }
        self.revision += 1;
        self.phase = Phase::Editing;
        self.notice = false;
        self.running = None;
        self.replace_confirmed = false;
        self.error = None;
        for row in &mut self.rows {
            row.preview = None;
            row.outcome = None;
        }
    }
    pub fn poll(&mut self) {
        while let Some(event) = self.worker.as_ref().and_then(Worker::poll) {
            match event {
                Event::Loaded { revision, result } if revision == self.revision => match result {
                    Ok(references) => {
                        self.rows = references
                            .into_iter()
                            .map(|reference| Row {
                                stem: export::default_stem(&reference),
                                reference,
                                custom: None,
                                preview: None,
                                outcome: None,
                            })
                            .collect();
                        self.phase = Phase::Editing;
                    }
                    Err(error) => {
                        self.phase = Phase::Editing;
                        self.error = Some(error);
                    }
                },
                Event::Reviewed { revision, result } if revision == self.revision => match result {
                    Ok(rows) => {
                        for (row, preview) in self.rows.iter_mut().zip(rows) {
                            row.preview = Some(preview);
                        }
                        self.phase = Phase::Ready;
                    }
                    Err(error) => {
                        self.phase = Phase::Editing;
                        self.error = Some(error);
                    }
                },
                Event::Started { revision, index } if revision == self.revision => {
                    self.running = Some(index)
                }
                Event::Item {
                    revision,
                    index,
                    outcome,
                } if revision == self.revision => {
                    if let Some(row) = self.rows.get_mut(index) {
                        row.outcome = Some(outcome);
                    }
                }
                Event::Finished { revision } if revision == self.revision => {
                    self.phase = Phase::Finished;
                    self.running = None;
                    self.notice = true;
                }
                Event::Failed { revision, error } if revision == self.revision => {
                    self.phase = Phase::Editing;
                    self.error = Some(error);
                    self.running = None;
                }
                _ => {}
            }
        }
        if self
            .folder_open
            .as_ref()
            .is_some_and(|thread| thread.is_finished())
            && let Some(thread) = self.folder_open.take()
        {
            match thread.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    self.error = Some(format!("Destination could not be opened: {error}"))
                }
                Err(_) => self.error = Some("Destination opener failed".into()),
            }
        }
    }
    fn name(&self, index: usize) -> String {
        let number = if self.numbering {
            format!("_{:03}", index + 1)
        } else {
            String::new()
        };
        format!(
            "{}{}{}{}",
            self.prefix, self.rows[index].stem, self.suffix, number
        )
    }
    fn review(&mut self) {
        let request = Request {
            directory: self.directory.clone(),
            conflict: self.conflict,
            items: self
                .rows
                .iter()
                .enumerate()
                .map(|(index, row)| Item {
                    reference: row.reference.clone(),
                    stem: self.name(index),
                    settings: row.custom.as_ref().unwrap_or(&self.common).clone(),
                })
                .collect(),
        };
        let result = self
            .worker
            .as_mut()
            .ok_or_else(|| "Export worker is unavailable".into())
            .and_then(|worker| worker.review(self.revision, request));
        match result {
            Ok(()) => {
                self.phase = Phase::Reviewing;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn start(&mut self) {
        let result = self
            .worker
            .as_mut()
            .ok_or_else(|| "Export worker is unavailable".into())
            .and_then(|worker| worker.run(self.revision));
        match result {
            Ok(()) => {
                self.phase = Phase::Running;
                self.notice = true;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn choose_folder(&mut self, _ctx: &egui::Context) {
        #[cfg(windows)]
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Export sounds to folder")
            .pick_folder()
        {
            self.directory = path;
            self.invalidate();
        }
        #[cfg(not(windows))]
        {
            self.error = Some("Native folder selection is currently supported on Windows".into());
        }
    }
    fn open_folder(&mut self, ctx: &egui::Context) {
        if self.folder_open.is_some() {
            return;
        }
        let path = self.directory.clone();
        let context = ctx.clone();
        match std::thread::Builder::new()
            .name("sift-export-reveal".into())
            .spawn(move || {
                let result = reveal_folder(path);
                context.request_repaint();
                result
            }) {
            Ok(thread) => self.folder_open = Some(thread),
            Err(error) => self.error = Some(format!("Destination opener could not start: {error}")),
        }
    }
    pub fn status(&mut self, ui: &mut egui::Ui) {
        if !self.notice {
            return;
        }
        let completed = self.rows.iter().filter(|row| row.outcome.is_some()).count();
        let label = if self.phase == Phase::Running {
            format!("Exporting {completed}/{}", self.rows.len())
        } else {
            let (ok, skipped, failed, cancelled) = self.counts();
            format!("Export: {ok} saved, {skipped} skipped, {failed} failed, {cancelled} cancelled")
        };
        ui.horizontal(|ui| {
            if ui.small_button(label).clicked() {
                self.visible = true;
            }
            if self.phase != Phase::Running && ui.small_button("Dismiss").clicked() {
                self.notice = false;
            }
        });
        if self.phase == Phase::Running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
    fn counts(&self) -> (usize, usize, usize, usize) {
        let (mut ok, mut skipped, mut failed, mut cancelled) = (0, 0, 0, 0);
        for row in &self.rows {
            match row.outcome {
                Some(Outcome::Exported(_)) => ok += 1,
                Some(Outcome::Skipped(_)) => skipped += 1,
                Some(Outcome::Failed(_)) => failed += 1,
                Some(Outcome::Cancelled) => cancelled += 1,
                None => {}
            }
        }
        (ok, skipped, failed, cancelled)
    }
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.visible {
            return;
        }
        let response = egui::Modal::new(egui::Id::new("export-sounds")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 50.0).clamp(600.0, 900.0));
            // A modal remembers its previous size. Let newly revealed controls grow
            // the scroll viewport instead of trapping it at the old content height.
            ui.set_max_height((ctx.content_rect().height() - 70.0).max(200.0));
            egui::ScrollArea::vertical()
                .id_salt("export-dialog-scroll")
                .max_height((ctx.content_rect().height() - 70.0).max(200.0))
                .auto_shrink([false, true])
                .show(ui, |ui| self.contents(ui, ctx))
                .inner
        });
        if response.inner || response.should_close() {
            self.visible = false;
            if self.phase != Phase::Running {
                if let Some(worker) = &self.worker {
                    worker.cancel();
                }
                if matches!(self.phase, Phase::Reviewing | Phase::Ready | Phase::Loading) {
                    self.invalidate();
                }
            }
        }
    }
    fn contents(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
        ui.heading(format!("Export sounds ({})", self.rows.len()));
        ui.weak("Create copies with independent names and settings. Originals stay unchanged.");
        if self.phase == Phase::Loading {
            ui.spinner();
            ui.label("Loading the captured sound selection...");
        }
        let editable = matches!(self.phase, Phase::Editing | Phase::Ready | Phase::Finished);
        let mut changed = false;
        ui.add_enabled_ui(editable, |ui| {
            changed |= self.shared_settings(ui, ctx);
        });
        ui.separator();
        self.table(ui, ctx, editable);
        ui.add_enabled_ui(editable, |ui| {
            changed |= self.row_settings(ui);
        });
        if changed {
            self.invalidate();
        }
        self.actions(ui, ctx, editable)
    }
    fn shared_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Destination");
            let destination = if self.directory.as_os_str().is_empty() {
                "Choose a folder".into()
            } else {
                self.directory.display().to_string()
            };
            ui.add_sized(
                [ui.available_width() - 130.0, 20.0],
                egui::Label::new(&destination).truncate(),
            )
            .on_hover_text(destination);
            if ui.button("Choose folder...").clicked() {
                self.choose_folder(ctx);
            }
        });
        ui.label(RichText::new("Shared settings").strong());
        changed |= controls::settings(ui, &mut self.common, "export-common");
        ui.horizontal(|ui| {
            ui.label("Prefix");
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.prefix)
                        .char_limit(60)
                        .desired_width(120.0),
                )
                .changed();
            ui.label("Suffix");
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.suffix)
                        .char_limit(60)
                        .desired_width(120.0),
                )
                .changed();
            changed |= ui.checkbox(&mut self.numbering, "Number files").changed();
            egui::ComboBox::from_id_salt("export-conflicts")
                .selected_text(self.conflict.label())
                .show_ui(ui, |ui| {
                    for conflict in [Conflict::KeepBoth, Conflict::Skip, Conflict::Replace] {
                        changed |= ui
                            .selectable_value(&mut self.conflict, conflict, conflict.label())
                            .changed();
                    }
                });
        });
        changed
    }
    fn row_settings(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        if let Some(row) = self.rows.get_mut(self.selected) {
            ui.horizontal(|ui| {
                ui.label("Export name");
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut row.stem)
                            .char_limit(150)
                            .desired_width(300.0),
                    )
                    .changed();
                let mut custom = row.custom.is_some();
                if ui.checkbox(&mut custom, "Custom settings").changed() {
                    row.custom = custom.then(|| self.common.clone());
                    changed = true;
                }
                if row.custom.is_some() && ui.button("Use shared settings").clicked() {
                    row.custom = None;
                    changed = true;
                }
            });
            if let Some(settings) = &mut row.custom {
                changed |= controls::settings(ui, settings, ("export-custom", self.selected));
            } else {
                ui.weak("This sound follows shared settings. Changing its name does not change its source.");
            }
        }
        changed
    }
    fn actions(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, editable: bool) -> bool {
        let mut close = false;
        if let Some(error) = &self.error {
            ui.colored_label(Color32::from_rgb(245, 143, 140), error);
        }
        if self.phase == Phase::Ready
            && self
                .rows
                .iter()
                .any(|row| row.preview.as_ref().is_some_and(|p| p.replaces))
        {
            ui.checkbox(
                &mut self.replace_confirmed,
                "Replace the existing destination files listed above",
            );
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    editable && !self.rows.is_empty(),
                    egui::Button::new("Review export"),
                )
                .clicked()
            {
                self.review();
            }
            let replaces = self
                .rows
                .iter()
                .any(|row| row.preview.as_ref().is_some_and(|p| p.replaces));
            if ui
                .add_enabled(
                    self.phase == Phase::Ready && (!replaces || self.replace_confirmed),
                    egui::Button::new("Export files"),
                )
                .clicked()
            {
                self.start();
            }
            if matches!(self.phase, Phase::Running | Phase::Reviewing)
                && ui.button("Cancel export").clicked()
            {
                if self.phase == Phase::Reviewing {
                    self.invalidate();
                } else if let Some(worker) = &self.worker {
                    worker.cancel();
                }
            }
            if ui
                .button(if self.phase == Phase::Running {
                    "Hide"
                } else {
                    "Close"
                })
                .clicked()
            {
                close = true;
            }
            if self.phase == Phase::Finished && ui.button("Open destination").clicked() {
                self.open_folder(ctx);
            }
        });
        if self.phase == Phase::Reviewing {
            ui.weak("Checking sources, names, conflicts and encoder availability...");
        }
        if self.phase == Phase::Running {
            let value = self
                .worker
                .as_ref()
                .map_or(0, |worker| worker.progress.load(Ordering::Relaxed));
            let completed = self.rows.iter().filter(|row| row.outcome.is_some()).count();
            ui.add(
                egui::ProgressBar::new(value.min(1000) as f32 / 1000.0).text(format!(
                    "{} of {} files complete",
                    completed,
                    self.rows.len()
                )),
            );
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        if self.phase == Phase::Finished {
            let (ok, skip, failed, cancelled) = self.counts();
            ui.label(format!(
                "{ok} exported · {skip} skipped · {failed} failed · {cancelled} cancelled"
            ));
        }
        close
    }
    fn table(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, editable: bool) {
        ui.horizontal(|ui| {
            ui.add_sized([200.0, 18.0], egui::Label::new("Source"));
            ui.add_sized([300.0, 18.0], egui::Label::new("Export name"));
            ui.label("Settings / status");
        });
        egui::ScrollArea::vertical()
            .id_salt("export-files")
            .max_height((ctx.content_rect().height() * 0.23).clamp(56.0, 190.0))
            .auto_shrink([false, true])
            .show_rows(ui, 28.0, self.rows.len(), |ui, range| {
                for index in range {
                    let row = &self.rows[index];
                    let settings = row.custom.as_ref().unwrap_or(&self.common);
                    let extension = settings.format.extension().unwrap_or(&row.reference.format);
                    let published = match &row.outcome {
                        Some(Outcome::Exported(path) | Outcome::Skipped(path)) => Some(path),
                        _ => None,
                    };
                    let target =
                        published.or_else(|| row.preview.as_ref().map(|preview| &preview.target));
                    let name = target
                        .map(|path| {
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned()
                        })
                        .unwrap_or_else(|| format!("{}.{}", self.name(index), extension));
                    let status = match &row.outcome {
                        Some(Outcome::Exported(_)) => "Exported".into(),
                        Some(Outcome::Skipped(_)) => "Skipped".into(),
                        Some(Outcome::Cancelled) => "Cancelled".into(),
                        Some(Outcome::Failed(error)) => format!("Failed: {error}"),
                        None if self.phase == Phase::Running && self.running == Some(index) => {
                            "Exporting...".into()
                        }
                        None => match &row.preview {
                            Some(p) if p.skipped => "Will skip".into(),
                            Some(p) if p.replaces => "Will replace".into(),
                            _ => format!(
                                "{}{}",
                                settings.format.label(),
                                if row.custom.is_some() {
                                    " · Custom"
                                } else {
                                    ""
                                }
                            ),
                        },
                    };
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled_ui(editable, |ui| {
                                ui.add_sized(
                                    [200.0, 26.0],
                                    egui::Button::selectable(
                                        self.selected == index,
                                        &row.reference.name,
                                    )
                                    .truncate(),
                                )
                            })
                            .inner
                            .on_hover_text(&row.reference.display_path)
                            .clicked()
                        {
                            self.selected = index;
                        }
                        ui.add_sized([300.0, 26.0], egui::Label::new(name).truncate());
                        ui.add(egui::Label::new(&status).truncate())
                            .on_hover_text(status);
                    });
                }
            });
    }
}
fn reveal_folder(path: PathBuf) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let system = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| std::io::Error::other("Windows directory is unavailable"))?;
        std::process::Command::new(system.join("explorer.exe"))
            .arg(path)
            .creation_flags(0x0800_0000)
            .spawn()?;
        // Explorer owns the user-requested window after successful process launch.
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _path = path;
        Err(std::io::Error::other(
            "Opening destination folders is supported on Windows",
        ))
    }
}
impl Drop for Export {
    fn drop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel();
        }
        if let Some(thread) = self.folder_open.take() {
            match thread.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("Destination opener failed during shutdown: {error}"),
                Err(_) => eprintln!("Destination opener panicked during shutdown"),
            }
        }
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/export.rs"
    ));
}
