//! Explicit derived copies. Export settings never affect preview or source files.

mod files;
mod process;
pub mod worker;

use crate::collections::Reference;
pub use files::{Plan, PreviewRow, prepare};
pub use process::Runtime;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Original,
    Wav,
    Aiff,
    Flac,
    Mp3,
    Ogg,
}
impl Format {
    pub const ALL: [Self; 6] = [
        Self::Original,
        Self::Wav,
        Self::Aiff,
        Self::Flac,
        Self::Mp3,
        Self::Ogg,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Wav => "WAV",
            Self::Aiff => "AIFF",
            Self::Flac => "FLAC",
            Self::Mp3 => "MP3",
            Self::Ogg => "Ogg Vorbis",
        }
    }
    pub fn extension(self) -> Option<&'static str> {
        match self {
            Self::Original => None,
            Self::Wav => Some("wav"),
            Self::Aiff => Some("aiff"),
            Self::Flac => Some("flac"),
            Self::Mp3 => Some("mp3"),
            Self::Ogg => Some("ogg"),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channels {
    #[default]
    Original,
    Mono,
    Stereo,
}
impl Channels {
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Keep original",
            Self::Mono => "Mono",
            Self::Stereo => "Stereo",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Depth {
    Int16,
    #[default]
    Int24,
    Float32,
}
impl Depth {
    pub fn label(self) -> &'static str {
        match self {
            Self::Int16 => "16-bit",
            Self::Int24 => "24-bit",
            Self::Float32 => "32-bit float",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mp3Quality {
    Bitrate(u16),
    Vbr(u8),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub format: Format,
    pub channels: Channels,
    pub sample_rate: Option<u32>,
    pub depth: Depth,
    pub flac_compression: u8,
    pub mp3: Mp3Quality,
    pub vorbis_quality: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            format: Format::Original,
            channels: Channels::Original,
            sample_rate: None,
            depth: Depth::Int24,
            flac_compression: 5,
            mp3: Mp3Quality::Bitrate(320),
            vorbis_quality: 5,
        }
    }
}
pub const SAMPLE_RATES: [u32; 11] = [
    8000, 16000, 22050, 24000, 32000, 44100, 48000, 88200, 96000, 176400, 192000,
];
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self
            .sample_rate
            .is_some_and(|rate| !SAMPLE_RATES.contains(&rate))
            || self.flac_compression > 12
            || self.vorbis_quality > 10
            || matches!(self.mp3, Mp3Quality::Bitrate(rate) if ![64,96,128,160,192,224,256,320].contains(&rate))
            || matches!(self.mp3, Mp3Quality::Vbr(quality) if quality > 9)
        {
            return Err(Error::Invalid("Unsupported encoding setting".into()));
        }
        if self.format == Format::Flac && self.depth == Depth::Float32 {
            return Err(Error::Invalid(
                "FLAC requires integer samples; choose 16-bit or 24-bit".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Conflict {
    #[default]
    KeepBoth,
    Skip,
    Replace,
}
impl Conflict {
    pub fn label(self) -> &'static str {
        match self {
            Self::KeepBoth => "Keep both",
            Self::Skip => "Skip existing",
            Self::Replace => "Replace existing",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Item {
    pub reference: Reference,
    pub stem: String,
    pub settings: Settings,
}
#[derive(Clone, Debug)]
pub struct Request {
    pub directory: PathBuf,
    pub conflict: Conflict,
    pub items: Vec<Item>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Exported(PathBuf),
    Skipped(PathBuf),
    Cancelled,
    Failed(String),
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Export {stage} failed: {cause}")]
    Stage {
        stage: &'static str,
        #[source]
        cause: Box<Error>,
    },
    #[error("Export file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("Export cancelled")]
    Cancelled,
    #[error("FFmpeg export failed: {0}")]
    Encoder(String),
}
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    fn at(self, stage: &'static str) -> Self {
        if matches!(self, Self::Cancelled) {
            self
        } else {
            Self::Stage {
                stage,
                cause: Box::new(self),
            }
        }
    }
}

pub fn validate_stem(name: &str) -> Result<()> {
    let reserved = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.is_empty()
        || name.len() > 180
        || name.ends_with([' ', '.'])
        || name.starts_with(' ')
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&reserved.as_str())
        || ["COM", "LPT"].iter().any(|prefix| {
            reserved.strip_prefix(prefix).is_some_and(|suffix| {
                ["1", "2", "3", "4", "5", "6", "7", "8", "9", "¹", "²", "³"].contains(&suffix)
            })
        })
    {
        return Err(Error::Invalid("Use a file name without reserved names, path separators or trailing spaces/dots (at most 180 UTF-8 bytes)".into()));
    }
    Ok(())
}

pub fn default_stem(reference: &Reference) -> String {
    let stem = std::path::Path::new(&reference.name)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut cleaned = cleaned.trim_matches([' ', '.']).to_owned();
    while cleaned.len() > 170 {
        cleaned.pop();
    }
    if validate_stem(&cleaned).is_err() {
        cleaned = "Sound".into();
    }
    cleaned
}
