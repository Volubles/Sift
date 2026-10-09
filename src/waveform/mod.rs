mod cache;

use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, SyncSender},
};
use std::thread::{self, JoinHandle};
use std::time::SystemTime;

use crate::audio::{AudioError, AudioResult, decode::Decoder};
use crate::library::Cancellation;

pub const MAX_BINS: usize = 2048;

#[derive(Clone, Debug)]
pub struct Waveform {
    pub rate: u32,
    pub frames: u64,
    pub frames_per_bin: u64,
    pub peaks: Vec<[f32; 2]>,
    pub(crate) source_size: u64,
    pub(crate) source_modified: SystemTime,
}

impl Waveform {
    pub fn duration(&self) -> f64 {
        self.frames as f64 / f64::from(self.rate)
    }
}

pub fn analyze(path: &Path, cancellation: Cancellation) -> AudioResult<Waveform> {
    let before = path.metadata()?;
    let mut decoder = Decoder::open(path, cancellation.clone())?;
    let mut peaks = Vec::with_capacity(MAX_BINS);
    let mut span = 1u64;
    let mut frames = 0u64;
    let mut current = [f32::INFINITY, f32::NEG_INFINITY];
    let mut count = 0u64;
    let mut packet = Vec::new();
    while decoder.next(&mut packet)? {
        for frame in &packet {
            current[0] = current[0].min(frame[0]).min(frame[1]);
            current[1] = current[1].max(frame[0]).max(frame[1]);
            count += 1;
            frames += 1;
            if count == span {
                if peaks.len() == MAX_BINS {
                    for index in 0..MAX_BINS / 2 {
                        let left: [f32; 2] = peaks[index * 2];
                        let right: [f32; 2] = peaks[index * 2 + 1];
                        peaks[index] = [left[0].min(right[0]), left[1].max(right[1])];
                    }
                    peaks.truncate(MAX_BINS / 2);
                    span *= 2;
                } else {
                    peaks.push(current);
                    current = [f32::INFINITY, f32::NEG_INFINITY];
                    count = 0;
                }
            }
        }
        if cancellation.is_cancelled() {
            return Err(AudioError::Cancelled);
        }
        thread::yield_now();
    }
    if count > 0 {
        peaks.push(current);
    }
    if frames == 0 {
        return Err(AudioError::Invalid(
            "Audio file contains no decoded frames".into(),
        ));
    }
    let after = path.metadata()?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(AudioError::Invalid(
            "Source changed during waveform analysis".into(),
        ));
    }
    Ok(Waveform {
        rate: decoder.rate,
        frames,
        frames_per_bin: span,
        peaks,
        source_size: before.len(),
        source_modified: before.modified()?,
    })
}

pub use cache::Cache;

pub struct Ready {
    pub generation: u64,
    pub result: AudioResult<Arc<Waveform>>,
    pub cache_hit: bool,
}

struct Request {
    generation: u64,
    path: PathBuf,
    cancellation: Cancellation,
}

pub struct Waveforms {
    desired: Arc<Mutex<Option<Request>>>,
    wake: Option<SyncSender<()>>,
    results: Option<Receiver<Ready>>,
    cancellation: Option<Cancellation>,
    thread: Option<JoinHandle<()>>,
}

impl Waveforms {
    pub fn start(database: PathBuf, notify: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let desired: Arc<Mutex<Option<Request>>> = Arc::new(Mutex::new(None));
        let requested = desired.clone();
        let (wake, receiver) = mpsc::sync_channel(1);
        let (sender, results) = mpsc::sync_channel(2);
        let thread = thread::Builder::new()
            .name("sift-waveform".into())
            .spawn(move || {
                let mut cache = match Cache::open(&database.with_extension("waveforms.sqlite")) {
                    Ok(cache) => Some(cache),
                    Err(error) => {
                        eprintln!("Waveform cache unavailable; using uncached analysis: {error}");
                        None
                    }
                };
                while receiver.recv().is_ok() {
                    let request = match requested.lock() {
                        Ok(mut slot) => slot.take(),
                        Err(_) => {
                            eprintln!("Waveform request state poisoned");
                            break;
                        }
                    };
                    let Some(request) = request else {
                        continue;
                    };
                    if request.cancellation.is_cancelled() {
                        continue;
                    }
                    let mut cache_hit = false;
                    let result = (|| {
                        if let Some(cache) = &mut cache {
                            match cache.get(&request.path) {
                                Ok(Some(waveform)) => {
                                    cache_hit = true;
                                    return Ok(Arc::new(waveform));
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    eprintln!("Waveform cache lookup failed; rebuilding: {error}")
                                }
                            }
                        }
                        let waveform = analyze(&request.path, request.cancellation.clone())?;
                        if let Some(cache) = &mut cache
                            && let Err(error) = cache.save(&request.path, &waveform)
                        {
                            eprintln!("Waveform cache save failed: {error}");
                        }
                        Ok(Arc::new(waveform))
                    })();
                    if request.cancellation.is_cancelled() {
                        continue;
                    }
                    if sender
                        .send(Ready {
                            generation: request.generation,
                            result,
                            cache_hit,
                        })
                        .is_err()
                    {
                        break;
                    }
                    notify();
                }
            })?;
        Ok(Self {
            desired,
            wake: Some(wake),
            results: Some(results),
            cancellation: None,
            thread: Some(thread),
        })
    }

    pub fn request(&mut self, generation: u64, path: PathBuf) -> AudioResult<()> {
        if let Some(token) = self.cancellation.take() {
            token.cancel();
        }
        let cancellation = Cancellation::default();
        let mut slot = self
            .desired
            .lock()
            .map_err(|_| AudioError::Invalid("Waveform worker state is unavailable".into()))?;
        *slot = Some(Request {
            generation,
            path,
            cancellation: cancellation.clone(),
        });
        self.cancellation = Some(cancellation);
        if let Some(wake) = &self.wake {
            match wake.try_send(()) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                Err(_) => return Err(AudioError::Invalid("Waveform worker stopped".into())),
            }
        }
        Ok(())
    }
    pub fn poll(&self) -> Option<Ready> {
        self.results.as_ref()?.try_recv().ok()
    }
}

impl Drop for Waveforms {
    fn drop(&mut self) {
        if let Some(token) = &self.cancellation {
            token.cancel();
        }
        drop(self.results.take());
        drop(self.wake.take());
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("Waveform worker panicked during shutdown");
        }
    }
}
