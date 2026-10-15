use super::{Conflict, Error, Format, Item, Outcome, Request, Result, Runtime};
use crate::library::{AudioMetadata, Cancellation, FileStamp};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tempfile::NamedTempFile;

#[derive(Clone, Debug)]
pub struct PreviewRow {
    pub target: PathBuf,
    pub skipped: bool,
    pub replaces: bool,
}
struct Job {
    item: Item,
    source: File,
    identity: same_file::Handle,
    stamp: FileStamp,
    audio: AudioMetadata,
    preview: PreviewRow,
}
pub struct Plan {
    directory: PathBuf,
    _directory_guards: Vec<File>,
    jobs: Vec<Job>,
    conflict: Conflict,
}

fn cancelled(token: &Cancellation) -> Result<()> {
    if token.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
fn exists(path: &Path) -> Result<bool> {
    match path.symlink_metadata() {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
fn source_file(path: &Path) -> Result<File> {
    if !path.is_absolute() {
        return Err(Error::Invalid("Source paths must be absolute".into()));
    }
    let before = path.symlink_metadata()?;
    if !before.is_file() || crate::file_state::is_link(&before) {
        return Err(Error::Invalid(
            "Choose ordinary source files without links or reparse points".into(),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0020_0000);
    }
    let file = options.open(path)?;
    if crate::file_state::is_link(&file.metadata()?) {
        return Err(Error::Invalid("The source became a link".into()));
    }
    Ok(file)
}
fn directory_guards(path: &Path) -> Result<Vec<File>> {
    if !path.is_absolute() {
        return Err(Error::Invalid(
            "Choose an existing absolute destination folder".into(),
        ));
    }
    let ancestors: Vec<_> = path
        .ancestors()
        .filter(|path| !path.as_os_str().is_empty())
        .collect();
    if ancestors.len() > 128 {
        return Err(Error::Invalid(
            "Destination exceeds the 128-directory export limit".into(),
        ));
    }
    let mut guards = Vec::with_capacity(ancestors.len());
    for path in ancestors.iter().rev() {
        let meta = path.symlink_metadata()?;
        if !meta.is_dir() || crate::file_state::is_link(&meta) {
            return Err(Error::Invalid(
                "Destination folders cannot contain links or reparse points".into(),
            ));
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options
                .share_mode(3)
                .custom_flags(0x0200_0000 | 0x0020_0000);
        }
        let file = options.open(path)?;
        if crate::file_state::is_link(&file.metadata()?) {
            return Err(Error::Invalid("Destination changed into a link".into()));
        }
        guards.push(file);
    }
    Ok(guards)
}
fn key(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}
fn alternative(path: &Path, number: usize) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    path.with_file_name(format!("{stem} ({number}){ext}"))
}

pub fn prepare(request: Request, token: &Cancellation) -> Result<Plan> {
    if request.items.is_empty() || request.items.len() > crate::search::MAX_ACTION_SOUNDS {
        return Err(Error::Invalid(
            "Export between 1 and 128 sounds at a time".into(),
        ));
    }
    let guards = directory_guards(&request.directory)?;
    let directory = request.directory.canonicalize()?;
    let mut source_keys = BTreeSet::new();
    let mut reserved = BTreeSet::new();
    let mut jobs = Vec::with_capacity(request.items.len());
    let mut bytes = 0usize;
    for item in request.items {
        cancelled(token)?;
        item.settings.validate()?;
        super::validate_stem(&item.stem)?;
        let source = source_file(&item.reference.source)?;
        bytes = bytes.saturating_add(crate::path_codec::encode(&item.reference.source).len());
        if bytes > 1024 * 1024
            || !source_keys.insert(crate::path_codec::encode(&item.reference.source))
        {
            return Err(Error::Invalid(
                "Export sources exceed the path budget or contain duplicates".into(),
            ));
        }
        let identity = same_file::Handle::from_file(source.try_clone()?)?;
        let stamp = FileStamp::read(&source.metadata()?);
        let extension = item
            .settings
            .format
            .extension()
            .map(str::to_owned)
            .or_else(|| {
                item.reference
                    .source
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        if extension.len() > 16 || !extension.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(Error::Invalid(
                "The source extension cannot be used for export".into(),
            ));
        }
        let name = if extension.is_empty() {
            item.stem.clone()
        } else {
            format!("{}.{extension}", item.stem)
        };
        let requested = directory.join(name);
        let mut target = requested.clone();
        let duplicate = reserved.contains(&key(&target));
        let mut present = exists(&target)?;
        if request.conflict == Conflict::Replace && duplicate {
            return Err(Error::Invalid(
                "Two sounds have the same export name; give them distinct names".into(),
            ));
        }
        if request.conflict == Conflict::KeepBoth && (present || duplicate) {
            let mut chosen = false;
            for n in 1..=10000 {
                let candidate = alternative(&requested, n);
                if !reserved.contains(&key(&candidate)) && !exists(&candidate)? {
                    target = candidate;
                    present = false;
                    chosen = true;
                    break;
                }
            }
            if !chosen {
                return Err(Error::Invalid("Could not find a free export name".into()));
            }
        }
        reserved.insert(key(&target));
        let skipped = request.conflict == Conflict::Skip && (present || duplicate);
        let audio = if !skipped && item.settings.format != Format::Original {
            crate::library::metadata::read(&item.reference.source, token.clone())
                .map_err(|e| Error::Invalid(format!("Source metadata could not be read: {e}")))?
        } else {
            item.reference.audio.clone()
        };
        if !skipped && item.settings.format != Format::Original {
            super::process::validate_audio(&item.settings, &audio)?;
        }
        jobs.push(Job {
            item,
            source,
            identity,
            stamp,
            audio,
            preview: PreviewRow {
                target,
                skipped,
                replaces: present && request.conflict == Conflict::Replace,
            },
        });
    }
    let plan = Plan {
        directory,
        _directory_guards: guards,
        jobs,
        conflict: request.conflict,
    };
    for job in &plan.jobs {
        if job.preview.replaces {
            plan.check_replacement(&job.preview.target)?;
        }
    }
    Ok(plan)
}

impl Plan {
    pub fn rows(&self) -> Vec<PreviewRow> {
        self.jobs.iter().map(|job| job.preview.clone()).collect()
    }
    pub fn needs_encoder(&self) -> bool {
        self.jobs
            .iter()
            .any(|job| !job.preview.skipped && job.item.settings.format != Format::Original)
    }
    fn check_replacement(&self, path: &Path) -> Result<()> {
        if !exists(path)? {
            return Ok(());
        }
        let meta = path.symlink_metadata()?;
        if !meta.is_file() || crate::file_state::is_link(&meta) {
            return Err(Error::Invalid(
                "Replacement target must be an ordinary file".into(),
            ));
        }
        let target = same_file::Handle::from_path(path)?;
        if self.jobs.iter().any(|job| job.identity == target) {
            return Err(Error::Invalid(
                "Export cannot replace any selected source file or its hard links".into(),
            ));
        }
        Ok(())
    }
    pub fn execute(
        &self,
        runtime: Option<&Runtime>,
        token: &Cancellation,
        progress: &AtomicU64,
        mut report: impl FnMut(usize, Option<&Outcome>),
    ) -> Vec<Outcome> {
        let mut outcomes = Vec::with_capacity(self.jobs.len());
        let mut published = Vec::new();
        for (index, job) in self.jobs.iter().enumerate() {
            progress.store(0, Ordering::Relaxed);
            report(index, None);
            let outcome = if token.is_cancelled() {
                Outcome::Cancelled
            } else if job.preview.skipped {
                Outcome::Skipped(job.preview.target.clone())
            } else {
                match self.export_one(job, runtime, token, progress, &published) {
                    Ok(outcome) => outcome,
                    Err(Error::Cancelled) => Outcome::Cancelled,
                    Err(error) => {
                        eprintln!("Export item {index} failed: {error}");
                        Outcome::Failed(error.to_string())
                    }
                }
            };
            if let Outcome::Exported(path) = &outcome {
                published.push(path.clone());
                progress.store(1000, Ordering::Relaxed);
            }
            report(index, Some(&outcome));
            outcomes.push(outcome);
        }
        outcomes
    }
    fn export_one(
        &self,
        job: &Job,
        runtime: Option<&Runtime>,
        token: &Cancellation,
        progress: &AtomicU64,
        published: &[PathBuf],
    ) -> Result<Outcome> {
        self.verify_source(job)
            .map_err(|error| error.at("source verification"))?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".sift-export-")
            .suffix(".tmp")
            .tempfile_in(&self.directory)
            .map_err(|error| Error::Io(error).at("temporary creation"))?;
        let result = (|| {
            if job.item.settings.format == Format::Original {
                let mut source = job.source.try_clone()?;
                source.seek(SeekFrom::Start(0))?;
                let mut buffer = [0u8; 64 * 1024];
                let mut copied = 0u64;
                loop {
                    cancelled(token)?;
                    let count = source.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    temporary.write_all(&buffer[..count])?;
                    copied += count as u64;
                    progress.store(
                        copied.saturating_mul(999) / job.stamp.size_bytes.max(1),
                        Ordering::Relaxed,
                    );
                }
            } else {
                runtime
                    .ok_or_else(|| {
                        Error::Invalid(
                            "Export tools are unavailable; install the bundled FFmpeg tools".into(),
                        )
                    })?
                    .encode(
                        &job.item.reference.source,
                        temporary.path(),
                        &job.item.settings,
                        &job.audio,
                        token,
                        progress,
                    )?;
                let output = crate::library::metadata::read(temporary.path(), token.clone())
                    .map_err(|e| Error::Encoder(format!("Output verification failed: {e}")))?;
                let expected_rate = job.item.settings.sample_rate.or(job.audio.sample_rate);
                let expected_channels = match job.item.settings.channels {
                    super::Channels::Original => job.audio.channels,
                    super::Channels::Mono => Some(1),
                    super::Channels::Stereo => Some(2),
                };
                if output.sample_rate != expected_rate || output.channels != expected_channels {
                    return Err(Error::Encoder(
                        "Output rate/channels differ from the reviewed settings".into(),
                    ));
                }
            }
            cancelled(token)?;
            self.verify_source(job)?;
            temporary.as_file().sync_all()?;
            if self.directory.canonicalize()? != self.directory {
                return Err(Error::Invalid("Destination changed during export".into()));
            }
            Ok(())
        })();
        if let Err(error) = result {
            return close_with_error(temporary, error.at("render/verification"));
        }
        let original = job.preview.target.clone();
        let mut target = original.clone();
        let mut next_number = 1;
        for _ in 0..=10000 {
            if self.conflict == Conflict::Replace {
                if let Err(error) = self.check_replacement(&target) {
                    return close_with_error(temporary, error);
                }
                let repeated = (|| -> Result<bool> {
                    if !exists(&target)? {
                        return Ok(false);
                    }
                    let candidate = same_file::Handle::from_path(&target)?;
                    for path in published {
                        if same_file::Handle::from_path(path)? == candidate {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                })();
                match repeated {
                    Ok(false) => {}
                    Ok(true) => {
                        return close_with_error(
                            temporary,
                            Error::Invalid(
                                "This target is already used by another exported sound".into(),
                            ),
                        );
                    }
                    Err(error) => return close_with_error(temporary, error),
                }
                return match temporary.persist(&target) {
                    Ok(_) => Ok(Outcome::Exported(target)),
                    Err(error) => close_with_error(
                        error.file,
                        Error::Io(error.error).at("replacement publication"),
                    ),
                };
            }
            match temporary.persist_noclobber(&target) {
                Ok(_) => return Ok(Outcome::Exported(target)),
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    temporary = error.file;
                    if self.conflict == Conflict::Skip {
                        temporary.close()?;
                        return Ok(Outcome::Skipped(target));
                    }
                    loop {
                        target = alternative(&original, next_number);
                        next_number += 1;
                        if !self
                            .jobs
                            .iter()
                            .any(|other| key(&other.preview.target) == key(&target))
                        {
                            break;
                        }
                    }
                }
                Err(error) => {
                    return close_with_error(error.file, Error::Io(error.error).at("publication"));
                }
            }
        }
        close_with_error(
            temporary,
            Error::Invalid("Too many export-name conflicts".into()),
        )
    }
    fn verify_source(&self, job: &Job) -> Result<()> {
        if FileStamp::read(&job.source.metadata()?) != job.stamp
            || same_file::Handle::from_path(&job.item.reference.source)? != job.identity
        {
            return Err(Error::Invalid(
                "The source changed after export review; review the selection again".into(),
            ));
        }
        Ok(())
    }
}
fn close_with_error<T>(file: NamedTempFile, error: Error) -> Result<T> {
    match file.close() {
        Ok(()) => Err(error),
        Err(cleanup) => Err(Error::Invalid(format!(
            "{error}; temporary file cleanup failed: {cleanup}"
        ))),
    }
}
