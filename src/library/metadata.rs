use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use symphonia::core::formats::{TrackType, probe::Hint};
use symphonia::core::io::{MediaSource, MediaSourceStream};

use super::{AudioMetadata, Cancellation};

type ProbeResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn read(path: &Path, cancellation: Cancellation) -> ProbeResult<AudioMetadata> {
    let file = File::open(path)?;
    let before = file.metadata()?;
    let verification_handle = file.try_clone()?;
    let length = before.len();
    let source = ProbeSource {
        file,
        length,
        remaining: 8 * 1024 * 1024,
        cancellation,
    };
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }
    let format = symphonia::default::get_probe().probe(
        &hint,
        stream,
        Default::default(),
        Default::default(),
    )?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| io::Error::other("No audio track found"))?;
    let audio = track
        .codec_params
        .as_ref()
        .and_then(|parameters| parameters.audio())
        .ok_or_else(|| io::Error::other("Audio parameters are unavailable"))?;
    let duration_seconds = track
        .time_base
        .zip(track.duration)
        .and_then(|(base, duration)| base.calc_duration(duration))
        .map(|time| time.as_secs_f64())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
    let after = verification_handle.metadata()?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(io::Error::other("Source changed during metadata probing; rescan it").into());
    }
    Ok(AudioMetadata {
        duration_seconds,
        sample_rate: audio.sample_rate,
        channels: audio
            .channels
            .as_ref()
            .and_then(|channels| u32::try_from(channels.count()).ok()),
        bit_depth: audio.bits_per_sample,
    })
}

// Bound parser input and make cancellation visible between OS reads. A blocked
// network filesystem read cannot be interrupted by this cooperative token.
struct ProbeSource {
    file: File,
    length: u64,
    remaining: usize,
    cancellation: Cancellation,
}

impl Read for ProbeSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.cancellation.is_cancelled() {
            return Err(io::Error::other("Scan cancelled"));
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            return Err(io::Error::other(
                "Metadata probe exceeds its 8 MiB read budget",
            ));
        }
        let limit = buffer.len().min(self.remaining);
        let read = self.file.read(&mut buffer[..limit])?;
        self.remaining -= read;
        Ok(read)
    }
}

impl Seek for ProbeSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

impl MediaSource for ProbeSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.length)
    }
}
