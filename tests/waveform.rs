mod common;

use sift::library::Cancellation;
use sift::waveform::{self, Cache, MAX_BINS};

#[test]
fn long_waveform_preserves_extrema_and_bounds_bins() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("ramp.wav");
    common::wave(&source, 8_000, 160_000);
    let original = std::fs::read(&source).unwrap();
    let waveform = waveform::analyze(&source, Cancellation::default()).unwrap();
    assert_eq!(waveform.frames, 160_000);
    assert_eq!(waveform.duration(), 20.0);
    assert!(waveform.peaks.len() <= MAX_BINS);
    assert_eq!(
        waveform.peaks.len() as u64,
        waveform.frames.div_ceil(waveform.frames_per_bin)
    );
    assert!(
        waveform
            .peaks
            .iter()
            .any(|peak| peak[0] == 0.0 && peak[1] > 0.6)
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
}

#[test]
fn disk_cache_survives_reopen_and_invalidates_changed_sources() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("tone.wav");
    let database = temporary.path().join("waveforms.sqlite");
    common::wave(&source, 48_000, 4800);
    let waveform = waveform::analyze(&source, Cancellation::default()).unwrap();
    let mut cache = Cache::open(&database).unwrap();
    cache.save(&source, &waveform).unwrap();
    drop(cache);
    let mut cache = Cache::open(&database).unwrap();
    let restored = cache.get(&source).unwrap().unwrap();
    assert_eq!(restored.peaks, waveform.peaks);
    common::wave(&source, 48_000, 9600);
    assert!(cache.get(&source).unwrap().is_none());
    assert!(cache.save(&source, &waveform).is_err());
}

#[test]
fn malformed_cached_peaks_are_rejected_as_rebuildable_data() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("tone.wav");
    let database = temporary.path().join("waveforms.sqlite");
    common::wave(&source, 8_000, 80);
    let waveform = waveform::analyze(&source, Cancellation::default()).unwrap();
    let mut cache = Cache::open(&database).unwrap();
    cache.save(&source, &waveform).unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute("UPDATE waveforms SET peaks=x'0000'", [])
        .unwrap();
    assert!(cache.get(&source).unwrap().is_none());
}
