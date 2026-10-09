use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::Ordering,
    mpsc::{self, Receiver, SyncSender},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, Producer};

use super::{
    AudioError, AudioResult,
    decode::Decoder,
    resample::RateConverter,
    transport::{self, Frame, Shared},
};
use crate::library::Cancellation;

#[derive(Clone)]
struct Desired {
    generation: u64,
    path: Option<PathBuf>,
    seek: f64,
    paused: bool,
    shutdown: bool,
    cancellation: Cancellation,
}

pub struct Message {
    pub generation: u64,
    pub error: Option<String>,
    pub output: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub phase: u8,
    pub seconds: f64,
    pub rendered: u64,
    pub nonzero: u64,
    pub underruns: u64,
}

pub struct Player {
    desired: Arc<Mutex<Desired>>,
    shared: Arc<Shared>,
    wake: Option<SyncSender<()>>,
    messages: Option<Receiver<Message>>,
    thread: Option<JoinHandle<()>>,
}

impl Player {
    pub fn start(notify: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let desired = Arc::new(Mutex::new(Desired {
            generation: 1,
            path: None,
            seek: 0.0,
            paused: true,
            shutdown: false,
            cancellation: Cancellation::default(),
        }));
        let requested = desired.clone();
        let state = shared.clone();
        let (wake, commands) = mpsc::sync_channel(1);
        let (sender, messages) = mpsc::sync_channel(4);
        let thread = thread::Builder::new()
            .name("sift-audio".into())
            .spawn(move || run(requested, state, commands, sender, notify))?;
        Ok(Self {
            desired,
            shared,
            wake: Some(wake),
            messages: Some(messages),
            thread: Some(thread),
        })
    }

    fn request(&self, path: Option<PathBuf>, seek: f64, paused: bool) -> AudioResult<()> {
        if !seek.is_finite() || seek < 0.0 {
            return Err(AudioError::Invalid(
                "Seek position must be finite and non-negative".into(),
            ));
        }
        let mut desired = self
            .desired
            .lock()
            .map_err(|_| AudioError::Invalid("Audio control state is unavailable".into()))?;
        desired.cancellation.cancel();
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.shared.paused.store(true, Ordering::Release);
        self.shared.phase.store(
            if path.is_none() {
                transport::IDLE
            } else if paused {
                transport::PAUSED
            } else {
                transport::LOADING
            },
            Ordering::Release,
        );
        self.shared.eof.store(0, Ordering::Release);
        self.shared.position.store(
            (seek * f64::from(self.shared.rate.load(Ordering::Relaxed))) as u64,
            Ordering::Relaxed,
        );
        *desired = Desired {
            generation,
            path,
            seek,
            paused,
            shutdown: false,
            cancellation: Cancellation::default(),
        };
        drop(desired);
        if let Some(wake) = &self.wake {
            match wake.try_send(()) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                Err(_) => return Err(AudioError::Invalid("Audio worker stopped".into())),
            }
        }
        Ok(())
    }

    pub fn play(&self, path: PathBuf, seconds: f64) -> AudioResult<()> {
        self.request(Some(path), seconds, false)
    }
    pub fn stop(&self) -> AudioResult<()> {
        self.request(None, 0.0, true)
    }
    pub fn pause(&self) -> AudioResult<()> {
        let path = self
            .desired
            .lock()
            .map_err(|_| AudioError::Invalid("Audio control state is unavailable".into()))?
            .path
            .clone();
        self.request(path, self.snapshot().seconds, true)
    }
    pub fn resume(&self) -> AudioResult<()> {
        let desired = self
            .desired
            .lock()
            .map_err(|_| AudioError::Invalid("Audio control state is unavailable".into()))?
            .clone();
        self.request(desired.path, desired.seek, false)
    }
    pub fn reconnect(&self, path: PathBuf, seconds: f64) -> AudioResult<()> {
        self.request(Some(path), seconds, false)
    }
    pub fn volume(&self, value: f32) -> AudioResult<()> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(AudioError::Invalid("Volume must be between 0 and 1".into()));
        }
        self.shared.volume.store(value.to_bits(), Ordering::Relaxed);
        Ok(())
    }
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            phase: self.shared.phase.load(Ordering::Acquire),
            seconds: self.shared.position.load(Ordering::Relaxed) as f64
                / f64::from(self.shared.rate.load(Ordering::Relaxed)),
            rendered: self.shared.rendered.load(Ordering::Relaxed),
            nonzero: self.shared.nonzero.load(Ordering::Relaxed),
            underruns: self.shared.underruns.load(Ordering::Relaxed),
        }
    }
    pub fn poll(&self) -> Option<Message> {
        self.messages.as_ref()?.try_recv().ok()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Ok(mut desired) = self.desired.lock() {
            desired.shutdown = true;
            desired.cancellation.cancel();
        }
        self.shared.paused.store(true, Ordering::Release);
        if let Some(wake) = &self.wake {
            wake.try_send(()).ok();
        }
        drop(self.messages.take());
        drop(self.wake.take());
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("Audio worker panicked during shutdown");
        }
    }
}

