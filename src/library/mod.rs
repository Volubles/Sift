mod metadata;

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::catalog::{Catalog, IndexedFile};
use crate::file_state::{is_link, timestamp};
use crate::{Error, Result};

pub(crate) const PROBE_VERSION: i64 = 1;
const BATCH_SIZE: usize = 64;

#[derive(Clone, Copy)]
enum ProbePolicy {
    ChangedFiles,
    AllFiles,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioMetadata {
    pub duration_seconds: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub bit_depth: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub size_bytes: u64,
    pub modified_ns: Option<i64>,
    pub created_ns: Option<i64>,
}

impl FileStamp {
    pub(crate) fn read(metadata: &std::fs::Metadata) -> Self {
        Self {
            size_bytes: metadata.len(),
            modified_ns: metadata.modified().ok().and_then(timestamp),
            created_ns: metadata.created().ok().and_then(timestamp),
        }
    }
}

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    pub(crate) fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScanReport {
    pub visited: usize,
    pub indexed: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub deferred: usize,
    pub errors: usize,
    pub skipped_links: usize,
    pub cancelled: bool,
    pub incomplete: bool,
    pub issues: Vec<ScanIssue>,
}

#[derive(Clone, Debug)]
pub struct ScanIssue {
    pub path: PathBuf,
    pub message: String,
}

impl ScanReport {
    fn issue(&mut self, path: PathBuf, error: impl std::fmt::Display) {
        self.errors += 1;
        if self.issues.len() < 16 {
            self.issues.push(ScanIssue {
                path,
                message: error.to_string(),
            });
        }
    }
    pub fn changed(&self) -> bool {
        self.indexed > 0 || self.removed > 0
    }
}

pub(crate) fn ordinary_directory(path: &Path) -> Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(Error::InvalidRoot);
    }
    for ancestor in path
        .ancestors()
        .skip(1)
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
    {
        if is_link(&std::fs::symlink_metadata(ancestor)?) {
            return Err(Error::InvalidRoot);
        }
    }
    Ok(path.canonicalize()?)
}

pub fn scan(
    root: &Path,
    catalog: &mut Catalog,
    cancellation: &Cancellation,
    progress: impl FnMut(&ScanReport),
) -> Result<ScanReport> {
    let root = ordinary_directory(root)?;
    scan_tree(
        &root,
        &root,
        catalog,
        cancellation,
        ProbePolicy::ChangedFiles,
        progress,
    )
}

/// A missed notification may hide a timestamp-preserving edit. Recheck metadata
/// once to recover completeness; ordinary updates still use the cheap signature.
pub fn scan_after_event_loss(
    root: &Path,
    catalog: &mut Catalog,
    cancellation: &Cancellation,
    progress: impl FnMut(&ScanReport),
) -> Result<ScanReport> {
    let root = ordinary_directory(root)?;
    scan_tree(
        &root,
        &root,
        catalog,
        cancellation,
        ProbePolicy::AllFiles,
        progress,
    )
}

fn scan_tree(
    root: &Path,
    subtree: &Path,
    catalog: &mut Catalog,
    cancellation: &Cancellation,
    policy: ProbePolicy,
    mut progress: impl FnMut(&ScanReport),
) -> Result<ScanReport> {
    let mut report = ScanReport::default();
    if cancellation.is_cancelled() {
        report.cancelled = true;
        return Ok(report);
    }
    let root_id = catalog.register_root(root)?;
    let initial_root = FileStamp::read(&std::fs::symlink_metadata(root)?);
    catalog.begin_inventory()?;
    let mut seen = Vec::with_capacity(BATCH_SIZE);
    let mut changed = Vec::with_capacity(BATCH_SIZE);
    visit_directory(
        subtree,
        0,
        cancellation,
        &mut report,
        &mut |candidate, report| {
            report.visited += 1;
            seen.push(candidate.source.clone());
            if matches!(policy, ProbePolicy::ChangedFiles)
                && catalog.unchanged(&candidate.source, &candidate.stamp)?
            {
                report.unchanged += 1;
            } else if let Some(file) = probe(root, candidate, cancellation, report)? {
                changed.push(file);
            }
            if seen.len() == BATCH_SIZE {
                report.indexed += catalog.upsert_batch(root_id, &changed)?;
                catalog.record_seen(&seen)?;
                seen.clear();
                changed.clear();
                progress(report);
            }
            Ok(())
        },
    )?;
    report.indexed += catalog.upsert_batch(root_id, &changed)?;
    catalog.record_seen(&seen)?;
    report.cancelled = cancellation.is_cancelled();
    if !report.cancelled && !report.incomplete {
        // An offline/replaced root must never be interpreted as deleted audio.
        let final_root = ordinary_directory(root)?;
        let final_stamp = FileStamp::read(&std::fs::symlink_metadata(root)?);
        if final_root != root || initial_root.created_ns != final_stamp.created_ns {
            report.incomplete = true;
            report.issue(
                root.to_path_buf(),
                "The source root changed during enumeration; cached files were retained",
            );
        } else {
            report.removed = catalog.finish_inventory(subtree)?;
        }
    }
    progress(&report);
    Ok(report)
}

struct Candidate {
    source: PathBuf,
    format: String,
    stamp: FileStamp,
}

