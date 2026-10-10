mod metadata;

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::catalog::{Catalog, IndexedFile};
use crate::{Error, Result};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioMetadata {
    pub duration_seconds: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub bit_depth: Option<u32>,
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
}

#[derive(Clone, Debug, Default)]
pub struct ScanReport {
    pub indexed: usize,
    pub errors: usize,
    pub skipped_links: usize,
    pub cancelled: bool,
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
}

pub fn scan(
    root: &Path,
    catalog: &mut Catalog,
    cancellation: &Cancellation,
    mut progress: impl FnMut(&ScanReport),
) -> Result<ScanReport> {
    let metadata = std::fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::InvalidRoot);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Error::InvalidRoot);
        }
    }
    let root = root.canonicalize()?;
    if !root.is_dir() {
        return Err(Error::InvalidRoot);
    }
    if cancellation.is_cancelled() {
        return Ok(ScanReport {
            cancelled: true,
            ..Default::default()
        });
    }
    let root_id = catalog.register_root(&root)?;
    let mut report = ScanReport::default();
    let mut batch = Vec::with_capacity(64);
    visit_directory(
        &root,
        &root,
        0,
        cancellation,
        &mut report,
        &mut |file, report| {
            batch.push(file);
            if batch.len() == 64 {
                catalog.upsert_batch(root_id, &batch)?;
                report.indexed += batch.len();
                batch.clear();
                progress(report);
            }
            Ok(())
        },
    )?;
    if !batch.is_empty() {
        catalog.upsert_batch(root_id, &batch)?;
        report.indexed += batch.len();
    }
    report.cancelled = cancellation.is_cancelled();
    progress(&report);
    Ok(report)
}

fn visit_directory(
    root: &Path,
    directory: &Path,
    depth: usize,
    cancellation: &Cancellation,
    report: &mut ScanReport,
    accept: &mut impl FnMut(IndexedFile, &mut ScanReport) -> Result<()>,
) -> Result<()> {
    if depth >= 64 {
        report.issue(
            directory.to_path_buf(),
            "Directory nesting exceeds the 64-level scan limit",
        );
        return Ok(());
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
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
                report.issue(directory.to_path_buf(), error);
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) => {
                report.issue(path, error);
                continue;
            }
        };
        if file_type.is_symlink() {
            report.skipped_links += 1;
            continue;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            // Junctions and other reparse points are not followed into other trees.
            match entry.metadata() {
                Ok(info) if info.file_attributes() & 0x400 != 0 => {
                    report.skipped_links += 1;
                    continue;
                }
                Err(error) => {
                    report.issue(path, error);
                    continue;
                }
                _ => {}
            }
        }
        if file_type.is_dir() {
            visit_directory(root, &path, depth + 1, cancellation, report, accept)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(format) = supported_format(&path) else {
            continue;
        };
        let file_size = match entry.metadata() {
            Ok(info) => info.len(),
            Err(error) => {
                report.issue(path, error);
                continue;
            }
        };
        let (audio, metadata_error) = match metadata::read(&path, cancellation.clone()) {
            Ok(audio) => (audio, None),
            Err(_error) if cancellation.is_cancelled() => {
                break;
            }
            Err(error) => {
                report.issue(path.clone(), &error);
                (AudioMetadata::default(), Some(error.to_string()))
            }
        };
        let display_path = path
            .strip_prefix(root)
            .map_err(|_| Error::InvalidPath)?
            .to_string_lossy()
            .into_owned();
        let name = entry.file_name().to_string_lossy().into_owned();
        accept(
            IndexedFile {
                source: path,
                name,
                display_path,
                format,
                size_bytes: file_size,
                audio,
                metadata_error,
            },
            report,
        )?;
    }
    Ok(())
}

fn supported_format(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "wav" | "flac" | "mp3" | "aiff" | "aif" => Some(extension),
        _ => None,
    }
}
