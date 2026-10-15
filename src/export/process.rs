use super::{Channels, Depth, Error, Format, Mp3Quality, Result, Settings};
use crate::library::{AudioMetadata, Cancellation};
use std::{
    collections::VecDeque,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

pub struct Runtime {
    executable: PathBuf,
}
impl Runtime {
    pub fn bundled() -> Result<Self> {
        let path = std::env::current_exe()?
            .parent()
            .ok_or_else(|| Error::Invalid("Application folder is unavailable".into()))?
            .join("export-tools")
            .join(if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            });
        Ok(Self { executable: path })
    }
    /// Explicit paths are useful for packaging qualification; ordinary sessions use the bundled tool.
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }
    pub fn verify(&self, token: &Cancellation) -> Result<()> {
        let progress = AtomicU64::new(0);
        let mut version = self.command();
        version.arg("-version");
        let output = run(version, token, &progress, None)?;
        if output
            .split_whitespace()
            .any(|arg| arg == "--enable-gpl" || arg == "--enable-nonfree")
        {
            return Err(Error::Invalid(
                "Export tools must use the packaged LGPL build".into(),
            ));
        }
        let mut license = self.command();
        license.arg("-L");
        if !run(license, token, &progress, None)?.contains("GNU Lesser General Public License") {
            return Err(Error::Invalid(
                "The export tool did not report an LGPL license".into(),
            ));
        }
        let mut encoders = self.command();
        encoders.arg("-encoders");
        let list = run(encoders, token, &progress, None)?;
        for required in [
            "pcm_s16le",
            "pcm_s24le",
            "pcm_f32le",
            "pcm_s16be",
            "pcm_s24be",
            "pcm_f32be",
            "flac",
            "libmp3lame",
            "libvorbis",
        ] {
            if !list.split_whitespace().any(|word| word == required) {
                return Err(Error::Invalid(format!(
                    "Export tool is missing the {required} encoder"
                )));
            }
        }
        Ok(())
    }
    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.arg("-hide_banner");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        command
    }
    pub(super) fn encode(
        &self,
        source: &Path,
        target: &Path,
        settings: &Settings,
        audio: &AudioMetadata,
        token: &Cancellation,
        progress: &AtomicU64,
    ) -> Result<()> {
        validate_audio(settings, audio)?;
        let mut cmd = self.command();
        cmd.args([
            "-nostdin",
            "-nostats",
            "-loglevel",
            "error",
            "-y",
            "-threads",
            "2",
            "-filter_threads",
            "1",
            "-progress",
            "pipe:1",
            "-stats_period",
            "0.2",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(source);
        cmd.args([
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-dn",
            "-map_metadata",
            "-1",
            "-threads",
            "2",
        ]);
        let rate = settings
            .sample_rate
            .or(audio.sample_rate)
            .ok_or_else(|| Error::Invalid("Source sample rate is unknown".into()))?;
        cmd.arg("-ar").arg(rate.to_string());
        match settings.channels {
            Channels::Original => {}
            Channels::Mono => {
                cmd.args(["-ac", "1"]);
            }
            Channels::Stereo => {
                cmd.args(["-ac", "2"]);
            }
        }
        match settings.format {
            Format::Original => {
                return Err(Error::Invalid("Original exports use byte copying".into()));
            }
            Format::Wav | Format::Aiff => {
                let codec = match (settings.format, settings.depth) {
                    (Format::Wav, Depth::Int16) => "pcm_s16le",
                    (Format::Wav, Depth::Int24) => "pcm_s24le",
                    (Format::Wav, Depth::Float32) => "pcm_f32le",
                    (_, Depth::Int16) => "pcm_s16be",
                    (_, Depth::Int24) => "pcm_s24be",
                    (_, Depth::Float32) => "pcm_f32be",
                };
                cmd.args([
                    "-c:a",
                    codec,
                    "-f",
                    if settings.format == Format::Wav {
                        "wav"
                    } else {
                        "aiff"
                    },
                ]);
                if settings.format == Format::Wav {
                    cmd.args(["-rf64", "auto"]);
                }
            }
            Format::Flac => {
                cmd.args([
                    "-c:a",
                    "flac",
                    "-sample_fmt",
                    if settings.depth == Depth::Int16 {
                        "s16"
                    } else {
                        "s32"
                    },
                    "-bits_per_raw_sample",
                    if settings.depth == Depth::Int16 {
                        "16"
                    } else {
                        "24"
                    },
                    "-compression_level",
                ])
                .arg(settings.flac_compression.to_string())
                .args(["-f", "flac"]);
            }
            Format::Mp3 => {
                cmd.args(["-c:a", "libmp3lame"]);
                match settings.mp3 {
                    Mp3Quality::Bitrate(kbps) => {
                        cmd.arg("-b:a").arg(format!("{kbps}k"));
                    }
                    Mp3Quality::Vbr(q) => {
                        cmd.arg("-q:a").arg(q.to_string());
                    }
                }
                cmd.args(["-f", "mp3"]);
            }
            Format::Ogg => {
                cmd.args(["-c:a", "libvorbis", "-q:a"])
                    .arg(settings.vorbis_quality.to_string())
                    .args(["-f", "ogg"]);
            }
        }
        cmd.arg(target);
        run(cmd, token, progress, audio.duration_seconds)?;
        if File::open(target)?.metadata()?.len() == 0 {
            return Err(Error::Encoder("Encoder produced an empty file".into()));
        }
        Ok(())
    }
}
pub(super) fn validate_audio(settings: &Settings, audio: &AudioMetadata) -> Result<()> {
    settings.validate()?;
    let rate = settings.sample_rate.or(audio.sample_rate).ok_or_else(|| {
        Error::Invalid("Source sample rate is unknown; refresh its library".into())
    })?;
    let channels = match settings.channels {
        Channels::Original => audio.channels.unwrap_or(0),
        Channels::Mono => 1,
        Channels::Stereo => 2,
    };
    if channels == 0 {
        return Err(Error::Invalid("Source channel count is unknown".into()));
    }
    if settings.format == Format::Mp3
        && (channels > 2
            || ![8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000].contains(&rate))
    {
        return Err(Error::Invalid("MP3 supports mono/stereo and sample rates up to 48 kHz; choose a supported rate and channel setting".into()));
    }
    if settings.format == Format::Ogg && channels > 8 {
        return Err(Error::Invalid(
            "Ogg Vorbis export supports at most eight channels; choose mono or stereo".into(),
        ));
    }
    Ok(())
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        match self.0.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                if let Err(error) = self.0.kill() {
                    eprintln!("Export process cleanup could not stop the encoder: {error}");
                }
                if let Err(error) = self.0.wait() {
                    eprintln!("Export process cleanup could not join the encoder: {error}");
                }
            }
        }
    }
}
fn run(
    mut command: Command,
    token: &Cancellation,
    progress: &AtomicU64,
    duration: Option<f64>,
) -> Result<String> {
    if token.is_cancelled() {
        return Err(Error::Cancelled);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    thread::scope(|scope| {
        let mut running=Running(command.spawn().map_err(|error|Error::Encoder(format!("Export tool could not start: {error}. Install the packaged export-tools folder beside Sift")))?);
        let stdout = running
            .0
            .stdout
            .take()
            .ok_or_else(|| Error::Encoder("Encoder stdout is unavailable".into()))?;
        let stderr = running
            .0
            .stderr
            .take()
            .ok_or_else(|| Error::Encoder("Encoder stderr is unavailable".into()))?;
        let output = thread::Builder::new()
            .name("sift-export-progress".into())
            .spawn_scoped(scope, move || read_stdout(stdout, progress, duration))?;
        let errors = thread::Builder::new()
            .name("sift-export-errors".into())
            .spawn_scoped(scope, move || read_tail(stderr))?;
        let status = loop {
            if token.is_cancelled() {
                running.0.kill()?;
                break running.0.wait()?;
            }
            if let Some(status) = running.0.try_wait()? {
                break status;
            }
            thread::sleep(Duration::from_millis(25));
        };
        let output = output
            .join()
            .map_err(|_| Error::Encoder("Encoder progress reader failed".into()))??;
        let errors = errors
            .join()
            .map_err(|_| Error::Encoder("Encoder error reader failed".into()))??;
        if token.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if !status.success() {
            return Err(Error::Encoder(format!("{status}: {}", errors.trim())));
        }
        Ok(output)
    })
}
fn read_stdout(
    mut stream: impl Read,
    progress: &AtomicU64,
    duration: Option<f64>,
) -> std::io::Result<String> {
    let mut buffer = [0u8; 4096];
    let mut output = Vec::new();
    let mut line = Vec::new();
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.extend(
            buffer[..count]
                .iter()
                .take((128 * 1024usize).saturating_sub(output.len())),
        );
        for byte in &buffer[..count] {
            if *byte == b'\n' {
                if let Some(micros) = std::str::from_utf8(&line)
                    .ok()
                    .and_then(|line| line.trim().strip_prefix("out_time_us="))
                    .and_then(|time| time.parse::<u64>().ok())
                    && let Some(seconds) =
                        duration.filter(|seconds| seconds.is_finite() && *seconds > 0.0)
                {
                    progress.store(
                        ((micros as f64 / (seconds * 1000.0)) as u64).min(999),
                        Ordering::Relaxed,
                    );
                }
                line.clear();
            } else if line.len() < 1024 {
                line.push(*byte);
            } else {
                line.clear();
            }
        }
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}
fn read_tail(mut stream: impl Read) -> std::io::Result<String> {
    let mut tail = VecDeque::with_capacity(8192);
    let mut buffer = [0u8; 4096];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        for byte in &buffer[..count] {
            if tail.len() == 8192 {
                tail.pop_front();
            }
            tail.push_back(*byte);
        }
    }
    Ok(String::from_utf8_lossy(&tail.into_iter().collect::<Vec<_>>()).into_owned())
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/export/process.rs"
    ));
}
