mod common;

use sift::{
    catalog::Catalog,
    collections::{self, Availability, Error, Reference, Store, favorites::MAX_STATUS_PATHS},
    library::{self, Cancellation},
    search::SearchQuery,
};

fn fixture(directory: &std::path::Path, name: &str) -> Reference {
    let path = directory.join(name);
    common::wave(&path, 8000, 16);
    Reference {
        source: path.canonicalize().unwrap(),
        name: name.into(),
        display_path: format!("Effects/{name}"),
        format: "wav".into(),
        size_bytes: std::fs::metadata(&path).unwrap().len(),
        audio: Default::default(),
        metadata_error: None,
    }
}

#[test]
fn favorites_survive_restart_index_rebuild_and_collection_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let sounds = directory.path().join("sounds");
    std::fs::create_dir(&sounds).unwrap();
    let reference = fixture(&sounds, "雨 space.wav");
    let original = std::fs::read(&reference.source).unwrap();
    let catalog_path = directory.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&catalog_path).unwrap();
    library::scan(&sounds, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    let sample = catalog.search(&SearchQuery::All, 0).unwrap().rows.remove(0);
    let durable = collections::data_path(&catalog_path);
    let mut store = Store::open(&durable).unwrap();
    let collection = store.create("Effects").unwrap();
    store.add(collection, &[Reference::from(&sample)]).unwrap();
    store.favorite(&Reference::from(&sample)).unwrap();
    let id = store.favorites(&SearchQuery::All, 0).unwrap().rows[0].id;
    store.favorite(&reference).unwrap();
    assert_eq!(store.favorites(&SearchQuery::All, 0).unwrap().total, 1);
    store.delete(collection).unwrap();
    drop(store);
    drop(catalog);
    std::fs::remove_file(&catalog_path).unwrap();
    let mut catalog = Catalog::open(&catalog_path).unwrap();
    library::scan(&sounds, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    let mut store = Store::open(&durable).unwrap();
    assert_eq!(
        store.favorites(&SearchQuery::All, 0).unwrap().rows[0].id,
        id
    );
    assert_eq!(
        store
            .favorite_status(std::slice::from_ref(&reference.source))
            .unwrap()[0],
        (reference.source.clone(), true)
    );
    store.unfavorite(&reference.source).unwrap();
    store.unfavorite(&reference.source).unwrap();
    store.favorite(&reference).unwrap();
    assert!(store.favorites(&SearchQuery::All, 0).unwrap().rows[0].id.0 > id.0);
    assert_eq!(std::fs::read(&reference.source).unwrap(), original);
}

#[test]
fn search_is_indexed_literal_and_paged_with_stable_order() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(&directory.path().join("saved.sqlite")).unwrap();
    for index in (0..130).rev() {
        store
            .favorite(&fixture(directory.path(), &format!("Door {index:03}.wav")))
            .unwrap();
    }
    let page = store
        .favorites(&SearchQuery::parse("Do").unwrap(), 0)
        .unwrap();
    assert_eq!(page.total, 130);
    assert_eq!(page.rows.len(), 128);
    assert_eq!(page.rows[0].reference.name, "Door 000.wav");
    assert_eq!(
        store.favorites(&SearchQuery::All, 128).unwrap().rows.len(),
        2
    );
    store
        .favorite(&fixture(directory.path(), "Metal NOT.wav"))
        .unwrap();
    assert_eq!(
        store
            .favorites(&SearchQuery::parse("metal NOT").unwrap(), 0)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        store
            .favorites(&SearchQuery::parse("effects door").unwrap(), 0)
            .unwrap()
            .total,
        130
    );
    let other = directory.path().join("absent.wav");
    assert_eq!(
        store
            .favorite_status(&[page.rows[0].reference.source.clone(), other])
            .unwrap()
            .iter()
            .map(|(_, saved)| *saved)
            .collect::<Vec<_>>(),
        [true, false]
    );
}

#[test]
fn changed_missing_and_unavailable_favorites_are_retained_and_removable() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(&directory.path().join("saved.sqlite")).unwrap();
    let reference = fixture(directory.path(), "sound.wav");
    store.favorite(&reference).unwrap();
    std::fs::write(&reference.source, b"changed").unwrap();
    assert_eq!(
        store.favorites(&SearchQuery::All, 0).unwrap().rows[0].availability,
        Availability::Changed
    );
    std::fs::remove_file(&reference.source).unwrap();
    assert_eq!(
        store.favorites(&SearchQuery::All, 0).unwrap().rows[0].availability,
        Availability::Missing
    );
    std::fs::create_dir(&reference.source).unwrap();
    assert!(matches!(
        store.favorites(&SearchQuery::All, 0).unwrap().rows[0].availability,
        Availability::Unavailable(_)
    ));
    store.unfavorite(&reference.source).unwrap();
    assert_eq!(store.favorites(&SearchQuery::All, 0).unwrap().total, 0);
}

#[test]
fn schema_one_migration_preserves_saved_ids_and_rolls_back_on_failure() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../src/collections/schema.sql"))
        .unwrap();
    connection
        .execute(
            "INSERT INTO collections(id,name,name_key) VALUES(7,'Film','film')",
            [],
        )
        .unwrap();
    drop(connection);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.list(0).unwrap().rows[0].id.0, 7);
    let reference = fixture(directory.path(), "sound.wav");
    store
        .add(collections::CollectionId(7), &[reference])
        .unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("DROP TABLE favorites; PRAGMA user_version=1;")
        .unwrap();
    // A conflicting object makes migration fail. It must not reset collections,
    // delete saved entries, or advance the schema version.
    connection
        .execute_batch("CREATE TABLE favorites(blocked INTEGER);")
        .unwrap();
    drop(connection);
    assert!(Store::open(&path).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM entries WHERE collection_id=7",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn failed_save_and_invalid_status_requests_preserve_existing_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.sqlite");
    let mut store = Store::open(&path).unwrap();
    let reference = fixture(directory.path(), "sound.wav");
    rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TRIGGER reject_favorite BEFORE INSERT ON favorites BEGIN SELECT RAISE(ABORT,'fixture rejection'); END;").unwrap();
    assert!(store.favorite(&reference).is_err());
    assert_eq!(store.favorites(&SearchQuery::All, 0).unwrap().total, 0);
    assert!(matches!(
        store.favorite_status(&vec![reference.source; MAX_STATUS_PATHS + 1]),
        Err(Error::InvalidStatusBatch)
    ));
    assert!(matches!(
        store.favorite_status(&["relative.wav".into()]),
        Err(Error::InvalidReference)
    ));
}