struct Output {
    stream: cpal::Stream,
    producer: Producer<Frame>,
    errors: Consumer<cpal::Error>,
    rate: u32,
    playing: bool,
    name: String,
}

impl Output {
    fn open(shared: Arc<Shared>) -> AudioResult<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| AudioError::Invalid("No default audio output is available".into()))?;
        let default = device.default_output_config().map_err(device_error)?;
        let configuration = device
            .supported_output_configs()
            .map_err(device_error)?
            .filter(|range| {
                range.sample_format() == cpal::SampleFormat::F32
                    && (1..=8).contains(&range.channels())
            })
            .find_map(|range| range.try_with_sample_rate(default.sample_rate()))
            .ok_or_else(|| {
                AudioError::Invalid(
                    "The output does not support float PCM at its default sample rate".into(),
                )
            })?;
        let config = configuration.config();
        let rate = config.sample_rate;
        shared.rate.store(rate, Ordering::Relaxed);
        let name = device
            .description()
            .map_err(device_error)?
            .name()
            .to_owned();
        let (producer, mut render) = transport::channel(shared, usize::from(config.channels));
        let (mut error_sender, errors) = rtrb::RingBuffer::new(8);
        let stream = device
            .build_output_stream(
                config,
                move |samples: &mut [f32], _| render.fill(samples),
                move |error| {
                    error_sender.push(error).ok();
                },
                Some(Duration::from_secs(2)),
            )
            .map_err(device_error)?;
        Ok(Self {
            stream,
            producer,
            errors,
            rate,
            playing: false,
            name,
        })
    }
    fn play(&mut self) -> AudioResult<()> {
        if !self.playing {
            self.stream.play().map_err(device_error)?;
            self.playing = true;
        }
        Ok(())
    }
    fn pause(&mut self) -> AudioResult<()> {
        if self.playing {
            self.stream.pause().map_err(device_error)?;
            self.playing = false;
        }
        Ok(())
    }
}

fn device_error(error: cpal::Error) -> AudioError {
    AudioError::Invalid(format!("Audio device operation failed: {error}"))
}

struct Job {
    decoder: Decoder,
    converter: RateConverter,
    desired: Desired,
    position: u64,
    finished: bool,
}

