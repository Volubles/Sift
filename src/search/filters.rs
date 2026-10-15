use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Wav,
    Flac,
    Mp3,
    Aiff,
}

impl Format {
    pub const ALL: [Self; 4] = [Self::Wav, Self::Flac, Self::Mp3, Self::Aiff];

    pub fn label(self) -> &'static str {
        match self {
            Self::Wav => "WAV",
            Self::Flac => "FLAC",
            Self::Mp3 => "MP3",
            Self::Aiff => "AIFF",
        }
    }

    pub(crate) fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Wav => &["wav"],
            Self::Flac => &["flac"],
            Self::Mp3 => &["mp3"],
            Self::Aiff => &["aif", "aiff"],
        }
    }
}

/// A fixed set of supported format choices. An empty set leaves format unrestricted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Formats(u8);

impl Formats {
    pub fn contains(self, format: Format) -> bool {
        self.0 & (1 << format as u8) != 0
    }
    pub fn set(&mut self, format: Format, included: bool) {
        let bit = 1 << format as u8;
        if included {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnknownLengths {
    #[default]
    Exclude,
    Include,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Length {
    #[default]
    Any,
    Range {
        min: Option<f64>,
        max: Option<f64>,
        unknown: UnknownLengths,
    },
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channels {
    #[default]
    Any,
    Mono,
    Stereo,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SampleRate {
    #[default]
    Any,
    Hz(u32),
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MetadataFilters {
    pub length: Length,
    pub formats: Formats,
    pub channels: Channels,
    pub sample_rate: SampleRate,
}

impl MetadataFilters {
    pub fn validate(self) -> Result<()> {
        if let Length::Range { min, max, .. } = self.length {
            if min.is_none() && max.is_none() {
                return Err(Error::InvalidQuery(
                    "Set a length bound or choose Any length".into(),
                ));
            }
            if min
                .into_iter()
                .chain(max)
                .any(|value| !value.is_finite() || value < 0.0)
            {
                return Err(Error::InvalidQuery(
                    "Length bounds must be finite, nonnegative seconds".into(),
                ));
            }
            if let (Some(min), Some(max)) = (min, max)
                && min > max
            {
                return Err(Error::InvalidQuery(
                    "Minimum length must not exceed maximum length".into(),
                ));
            }
        }
        if self.sample_rate == SampleRate::Hz(0) {
            return Err(Error::InvalidQuery(
                "Sample rate must be positive or unknown".into(),
            ));
        }
        Ok(())
    }

    pub fn active_count(self) -> usize {
        usize::from(self.length != Length::Any)
            + usize::from(!self.formats.is_empty())
            + usize::from(self.channels != Channels::Any)
            + usize::from(self.sample_rate != SampleRate::Any)
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/search/filters.rs"
    ));
}
