use std::collections::VecDeque;

use rubato::{Fft, FixedSync, Indexing, Resampler, audioadapter_buffers::direct::InterleavedSlice};

use super::{AudioError, AudioResult, Stereo};

pub struct RateConverter {
    fft: Option<Fft<f32>>,
    input: VecDeque<Stereo>,
    source_rate: u32,
    output_rate: u32,
    fed: u64,
    emitted: u64,
    trim: usize,
    input_buffer: Vec<f32>,
    output_buffer: Vec<f32>,
}

impl RateConverter {
    pub fn new(source_rate: u32, output_rate: u32) -> AudioResult<Self> {
        if source_rate == 0 || output_rate == 0 {
            return Err(AudioError::Invalid("Sample rates must be positive".into()));
        }
        let fft = if source_rate == output_rate {
            None
        } else {
            Some(
                Fft::new(
                    source_rate as usize,
                    output_rate as usize,
                    1024,
                    2,
                    FixedSync::Input,
                )
                .map_err(|error| {
                    AudioError::Invalid(format!("Resampler initialization failed: {error}"))
                })?,
            )
        };
        let (input_frames, output_frames, trim) = fft
            .as_ref()
            .map(|fft| {
                (
                    fft.input_frames_max(),
                    fft.output_frames_max(),
                    fft.output_delay(),
                )
            })
            .unwrap_or((0, 0, 0));
        if input_frames > 65_536 || output_frames > 65_536 {
            return Err(AudioError::Invalid(
                "Sample-rate ratio exceeds preview buffer limits".into(),
            ));
        }
        Ok(Self {
            fft,
            input: VecDeque::new(),
            source_rate,
            output_rate,
            fed: 0,
            emitted: 0,
            trim,
            input_buffer: vec![0.0; input_frames * 2],
            output_buffer: vec![0.0; output_frames * 2],
        })
    }

    pub fn process(
        &mut self,
        frames: &[Stereo],
        finish: bool,
        mut emit: impl FnMut(Stereo) -> AudioResult<()>,
    ) -> AudioResult<()> {
        if self.fft.is_none() {
            for &frame in frames {
                emit(frame)?;
            }
            self.fed += frames.len() as u64;
            self.emitted += frames.len() as u64;
            return Ok(());
        }
        if frames.len() > 65_536 {
            return Err(AudioError::Invalid(
                "Input packet exceeds resampler limits".into(),
            ));
        }
        self.input.extend(frames);
        let total_input = self.fed + self.input.len() as u64;
        let target = ((u128::from(total_input) * u128::from(self.output_rate))
            .div_ceil(u128::from(self.source_rate))) as u64;
        while self.input.len() >= self.fft.as_ref().map_or(0, |fft| fft.input_frames_next()) {
            let count = self.fft.as_ref().map_or(0, |fft| fft.input_frames_next());
            if count == 0 {
                break;
            }
            self.chunk(count, if finish { Some(target) } else { None }, &mut emit)?;
        }
        if finish {
            if !self.input.is_empty() {
                self.chunk(self.input.len(), Some(target), &mut emit)?;
            }
            for _ in 0..64 {
                if self.emitted >= target {
                    return Ok(());
                }
                self.chunk(0, Some(target), &mut emit)?;
            }
            return Err(AudioError::Invalid(
                "Resampler could not flush its delayed tail".into(),
            ));
        }
        Ok(())
    }

    fn chunk(
        &mut self,
        count: usize,
        target: Option<u64>,
        emit: &mut impl FnMut(Stereo) -> AudioResult<()>,
    ) -> AudioResult<()> {
        for index in 0..count {
            let frame = self
                .input
                .pop_front()
                .ok_or_else(|| AudioError::Invalid("Resampler input accounting failed".into()))?;
            self.input_buffer[index * 2..index * 2 + 2].copy_from_slice(&frame);
        }
        self.fed += count as u64;
        let Some(fft) = self.fft.as_mut() else {
            return Ok(());
        };
        let input = InterleavedSlice::new(&self.input_buffer, 2, count)
            .map_err(|error| AudioError::Invalid(error.to_string()))?;
        let output_frames = self.output_buffer.len() / 2;
        let mut output = InterleavedSlice::new_mut(&mut self.output_buffer, 2, output_frames)
            .map_err(|error| AudioError::Invalid(error.to_string()))?;
        let mut indexing = Indexing::new();
        indexing.partial_len = Some(count);
        let (_, produced) = fft
            .process_into_buffer(&input, &mut output, Some(&indexing))
            .map_err(|error| AudioError::Invalid(format!("Resampling failed: {error}")))?;
        for index in 0..produced {
            if self.trim > 0 {
                self.trim -= 1;
                continue;
            }
            if target.is_some_and(|target| self.emitted >= target) {
                break;
            }
            let frame = [
                self.output_buffer[index * 2],
                self.output_buffer[index * 2 + 1],
            ];
            if !frame.iter().all(|sample| sample.is_finite()) {
                return Err(AudioError::Invalid(
                    "Resampler produced a non-finite sample".into(),
                ));
            }
            emit(frame)?;
            self.emitted += 1;
        }
        Ok(())
    }
}
