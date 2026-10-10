#![cfg(all(windows, not(debug_assertions)))]
mod common;

use sift::catalog::Catalog;
use sift::library::{self, Cancellation};
use sift::search::SearchQuery;
use sift::worker::{Event, Workers};
use std::fs;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn receive_until(
    workers: &Workers,
    signals: &mpsc::Receiver<()>,
    mut predicate: impl FnMut(Event) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        while let Some(event) = workers.try_event() {
            if let Event::StartupFailed(error) = &event {
                panic!("Startup failed: {error}");
            }
            if predicate(event) {
                return;
            }
        }
        signals
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("Native change did not converge before the deadline");
    }
}

#[test]
#[ignore = "Creates 100,000 generated WAV files and measures release indexing/live updates; run explicitly"]
fn hundred_thousand_real_files_have_bounded_incremental_and_live_updates() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = workspace.join(".local").join("indexing-perf");
    fs::create_dir_all(&base).unwrap();
    assert!(
        base.canonicalize()
            .unwrap()
            .starts_with(workspace.canonicalize().unwrap())
    );
    let temporary = tempfile::tempdir_in(&base).unwrap();
    let root = temporary.path().join("generated-sounds");
    fs::create_dir(&root).unwrap();
    let create_started = Instant::now();
    for folder in 0..100 {
        let directory = root.join(format!("bank-{folder:03}"));
        fs::create_dir(&directory).unwrap();
        for file in 0..1000 {
            common::wave(&directory.join(format!("sound-{file:04}.wav")), 8_000, 64);
        }
    }
    eprintln!(
        "fixture count=100000 bytes_per_file=172 creation_ms={}",
        create_started.elapsed().as_millis()
    );
    let baseline = workspace.join("target/release/sift-before-live-index.exe");
    let mut old_times = Vec::new();
    let import_only = std::env::var_os("SIFT_INDEX_BENCH_IMPORT_ONLY").is_some();
    if baseline.is_file() && !import_only {
        for iteration in 0..3 {
            let start = Instant::now();
            let output = std::process::Command::new(&baseline)
                .args(["--database"])
                .arg(temporary.path().join("old.sqlite"))
                .arg("index")
                .arg(&root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "Baseline failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            eprintln!(
                "baseline run={iteration} elapsed_ms={} {}",
                start.elapsed().as_millis(),
                String::from_utf8_lossy(&output.stdout).trim()
            );
            old_times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let database = temporary.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&database).unwrap();
    let first_start = Instant::now();
    let first = library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    assert_eq!(first.indexed, 100_000);
    assert_eq!(first.errors, 0);
    eprintln!(
        "first_incremental_build elapsed_ms={}",
        first_start.elapsed().as_millis()
    );
    if import_only {
        return;
    }
    let mut scan_times = Vec::new();
    for iteration in 0..3 {
        let start = Instant::now();
        let report = library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
        assert_eq!(report.unchanged, 100_000);
        assert_eq!(report.indexed, 0);
        assert_eq!(report.errors, 0);
        scan_times.push(start.elapsed().as_secs_f64() * 1000.0);
        eprintln!(
            "unchanged run={iteration} elapsed_ms={} probed=0 unchanged={}",
            start.elapsed().as_millis(),
            report.unchanged
        );
    }
    let (wake, signals) = mpsc::channel();
    let startup = Instant::now();
    let workers = Workers::start(database.clone(), move || wake.send(()).unwrap()).unwrap();
    let mut ready_ms = 0.0;
    receive_until(&workers, &signals, |event| {
        if matches!(event, Event::Ready) {
            ready_ms = startup.elapsed().as_secs_f64() * 1000.0;
        }
        matches!(event, Event::SyncFinished { .. })
    });
    let mut reader = Catalog::open_reader(&database).unwrap();
    let mut latencies = Vec::new();
    let mut checked = Vec::new();
    for iteration in 0..20 {
        let start = Instant::now();
        let source = root.join(format!("new-{iteration:03}.wav"));
        common::wave(&source, 8_000, 64);
        receive_until(&workers, &signals, |event| {
            if let Event::SyncFinished { report, .. } = event
                && reader.search(&SearchQuery::All, 0).unwrap().total == 100_001 + iteration
            {
                checked.push(report.visited);
                return true;
            }
            false
        });
        latencies.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let burst = Instant::now();
    for iteration in 0..10 {
        common::wave(&root.join(format!("burst-{iteration:03}.wav")), 8_000, 64);
    }
    receive_until(&workers, &signals, |event| {
        matches!(event, Event::SyncFinished { .. })
            && reader.search(&SearchQuery::All, 0).unwrap().total == 100_030
    });
    let burst_ms = burst.elapsed().as_secs_f64() * 1000.0;
    latencies.sort_by(f64::total_cmp);
    let record = format!(
        "pid={} ready_ms={ready_ms:.3} scan_ms={scan_times:?} baseline_ms={old_times:?} live_median_ms={:.3} live_p95_ms={:.3} live_max_ms={:.3} burst10_ms={burst_ms:.3} checked_per_single_change={checked:?}\n",
        std::process::id(),
        latencies[latencies.len() / 2],
        latencies[18],
        latencies[19]
    );
    eprintln!("{record}");
    if let Some(report_path) = std::env::var_os("SIFT_INDEX_BENCH_REPORT") {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(report_path)
            .unwrap();
        file.write_all(record.as_bytes()).unwrap();
        file.flush().unwrap();
    }
    // Observe absence of spontaneous work for 60 seconds. No sleeping poll loop:
    // the process blocks on worker notifications while the host records resources.
    let idle_deadline = Instant::now() + Duration::from_secs(60);
    loop {
        while let Some(event) = workers.try_event() {
            assert!(
                !matches!(event, Event::SyncStarted),
                "Unexpected scan during the idle observation"
            );
        }
        match signals.recv_timeout(idle_deadline.saturating_duration_since(Instant::now())) {
            Ok(()) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(error) => panic!("Worker signal disconnected: {error}"),
        }
    }
    eprintln!("idle_observation seconds=60 spontaneous_scans=0");
    drop(workers);
}