fn probe(
    root: &Path,
    candidate: Candidate,
    cancellation: &Cancellation,
    report: &mut ScanReport,
) -> Result<Option<IndexedFile>> {
    let (audio, metadata_error) = match metadata::read(&candidate.source, cancellation.clone()) {
        Ok(audio) => (audio, None),
        Err(_) if cancellation.is_cancelled() => return Ok(None),
        Err(error) => {
            report.issue(candidate.source.clone(), &error);
            (AudioMetadata::default(), Some(error.to_string()))
        }
    };
    let current = match std::fs::symlink_metadata(&candidate.source) {
        Ok(metadata) if metadata.is_file() && !is_link(&metadata) => FileStamp::read(&metadata),
        Ok(_) => {
            report.deferred += 1;
            return Ok(None);
        }
        Err(error) => {
            report.deferred += 1;
            report.issue(candidate.source, error);
            return Ok(None);
        }
    };
    if current != candidate.stamp {
        report.deferred += 1;
        return Ok(None);
    }
    let display_path = candidate
        .source
        .strip_prefix(root)
        .map_err(|_| Error::InvalidPath)?
        .to_string_lossy()
        .into_owned();
    let name = candidate
        .source
        .file_name()
        .ok_or(Error::InvalidPath)?
        .to_string_lossy()
        .into_owned();
    Ok(Some(IndexedFile {
        source: candidate.source,
        name,
        display_path,
        format: candidate.format,
        size_bytes: candidate.stamp.size_bytes,
        audio,
        metadata_error,
        stamp: Some(candidate.stamp),
    }))
}

fn visit_directory(
    directory: &Path,
    depth: usize,
    cancellation: &Cancellation,
    report: &mut ScanReport,
    accept: &mut impl FnMut(Candidate, &mut ScanReport) -> Result<()>,
) -> Result<()> {
    if depth >= 64 {
        report.incomplete = true;
        report.issue(
            directory.to_path_buf(),
            "Directory nesting exceeds the 64-level scan limit",
        );
        return Ok(());
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            report.incomplete = true;
            report.issue(directory.to_path_buf(), error);
            return Ok(());
        }
    };
    for entry in entries {
        if cancellation.is_cancelled() {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.incomplete = true;
                report.issue(directory.to_path_buf(), error);
                continue;
            }
        };
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                report.incomplete = true;
                report.issue(path, error);
                continue;
            }
        };
        if is_link(&metadata) {
            report.skipped_links += 1;
            continue;
        }
        if metadata.is_dir() {
            visit_directory(&path, depth + 1, cancellation, report, accept)?;
        } else if metadata.is_file()
            && let Some(format) = supported_format(&path)
        {
            accept(
                Candidate {
                    source: path,
                    format,
                    stamp: FileStamp::read(&metadata),
                },
                report,
            )?;
        }
    }
    Ok(())
}

/// Reconcile only the named path or subtree. Watch events are hints: inspect the
/// current filesystem, validate ancestors, and retain cache on ambiguous failure.
pub fn synchronize_path(
    root: &Path,
    path: &Path,
    catalog: &mut Catalog,
    cancellation: &Cancellation,
) -> Result<ScanReport> {
    let registered_root = root;
    let root = ordinary_directory(root)?;
    let relative = path
        .strip_prefix(registered_root)
        .or_else(|_| path.strip_prefix(&root))
        .map_err(|_| Error::InvalidPath)?;
    let path = root.join(relative);
    let path = path.as_path();
    if !path.is_absolute()
        || !path.starts_with(&root)
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(Error::InvalidPath);
    }
    for ancestor in path
        .ancestors()
        .skip(1)
        .take_while(|ancestor| ancestor.starts_with(&root))
    {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if is_link(&metadata) => return Err(Error::InvalidRoot),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut report = ScanReport::default();
    if cancellation.is_cancelled() {
        report.cancelled = true;
        return Ok(report);
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if is_link(&metadata) => {
            report.skipped_links = 1;
            report.removed = catalog.remove_cached_path(&root, path)?;
        }
        Ok(metadata) if metadata.is_dir() => {
            let canonical = ordinary_directory(path)?;
            if !canonical.starts_with(&root) {
                return Err(Error::InvalidPath);
            }
            let removed = if canonical != path {
                catalog.remove_cached_path(&root, path)?
            } else {
                0
            };
            let mut result = scan_tree(
                &root,
                &canonical,
                catalog,
                cancellation,
                ProbePolicy::ChangedFiles,
                |_| {},
            )?;
            result.removed += removed;
            return Ok(result);
        }
        Ok(metadata) if metadata.is_file() => {
            if let Some(format) = supported_format(path) {
                report.visited = 1;
                let canonical = path.canonicalize()?;
                if !canonical.starts_with(&root) {
                    return Err(Error::InvalidPath);
                }
                if canonical != path {
                    report.removed = catalog.remove_cached_path(&root, path)?;
                }
                // A write event forces probing even if a tool preserved the stamp.
                if let Some(file) = probe(
                    &root,
                    Candidate {
                        source: canonical,
                        format,
                        stamp: FileStamp::read(&metadata),
                    },
                    cancellation,
                    &mut report,
                )? {
                    let id = catalog.register_root(&root)?;
                    report.indexed = catalog.upsert_batch(id, &[file])?;
                }
            } else {
                report.removed = catalog.remove_cached_path(&root, path)?;
            }
        }
        Ok(_) => {
            report.removed = catalog.remove_cached_path(&root, path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            report.removed = catalog.remove_cached_path(&root, path)?;
        }
        Err(error) => return Err(error.into()),
    }
    report.cancelled = cancellation.is_cancelled();
    Ok(report)
}

fn supported_format(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "wav" | "flac" | "mp3" | "aiff" | "aif" => Some(extension),
        _ => None,
    }
}
