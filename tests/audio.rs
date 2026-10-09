mod common;
use common::wave;

use sift::audio::{decode::Decoder, resample::RateConverter};
use sift::library::Cancellation;

#[test]
fn decoding_and_accurate_seek_preserve_sources_and_duplicate_mono() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("ramp.wav");
    wave(&path, 48_000, 48_000);
    let original = std::fs::read(&path).unwrap();
    let mut decoder = Decoder::open(&path, Cancellation::default()).unwrap();
    let mut packet = Vec::new();
    let mut count = 0;
    while decoder.next(&mut packet).unwrap() {
        count += packet.len();
        assert!(packet.iter().all(|frame| frame[0] == frame[1]));
    }
    assert_eq!(count, 48_000);
    decoder.seek(0.5).unwrap();
    count = 0;
    while decoder.next(&mut packet).unwrap() {
        count += packet.len();
    }
    assert_eq!(count, 24_000);
    assert!(decoder.seek(f64::NAN).is_err());
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn cancellation_stops_decoding_and_invalid_media_is_reported() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("tone.wav");
    wave(&path, 44_100, 44_100);
    let token = Cancellation::default();
    let mut decoder = Decoder::open(&path, token.clone()).unwrap();
    token.cancel();
    assert!(decoder.next(&mut Vec::new()).is_err());
    std::fs::write(&path, b"invalid wave").unwrap();
    assert!(Decoder::open(&path, Cancellation::default()).is_err());
}

#[test]
fn streaming_resampling_trims_delay_and_preserves_duration_and_stereo() {
    for (input_rate, output_rate) in [(44_100, 48_000), (48_000, 44_100), (48_000, 48_000)] {
        let mut converter = RateConverter::new(input_rate, output_rate).unwrap();
        let mut result = Vec::new();
        let frames: Vec<_> = (0..input_rate)
            .map(|index| {
                let sample =
                    (index as f32 * 440.0 * std::f32::consts::TAU / input_rate as f32).sin() * 0.2;
                [sample, -sample]
            })
            .collect();
        for chunk in frames.chunks(137) {
            converter
                .process(chunk, false, |frame| {
                    result.push(frame);
                    Ok(())
                })
                .unwrap();
        }
        converter
            .process(&[], true, |frame| {
                result.push(frame);
                Ok(())
            })
            .unwrap();
        assert_eq!(result.len(), output_rate as usize);
        assert!(
            result
                .iter()
                .all(|frame| frame.iter().all(|sample| sample.is_finite())
                    && (frame[0] + frame[1]).abs() < 1e-5)
        );
        assert!(
            result
                .iter()
                .map(|frame| frame[0].abs())
                .fold(0.0, f32::max)
                > 0.19
        );
    }
}

#[test]
fn very_short_resampled_clip_flushes_without_extra_silence() {
    let mut converter = RateConverter::new(44_100, 48_000).unwrap();
    let mut result = Vec::new();
    converter
        .process(&[[0.2, 0.2]; 20], true, |frame| {
            result.push(frame);
            Ok(())
        })
        .unwrap();
    assert_eq!(result.len(), 22);
    assert!(RateConverter::new(0, 48_000).is_err());
}
