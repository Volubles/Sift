use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use super::Stereo;
use rtrb::{Consumer, Producer, RingBuffer};

pub const BUFFER_FRAMES: usize = 16_384;
pub const IDLE: u8 = 0;
pub const LOADING: u8 = 1;
pub const PLAYING: u8 = 2;
pub const PAUSED: u8 = 3;
pub const ENDED: u8 = 4;
pub const FAILED: u8 = 5;

pub struct Shared {
    pub generation: AtomicU64,
    pub paused: AtomicBool,
    pub volume: AtomicU32,
    pub phase: AtomicU8,
    pub rate: AtomicU32,
    pub position: AtomicU64,
    pub eof: AtomicU64,
    pub underruns: AtomicU64,
    pub rendered: AtomicU64,
    pub nonzero: AtomicU64,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            generation: AtomicU64::new(1),
            paused: AtomicBool::new(true),
            volume: AtomicU32::new(0.4f32.to_bits()),
            phase: AtomicU8::new(IDLE),
            rate: AtomicU32::new(48_000),
            position: AtomicU64::new(0),
            eof: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            rendered: AtomicU64::new(0),
            nonzero: AtomicU64::new(0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub generation: u64,
    pub position: u64,
    pub samples: Stereo,
}

pub struct Render {
    consumer: Consumer<Frame>,
    shared: Arc<Shared>,
    channels: usize,
}

pub fn channel(shared: Arc<Shared>, channels: usize) -> (Producer<Frame>, Render) {
    let (producer, consumer) = RingBuffer::new(BUFFER_FRAMES);
    (
        producer,
        Render {
            consumer,
            shared,
            channels,
        },
    )
}

impl Render {
    /// The device callback only reads preallocated ring slots and atomics. It
    /// never performs I/O, allocations, logging, locking, or UI notifications.
    pub fn fill(&mut self, output: &mut [f32]) {
        output.fill(0.0);
        if self.channels == 0 {
            return;
        }
        let generation = self.shared.generation.load(Ordering::Acquire);
        // Discard obsolete audio even while paused, preserving current frames.
        for _ in 0..BUFFER_FRAMES {
            if self
                .consumer
                .peek()
                .is_ok_and(|frame| frame.generation != generation)
            {
                self.consumer.pop().ok();
            } else {
                break;
            }
        }
        if self.shared.paused.load(Ordering::Relaxed) {
            return;
        }
        let gain = f32::from_bits(self.shared.volume.load(Ordering::Relaxed));
        let mut rendered = 0;
        let mut nonzero = 0;
        for destination in output.chunks_exact_mut(self.channels) {
            if self.shared.generation.load(Ordering::Acquire) != generation {
                break;
            }
            match self.consumer.pop() {
                Ok(frame) if frame.generation == generation => {
                    let left = (frame.samples[0] * gain).clamp(-1.0, 1.0);
                    let right = (frame.samples[1] * gain).clamp(-1.0, 1.0);
                    destination[0] = if self.channels == 1 {
                        (left + right) * 0.5
                    } else {
                        left
                    };
                    if self.channels > 1 {
                        destination[1] = right;
                    }
                    self.shared
                        .position
                        .store(frame.position + 1, Ordering::Relaxed);
                    rendered += 1;
                    if left != 0.0 || right != 0.0 {
                        nonzero += 1;
                    }
                }
                _ => {
                    if self.shared.eof.load(Ordering::Acquire) == generation {
                        self.shared.phase.store(ENDED, Ordering::Release);
                    } else if self.shared.phase.load(Ordering::Relaxed) == PLAYING {
                        self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                    }
                    break;
                }
            }
        }
        self.shared.rendered.fetch_add(rendered, Ordering::Relaxed);
        self.shared.nonzero.fetch_add(nonzero, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_audio_is_discarded_without_losing_paused_current_audio() {
        let shared = Arc::new(Shared::default());
        let (mut producer, mut render) = channel(shared.clone(), 2);
        producer
            .push(Frame {
                generation: 1,
                position: 0,
                samples: [1.0, 1.0],
            })
            .unwrap();
        producer
            .push(Frame {
                generation: 2,
                position: 42,
                samples: [0.5, -0.5],
            })
            .unwrap();
        shared.generation.store(2, Ordering::Release);
        let mut output = [9.0; 4];
        render.fill(&mut output);
        assert_eq!(output, [0.0; 4]);
        shared.paused.store(false, Ordering::Relaxed);
        render.fill(&mut output);
        assert_eq!(output, [0.2, -0.2, 0.0, 0.0]);
        assert_eq!(shared.position.load(Ordering::Relaxed), 43);
    }

    #[test]
    fn silence_and_end_of_stream_do_not_advance_the_playhead() {
        let shared = Arc::new(Shared::default());
        let (_, mut render) = channel(shared.clone(), 2);
        shared.paused.store(false, Ordering::Relaxed);
        shared.eof.store(1, Ordering::Release);
        let mut output = [1.0; 16];
        render.fill(&mut output);
        assert_eq!(output, [0.0; 16]);
        assert_eq!(shared.phase.load(Ordering::Relaxed), ENDED);
        assert_eq!(shared.position.load(Ordering::Relaxed), 0);
    }
}
