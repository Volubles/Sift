mod common;

use sift::{
    catalog::Catalog,
    collections::{self, Availability, Error, Reference, Store},
    library::{self, Cancellation},
    search::SearchQuery,
};

fn fixture(directory: &std::path::Path, name: &str) -> Reference {
    let path = directory.join(name);
    common::wave(&path, 8000, 16);
    Reference {
        source: path.canonicalize().unwrap(),
        name: name.into(),
        display_path: name.into(),
        format: "wav".into(),
        size_bytes: std::fs::metadata(&path).unwrap().len(),
        audio: library::AudioMetadata {
            sample_rate: Some(8000),
            duration_seconds: Some(0.002),
            channels: Some(1),
            bit_depth: Some(16),
        },
        metadata_error: None,
    }
}

#[test]
fn collections_survive_restart_index_rebuild_and_preserve_originals() {
    let directory = tempfile::tempdir().unwrap();
    let sounds = directory.path().join("sounds");
    std::fs::create_dir(&sounds).unwrap();
    let reference = fixture(&sounds, "雨 space.wav");
    let original = std::fs::read(&reference.source).unwrap();
    let catalog_path = directory.path().join("catalog.sqlite");
    let durable = collections::data_path(&catalog_path);
    let mut catalog = Catalog::open(&catalog_path).unwrap();
    library::scan(&sounds, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    let sample = catalog.search(&SearchQuery::All, 0).unwrap().rows.remove(0);
    let mut store = Store::open(&durable).unwrap();
    let first = store.create("Song 1").unwrap();
    let second = store.create("Footsteps").unwrap();
    store.add(first, &[Reference::from(&sample)]).unwrap();
    store.add(second, std::slice::from_ref(&reference)).unwrap();
    store.add(first, std::slice::from_ref(&reference)).unwrap();
    let entry = store.entries(first, 0).unwrap().rows.remove(0);
    store.rename(first, "Film 1").unwrap();
    drop(store);
    drop(catalog);
    // Rebuild only the disposable index. Durable collection IDs/paths do not
    // depend on the catalog row IDs and never cascade with index deletion.
    std::fs::remove_file(&catalog_path).unwrap();
    let mut rebuilt = Catalog::open(&catalog_path).unwrap();
    library::scan(&sounds, &mut rebuilt, &Cancellation::default(), |_| {}).unwrap();
    let mut store = Store::open(&durable).unwrap();
    assert_eq!(store.list(0).unwrap().rows[0].name, "Film 1");
    let restored = store.entries(first, 0).unwrap();
    assert_eq!(restored.total, 1);
    assert_eq!(restored.rows[0].id, entry.id);
    assert_eq!(restored.rows[0].reference.source, reference.source);
    assert_eq!(restored.rows[0].availability, Availability::Ready);
    store.remove(first, entry.id).unwrap();
    assert_eq!(store.entries(second, 0).unwrap().total, 1);
    store.delete(first).unwrap();
    assert!(store.create("Replacement").unwrap().0 > first.0);
    assert_eq!(std::fs::read(&reference.source).unwrap(), original);
}

#[test]
fn missing_changed_and_unavailable_sources_keep_their_references() {
    let directory = tempfile::tempdir().unwrap();
    let reference = fixture(directory.path(), "sound.wav");
    let mut store = Store::open(&directory.path().join("collections.sqlite")).unwrap();
    let collection = store.create("Effects").unwrap();
    store
        .add(collection, std::slice::from_ref(&reference))
        .unwrap();
    let id = store.entries(collection, 0).unwrap().rows[0].id;
    std::fs::write(&reference.source, b"changed fixture bytes").unwrap();
    assert_eq!(
        store.entries(collection, 0).unwrap().rows[0].availability,
        Availability::Changed
    );
    std::fs::remove_file(&reference.source).unwrap();
    assert_eq!(
        store.entries(collection, 0).unwrap().rows[0].availability,
        Availability::Missing
    );
    std::fs::create_dir(&reference.source).unwrap();
    let entry = store.entries(collection, 0).unwrap().rows.remove(0);
    assert!(matches!(entry.availability, Availability::Unavailable(_)));
    assert_eq!(entry.id, id);
}

#[test]
fn batch_failure_rolls_back_all_new_references() {
    let directory = tempfile::tempdir().unwrap();
    let references = [
        fixture(directory.path(), "first.wav"),
        fixture(directory.path(), "blocked.wav"),
    ];
    let path = directory.path().join("collections.sqlite");
    let mut store = Store::open(&path).unwrap();
    let collection = store.create("Batch").unwrap();
    rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TRIGGER reject_fixture BEFORE INSERT ON entries WHEN new.name='blocked.wav' BEGIN SELECT RAISE(ABORT,'fixture rejection'); END;").unwrap();
    assert!(store.add(collection, &references).is_err());
    assert_eq!(store.entries(collection, 0).unwrap().total, 0);
}

#[test]
fn names_versions_and_paging_are_validated_without_resetting_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("collections.sqlite");
    let mut store = Store::open(&path).unwrap();
    assert!(matches!(store.create(" \n "), Err(Error::InvalidName)));
    let id = store.create("  Film 1  ").unwrap();
    assert!(matches!(store.create("film 1"), Err(Error::NameConflict)));
    store.rename(id, "Film 1").unwrap();
    for index in 0..130 {
        store.create(&format!("Collection {index}")).unwrap();
    }
    assert_eq!(store.list(0).unwrap().rows.len(), collections::PAGE_SIZE);
    assert_eq!(store.list(collections::PAGE_SIZE).unwrap().rows.len(), 3);
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);
    assert!(matches!(
        Store::open(&path),
        Err(Error::UnsupportedSchema(99))
    ));
    assert_eq!(
        rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("SELECT count(*) FROM collections", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        131
    );
}
