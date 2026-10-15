use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use symphonia::core::{
    audio::GenericAudioBufferRef,
    codecs::audio::AudioDecoder,
    common::Limit,
    formats::{FormatReader, SeekMode, SeekTo, TrackType, probe::Hint},
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    units::{Time, TimeBase},
};

use super::{AudioError, AudioResult, Stereo};
use crate::library::Cancellation;

pub struct Decoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
    time_base: Option<TimeBase>,
    pub rate: u32,
    pub channels: usize,
    pub duration: Option<f64>,
    cancellation: Cancellation,
    interleaved: Vec<f32>,
    discard_frames: usize,
}

impl Decoder {
    pub fn open(path: &Path, cancellation: Cancellation) -> AudioResult<Self> {
        if cancellation.is_cancelled() {
            return Err(AudioError::Cancelled);
        }
        let file = File::open(path)?;
        let length = file.metadata()?.len();
        let probing = Arc::new(AtomicBool::new(true));
        let source = Source {
            file,
            length,
            remaining_probe: 8 * 1024 * 1024,
            probing: probing.clone(),
            cancellation: cancellation.clone(),
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
            MetadataOptions::default()
                .limit_tag_bytes(Limit::Maximum(4096))
                .limit_visual_bytes(Limit::Maximum(0)),
        )?;
        probing.store(false, Ordering::Relaxed);
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| AudioError::Invalid("No audio track is available".into()))?;
        let parameters = track
            .codec_params
            .as_ref()
            .and_then(|value| value.audio())
            .ok_or_else(|| AudioError::Invalid("Audio parameters are missing".into()))?;
        let rate = parameters
            .sample_rate
            .filter(|rate| *rate > 0 && *rate <= 384_000)
            .ok_or_else(|| {
                AudioError::Invalid(
                    "Preview requires a known sample rate at or below 384 kHz".into(),
                )
            })?;
        let channels = parameters
            .channels
            .as_ref()
            .map(|value| value.count())
            .filter(|count| (1..=8).contains(count))
            .ok_or_else(|| AudioError::Invalid("Preview supports 1-8 source channels".into()))?;
        let decoder =
            symphonia::default::get_codecs().make_audio_decoder(parameters, &Default::default())?;
        let duration = track
            .time_base
            .zip(track.duration)
            .and_then(|(base, length)| base.calc_duration(length))
            .map(|time| time.as_secs_f64())
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
        let track_id = track.id;
        let time_base = track.time_base;
        Ok(Self {
            format,
            decoder,
            track: track_id,
            time_base,
            rate,
            channels,
            duration,
            cancellation,
            interleaved: Vec::new(),
            discard_frames: 0,
        })
    }

    pub fn seek(&mut self, seconds: f64) -> AudioResult<()> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(AudioError::Invalid(
                "Seek position must be finite and non-negative".into(),
            ));
        }
        let seconds = self
            .duration
            .map_or(seconds, |duration| seconds.min(duration));
        let time = Time::try_from_secs_f64(seconds)
            .ok_or_else(|| AudioError::Invalid("Seek position is out of range".into()))?;
        let sought = self.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: Some(self.track),
            },
        )?;
        self.decoder.reset();
        let actual = self
            .time_base
            .and_then(|base| base.calc_time(sought.actual_ts))
            .map(|time| time.as_secs_f64())
            .ok_or_else(|| {
                AudioError::Invalid("This file does not expose a usable seek time base".into())
            })?;
        self.discard_frames = ((seconds - actual).max(0.0) * f64::from(self.rate)).round() as usize;
        Ok(())
    }

    /// Decode one bounded packet. Multichannel preview is a conservative mono
    /// average; mono is duplicated and stereo preserves its original channels.
    pub fn next(&mut self, output: &mut Vec<Stereo>) -> AudioResult<bool> {
        output.clear();
        loop {
            if self.cancellation.is_cancelled() {
                return Err(AudioError::Cancelled);
            }
            let Some(packet) = self.format.next_packet()? else {
                return Ok(false);
            };
            if packet.track_id != self.track {
                continue;
            }
            if packet.data.len() > 2 * 1024 * 1024 {
                return Err(AudioError::Invalid(
                    "Audio packet exceeds the 2 MiB preview limit".into(),
                ));
            }
            let decoded: GenericAudioBufferRef<'_> = self.decoder.decode(&packet)?;
            if decoded.spec().rate() != self.rate || decoded.num_planes() != self.channels {
                return Err(AudioError::Invalid(
                    "Changing sample rate/channel layout within one stream is unsupported".into(),
                ));
            }
            if decoded.frames() > 65_536 {
                return Err(AudioError::Invalid(
                    "Decoded audio packet exceeds the preview frame limit".into(),
                ));
            }
            decoded.copy_to_vec_interleaved(&mut self.interleaved);
            let skip = self.discard_frames.min(decoded.frames());
            self.discard_frames -= skip;
            for frame in self.interleaved.chunks_exact(self.channels).skip(skip) {
                if frame.iter().any(|sample| !sample.is_finite()) {
                    return Err(AudioError::Invalid(
                        "Audio contains a non-finite sample".into(),
                    ));
                }
                output.push(match self.channels {
                    1 => [frame[0], frame[0]],
                    2 => [frame[0], frame[1]],
                    _ => {
                        let mono = frame
                            .iter()
                            .map(|value| *value / self.channels as f32)
                            .sum::<f32>();
                        [mono, mono]
                    }
                });
            }
            while !self.format.metadata().is_latest() {
                self.format.metadata().pop();
            }
            if !output.is_empty() {
                return Ok(true);
            }
        }
    }
}

struct Source {
    file: File,
    length: u64,
    remaining_probe: usize,
    probing: Arc<AtomicBool>,
    cancellation: Cancellation,
}

impl Read for Source {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.cancellation.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Audio work cancelled",
            ));
        }
        let probing = self.probing.load(Ordering::Relaxed);
        if probing && self.remaining_probe == 0 {
            return Err(io::Error::other(
                "Audio header exceeds its 8 MiB read budget",
            ));
        }
        let length = if probing {
            buffer.len().min(self.remaining_probe)
        } else {
            buffer.len()
        };
        let read = self.file.read(&mut buffer[..length])?;
        if probing {
            self.remaining_probe -= read;
        }
        Ok(read)
    }
}
impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}
impl MediaSource for Source {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.length)
    }
}
