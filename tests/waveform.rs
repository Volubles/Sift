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

#[test]
fn observed_write_can_rebuild_peaks_even_when_size_and_timestamp_were_preserved() {
    use std::sync::mpsc;
    use std::time::Duration;
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("tone.wav");
    common::wave(&source, 8_000, 800);
    let stamp = std::fs::metadata(&source).unwrap().modified().unwrap();
    let (wake, signals) = mpsc::channel();
    let mut worker =
        waveform::Waveforms::start(temporary.path().join("catalog.sqlite"), move || {
            wake.send(()).unwrap()
        })
        .unwrap();
    worker.request(1, source.clone()).unwrap();
    signals.recv_timeout(Duration::from_secs(3)).unwrap();
    let original = worker.poll().unwrap().result.unwrap();
    assert!(original.peaks.iter().any(|peak| peak[1] > 0.6));
    let mut bytes = std::fs::read(&source).unwrap();
    bytes[44..].fill(0);
    std::fs::write(&source, bytes).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(stamp))
        .unwrap();
    worker.rebuild(2, source.clone()).unwrap();
    signals.recv_timeout(Duration::from_secs(3)).unwrap();
    let fresh = worker.poll().unwrap();
    assert!(!fresh.cache_hit);
    assert!(
        fresh
            .result
            .unwrap()
            .peaks
            .iter()
            .all(|peak| *peak == [0.0, 0.0])
    );
}