fn run(
    desired: Arc<Mutex<Desired>>,
    shared: Arc<Shared>,
    commands: Receiver<()>,
    messages: SyncSender<Message>,
    notify: impl Fn(),
) {
    let mut output: Option<Output> = None;
    let mut job: Option<Job> = None;
    let mut generation = 0;
    let mut packet = Vec::new();
    loop {
        let active = job.as_ref().is_some_and(|job| !job.desired.paused);
        let needs_data = job.as_ref().is_some_and(|job| !job.finished)
            && output
                .as_ref()
                .is_some_and(|output| output.producer.slots() > transport::BUFFER_FRAMES / 2);
        let changed = if active && needs_data {
            match commands.try_recv() {
                Ok(()) => true,
                Err(mpsc::TryRecvError::Empty) => false,
                Err(_) => break,
            }
        } else if active {
            match commands.recv_timeout(Duration::from_millis(5)) {
                Ok(()) => true,
                Err(mpsc::RecvTimeoutError::Timeout) => false,
                Err(_) => break,
            }
        } else {
            if commands.recv().is_err() {
                break;
            }
            true
        };
        if changed {
            let wanted = match desired.lock() {
                Ok(desired) => desired.clone(),
                Err(_) => {
                    eprintln!("Audio control state poisoned");
                    break;
                }
            };
            if wanted.shutdown {
                break;
            }
            if wanted.generation != generation {
                generation = wanted.generation;
                job = None;
                if wanted.path.is_none() || wanted.paused {
                    if let Some(output) = &mut output
                        && let Err(error) = output.pause()
                    {
                        send_error(generation, error, &shared, &messages, &notify);
                    }
                    output = None;
                    continue;
                }
                // Rebuild the owned stream on file/seek changes. Queued OS audio
                // cannot be invalidated by tagging only our own ring-buffer frames.
                output = None;
                let result = (|| {
                    let path = wanted
                        .path
                        .as_ref()
                        .ok_or_else(|| AudioError::Invalid("No audio source selected".into()))?;
                    let mut decoder = Decoder::open(path, wanted.cancellation.clone())?;
                    let seek = decoder
                        .duration
                        .map_or(wanted.seek, |duration| wanted.seek.min(duration));
                    if seek > 0.0 {
                        decoder.seek(seek)?;
                    }
                    if wanted.cancellation.is_cancelled() {
                        return Err(AudioError::Cancelled);
                    }
                    if output.is_none() {
                        output = Some(Output::open(shared.clone())?);
                    }
                    let output = output.as_mut().ok_or_else(|| {
                        AudioError::Invalid("Audio output did not initialize".into())
                    })?;
                    let converter = RateConverter::new(decoder.rate, output.rate)?;
                    let position = (seek * f64::from(output.rate)) as u64;
                    shared.position.store(position, Ordering::Relaxed);
                    if messages
                        .send(Message {
                            generation,
                            error: None,
                            output: Some(output.name.clone()),
                        })
                        .is_ok()
                    {
                        notify();
                    }
                    Ok(Job {
                        decoder,
                        converter,
                        desired: wanted.clone(),
                        position,
                        finished: false,
                    })
                })();
                match result {
                    Ok(prepared) => {
                        job = Some(prepared);
                    }
                    Err(AudioError::Cancelled) => {}
                    Err(error) => {
                        send_error(generation, error, &shared, &messages, &notify);
                        output = None;
                    }
                }
            }
        }
        let (Some(current), Some(device)) = (&mut job, &mut output) else {
            continue;
        };
        if let Ok(error) = device.errors.pop() {
            send_error(generation, device_error(error), &shared, &messages, &notify);
            job = None;
            output = None;
            continue;
        }
        if shared.phase.load(Ordering::Acquire) == transport::ENDED {
            if let Err(error) = device.pause() {
                send_error(generation, error, &shared, &messages, &notify);
            }
            job = None;
            output = None;
            notify();
            continue;
        }
        if current.finished {
            continue;
        }
        let result = (|| {
            let more = current.decoder.next(&mut packet)?;
            let generation = current.desired.generation;
            let cancellation = current.desired.cancellation.clone();
            current.converter.process(&packet, !more, |samples| {
                let frame = Frame {
                    generation,
                    position: current.position,
                    samples,
                };
                loop {
                    if cancellation.is_cancelled() {
                        return Err(AudioError::Cancelled);
                    }
                    match device.producer.push(frame) {
                        Ok(()) => break,
                        Err(_) => thread::sleep(Duration::from_millis(1)),
                    }
                }
                current.position += 1;
                if !device.playing && device.producer.slots() <= transport::BUFFER_FRAMES - 4096 {
                    shared.paused.store(false, Ordering::Release);
                    device.play()?;
                }
                if device.playing && shared.generation.load(Ordering::Acquire) == generation {
                    shared.phase.store(transport::PLAYING, Ordering::Release);
                }
                Ok(())
            })?;
            if !more {
                current.finished = true;
                shared.eof.store(generation, Ordering::Release);
                if device.producer.slots() == transport::BUFFER_FRAMES {
                    shared.phase.store(transport::ENDED, Ordering::Release);
                } else if !device.playing {
                    shared.paused.store(false, Ordering::Release);
                    shared.phase.store(transport::PLAYING, Ordering::Release);
                    device.play()?;
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => {}
            Err(AudioError::Cancelled) => {
                job = None;
            }
            Err(error) => {
                if !current.desired.cancellation.is_cancelled() {
                    send_error(generation, error, &shared, &messages, &notify);
                }
                job = None;
            }
        }
    }
}

fn send_error(
    generation: u64,
    error: AudioError,
    shared: &Shared,
    messages: &SyncSender<Message>,
    notify: &impl Fn(),
) {
    if shared.generation.load(Ordering::Acquire) != generation {
        return;
    }
    shared.paused.store(true, Ordering::Release);
    shared.phase.store(transport::FAILED, Ordering::Release);
    if messages
        .send(Message {
            generation,
            error: Some(error.to_string()),
            output: None,
        })
        .is_ok()
    {
        notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_controls_do_not_change_the_active_generation() {
        let player = Player::start(|| {}).unwrap();
        let generation = player.generation();
        assert!(player.play(PathBuf::from("unused.wav"), f64::NAN).is_err());
        assert!(player.volume(f32::NAN).is_err());
        assert!(player.volume(2.0).is_err());
        assert_eq!(player.generation(), generation);
        assert_eq!(player.snapshot().phase, transport::IDLE);
    }

    #[test]
    fn missing_source_fails_before_opening_an_output_device() {
        let temporary = tempfile::tempdir().unwrap();
        let player = Player::start(|| {}).unwrap();
        player
            .play(temporary.path().join("missing.wav"), 0.0)
            .unwrap();
        let message = player
            .messages
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert!(message.error.is_some());
        assert!(message.output.is_none());
        assert_eq!(message.generation, player.generation());
    }
}
