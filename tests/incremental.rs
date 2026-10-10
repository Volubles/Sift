mod common;

use sift::catalog::Catalog;
use sift::collections::{Availability, Reference, Store};
use sift::library::{self, Cancellation};
use sift::search::SearchQuery;
use std::fs;

#[test]
fn unchanged_inventory_does_not_reprobe_or_write_the_main_database() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    for index in 0..130 {
        common::wave(&root.join(format!("sound-{index:03}.wav")), 8_000, 64);
    }
    fs::write(root.join("corrupt.wav"), b"known invalid media").unwrap();
    let path = temporary.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&path).unwrap();
    let first = library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    assert_eq!(first.indexed, 131);
    assert_eq!(first.errors, 1);
    let ids: Vec<_> = catalog
        .search(&SearchQuery::All, 0)
        .unwrap()
        .rows
        .into_iter()
        .map(|sample| sample.id)
        .collect();
    drop(catalog);
    let observer = rusqlite::Connection::open(&path).unwrap();
    let version = || {
        observer
            .pragma_query_value(None, "data_version", |row| row.get::<_, i64>(0))
            .unwrap()
    };
    let mut catalog = Catalog::open(&path).unwrap();
    let before = version();
    let second = library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    assert_eq!(second.visited, 131);
    assert_eq!(second.unchanged, 131);
    assert_eq!(second.indexed, 0);
    assert_eq!(second.errors, 0);
    assert_eq!(version(), before);
    assert_eq!(
        catalog
            .search(&SearchQuery::All, 0)
            .unwrap()
            .rows
            .into_iter()
            .map(|sample| sample.id)
            .collect::<Vec<_>>(),
        ids
    );
}

#[test]
fn additions_writes_renames_and_deletions_reconcile_without_losing_saved_work() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    let source = root.join("old.wav");
    common::wave(&source, 8_000, 64);
    let mut catalog = Catalog::open(&temporary.path().join("catalog.sqlite")).unwrap();
    library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    let sound = catalog.search(&SearchQuery::All, 0).unwrap().rows.remove(0);
    let mut saved = Store::open(&temporary.path().join("collections.sqlite")).unwrap();
    let collection = saved.create("Retained reference").unwrap();
    saved.add(collection, &[Reference::from(&sound)]).unwrap();
    let stamp = fs::metadata(&source).unwrap().modified().unwrap();
    common::wave(&source, 16_000, 64);
    fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(stamp))
        .unwrap();
    let changed =
        library::synchronize_path(&root, &source, &mut catalog, &Cancellation::default()).unwrap();
    assert_eq!(changed.indexed, 1);
    assert_eq!(
        catalog.search(&SearchQuery::All, 0).unwrap().rows[0]
            .audio
            .sample_rate,
        Some(16_000)
    );
    let destination = root.join("nested").join("new.wav");
    fs::create_dir(destination.parent().unwrap()).unwrap();
    fs::rename(&source, &destination).unwrap();
    assert_eq!(
        library::synchronize_path(&root, &source, &mut catalog, &Cancellation::default())
            .unwrap()
            .removed,
        1
    );
    assert_eq!(
        library::synchronize_path(
            &root,
            destination.parent().unwrap(),
            &mut catalog,
            &Cancellation::default()
        )
        .unwrap()
        .indexed,
        1
    );
    let page = catalog.search(&SearchQuery::All, 0).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].source, destination.canonicalize().unwrap());
    assert!(page.rows[0].id.0 > sound.id.0);
    assert_eq!(
        saved.entries(collection, 0).unwrap().rows[0].availability,
        Availability::Missing
    );
    assert_eq!(fs::read(&destination).unwrap().len(), 172);
    fs::remove_file(&destination).unwrap();
    assert_eq!(
        library::scan(&root, &mut catalog, &Cancellation::default(), |_| {})
            .unwrap()
            .removed,
        1
    );
    assert_eq!(catalog.search(&SearchQuery::All, 0).unwrap().total, 0);
    assert!(
        catalog
            .folders(&root.canonicalize().unwrap(), 0)
            .unwrap()
            .rows
            .is_empty()
    );
    assert_eq!(saved.entries(collection, 0).unwrap().total, 1);
}

#[test]
fn cancelled_or_incomplete_enumeration_does_not_delete_cached_samples() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    for index in 0..70 {
        common::wave(&root.join(format!("sound-{index:03}.wav")), 8_000, 64);
    }
    let mut catalog = Catalog::open(&temporary.path().join("catalog.sqlite")).unwrap();
    library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    fs::remove_file(root.join("sound-000.wav")).unwrap();
    let cancellation = Cancellation::default();
    let report = library::scan(&root, &mut catalog, &cancellation, |progress| {
        if progress.visited >= 64 {
            cancellation.cancel();
        }
    })
    .unwrap();
    assert!(report.cancelled);
    assert_eq!(catalog.search(&SearchQuery::All, 0).unwrap().total, 70);
    let deep = (0..65).fold(root.clone(), |path, _| path.join("d"));
    fs::create_dir_all(&deep).unwrap();
    common::wave(&deep.join("deep.wav"), 8_000, 64);
    let report = library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    assert!(report.incomplete);
    assert_eq!(catalog.search(&SearchQuery::All, 0).unwrap().total, 70);
    let offline = temporary.path().join("offline");
    fs::rename(&root, &offline).unwrap();
    assert!(library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).is_err());
    assert_eq!(catalog.search(&SearchQuery::All, 0).unwrap().total, 70);
}

#[test]
fn path_updates_cannot_escape_the_registered_root() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    let outside = temporary.path().join("outside.wav");
    common::wave(&outside, 8_000, 64);
    let original = fs::read(&outside).unwrap();
    let mut catalog = Catalog::open(&temporary.path().join("catalog.sqlite")).unwrap();
    assert!(
        library::synchronize_path(&root, &outside, &mut catalog, &Cancellation::default()).is_err()
    );
    assert!(
        library::synchronize_path(
            &root,
            &root.join("..").join("outside.wav"),
            &mut catalog,
            &Cancellation::default()
        )
        .is_err()
    );
    assert_eq!(catalog.search(&SearchQuery::All, 0).unwrap().total, 0);
    assert_eq!(fs::read(&outside).unwrap(), original);
}
