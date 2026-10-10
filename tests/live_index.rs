#![cfg(windows)]
mod common;

use sift::catalog::Catalog;
use sift::library::{self, Cancellation};
use sift::search::SearchQuery;
use sift::worker::{Event, SourceRefresh, Workers};
use std::fs;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn receive_until(
    workers: &Workers,
    signals: &mpsc::Receiver<()>,
    mut predicate: impl FnMut(Event) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        while let Some(event) = workers.try_event() {
            if let Event::StartupFailed(error) = &event {
                panic!("Worker startup failed: {error}");
            }
            if predicate(event) {
                return;
            }
        }
        signals
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("Timed out waiting for a native update");
    }
}

fn wait_for_catalog(
    workers: &Workers,
    signals: &mpsc::Receiver<()>,
    reader: &mut Catalog,
    predicate: impl Fn(&mut Catalog) -> bool,
) {
    receive_until(workers, signals, |event| {
        matches!(event, Event::SyncFinished { .. }) && predicate(reader)
    });
}

#[test]
fn actual_windows_changes_update_the_catalog_and_pause_resume_catches_up() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    let initial = root.join("old.wav");
    common::wave(&initial, 8_000, 64);
    let database = temporary.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&database).unwrap();
    library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    let (wake, signals) = mpsc::channel();
    let workers = Workers::start(database.clone(), move || wake.send(()).unwrap()).unwrap();
    let mut reader = Catalog::open_reader(&database).unwrap();
    wait_for_catalog(&workers, &signals, &mut reader, |catalog| {
        catalog.search(&SearchQuery::All, 0).unwrap().total == 1
    });

    let added = root.join("new_雨.wav");
    common::wave(&added, 8_000, 64);
    wait_for_catalog(&workers, &signals, &mut reader, |catalog| {
        catalog.search(&SearchQuery::All, 0).unwrap().total == 2
    });
    let old_stamp = fs::metadata(&added).unwrap().modified().unwrap();
    common::wave(&added, 16_000, 64);
    fs::File::options()
        .write(true)
        .open(&added)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old_stamp))
        .unwrap();
    wait_for_catalog(&workers, &signals, &mut reader, |catalog| {
        catalog
            .search(&SearchQuery::parse("new_").unwrap(), 0)
            .unwrap()
            .rows
            .first()
            .is_some_and(|sound| sound.audio.sample_rate == Some(16_000))
    });

    let destination = root.join("nested").join("renamed.wav");
    fs::create_dir(destination.parent().unwrap()).unwrap();
    fs::rename(&added, &destination).unwrap();
    wait_for_catalog(&workers, &signals, &mut reader, |catalog| {
        let page = catalog.search(&SearchQuery::All, 0).unwrap();
        page.total == 2
            && page.rows.iter().any(|sound| sound.name == "renamed.wav")
            && !page.rows.iter().any(|sound| sound.name == "new_雨.wav")
    });
    fs::remove_file(&initial).unwrap();
    wait_for_catalog(&workers, &signals, &mut reader, |catalog| {
        catalog.search(&SearchQuery::All, 0).unwrap().total == 1
    });

    workers.set_live_updates(false).unwrap();
    receive_until(&workers, &signals, |event| {
        matches!(event, Event::WatchStatus { enabled: false, .. })
    });
    let missed = root.join("while_paused.wav");
    common::wave(&missed, 8_000, 64);
    assert_eq!(reader.search(&SearchQuery::All, 0).unwrap().total, 1);
    workers.set_live_updates(true).unwrap();
    wait_for_catalog(&workers, &signals, &mut reader, |catalog| {
        catalog.search(&SearchQuery::All, 0).unwrap().total == 2
    });
    let original = fs::read(&destination).unwrap();
    drop(workers);
    assert_eq!(fs::read(&destination).unwrap(), original);
}

#[test]
fn partial_media_becomes_usable_after_the_writer_finishes() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    let database = temporary.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&database).unwrap();
    library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    let (wake, signals) = mpsc::channel();
    let workers = Workers::start(database.clone(), move || wake.send(()).unwrap()).unwrap();
    let mut reader = Catalog::open_reader(&database).unwrap();
    wait_for_catalog(&workers, &signals, &mut reader, |_| true);
    let file = root.join("in_progress.wav");
    fs::write(&file, b"RIFF").unwrap();
    wait_for_catalog(&workers, &signals, &mut reader, |catalog| {
        catalog
            .search(&SearchQuery::All, 0)
            .unwrap()
            .rows
            .first()
            .is_some_and(|sound| sound.metadata_error.is_some())
    });
    common::wave(&file, 48_000, 64);
    wait_for_catalog(&workers, &signals, &mut reader, |catalog| {
        catalog
            .search(&SearchQuery::All, 0)
            .unwrap()
            .rows
            .first()
            .is_some_and(|sound| {
                sound.metadata_error.is_none() && sound.audio.sample_rate == Some(48_000)
            })
    });
}

#[test]
fn case_only_renames_do_not_leave_duplicate_windows_locators() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    let old = root.join("Sound.wav");
    common::wave(&old, 8_000, 64);
    let database = temporary.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&database).unwrap();
    library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    let (wake, signals) = mpsc::channel();
    let workers = Workers::start(database.clone(), move || wake.send(()).unwrap()).unwrap();
    let mut reader = Catalog::open_reader(&database).unwrap();
    wait_for_catalog(&workers, &signals, &mut reader, |_| true);
    fs::rename(&old, root.join("sound.wav")).unwrap();
    wait_for_catalog(&workers, &signals, &mut reader, |catalog| {
        let page = catalog.search(&SearchQuery::All, 0).unwrap();
        page.total == 1 && page.rows[0].name == "sound.wav"
    });
}

#[test]
fn native_watcher_shutdown_is_bounded_even_without_sources() {
    let temporary = tempfile::tempdir().unwrap();
    let (wake, signals) = mpsc::channel();
    let workers = Workers::start(temporary.path().join("catalog.sqlite"), move || {
        wake.send(()).unwrap()
    })
    .unwrap();
    receive_until(&workers, &signals, |event| matches!(event, Event::Ready));
    let start = Instant::now();
    drop(workers);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn selected_source_checks_report_missing_files_without_replacing_newer_selection() {
    use sift::collections::{Availability, Reference};
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    let source = root.join("selected.wav");
    common::wave(&source, 8_000, 64);
    let database = temporary.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&database).unwrap();
    library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    let reference = Reference::from(&catalog.search(&SearchQuery::All, 0).unwrap().rows[0]);
    let (wake, signals) = mpsc::channel();
    let workers = Workers::start(database, move || wake.send(()).unwrap()).unwrap();
    receive_until(&workers, &signals, |event| matches!(event, Event::Ready));
    workers.set_live_updates(false).unwrap();
    receive_until(&workers, &signals, |event| {
        matches!(event, Event::WatchStatus { enabled: false, .. })
    });
    assert!(workers.check_source(1, reference.clone(), SourceRefresh::Verify));
    receive_until(&workers, &signals, |event| {
        if let Event::SourceReady {
            generation: 1,
            stamp,
            availability,
            ..
        } = event
        {
            assert!(stamp.is_some());
            assert_eq!(availability, Availability::Ready);
            true
        } else {
            false
        }
    });
    fs::remove_file(&source).unwrap();
    assert!(workers.check_source(2, reference, SourceRefresh::ObservedWrite));
    receive_until(&workers, &signals, |event| {
        if let Event::SourceReady {
            generation: 2,
            availability,
            ..
        } = event
        {
            assert_eq!(availability, Availability::Missing);
            true
        } else {
            false
        }
    });
}
