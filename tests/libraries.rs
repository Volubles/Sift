use std::fs;

use sift::catalog::{Catalog, IndexedFile, Scope};
use sift::libraries::{self, Availability, Store};
use sift::library::AudioMetadata;
use sift::search::SearchQuery;

fn indexed(source: std::path::PathBuf) -> IndexedFile {
    let name = source.file_name().unwrap().to_string_lossy().into_owned();
    IndexedFile {
        source,
        display_path: name.clone(),
        name,
        format: "wav".into(),
        size_bytes: 44,
        audio: AudioMetadata::default(),
        metadata_error: None,
    }
}

#[test]
fn overlapping_roots_and_component_scopes_preserve_one_sound_and_stable_id() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    let child = root.join("雨");
    let neighbor = root.join("雨-other");
    let mut catalog = Catalog::open(&temporary.path().join("catalog.sqlite")).unwrap();
    let parent_id = catalog.register_root(&root).unwrap();
    catalog
        .upsert_batch(
            parent_id,
            &[
                indexed(child.join("door.wav")),
                indexed(child.join("nested").join("door-two.wav")),
                indexed(neighbor.join("door-three.wav")),
            ],
        )
        .unwrap();
    let before = catalog
        .search_scoped(&SearchQuery::All, &Scope::Folders(vec![child.clone()]), 0)
        .unwrap();
    assert_eq!(before.total, 2);
    let child_id = catalog.register_root(&child).unwrap();
    catalog
        .upsert_batch(child_id, &[indexed(child.join("door.wav"))])
        .unwrap();
    let after = catalog
        .search_scoped(
            &SearchQuery::parse("door").unwrap(),
            &Scope::Folders(vec![root.clone(), child.clone(), root.clone()]),
            0,
        )
        .unwrap();
    assert_eq!(after.total, 3);
    assert_eq!(
        after
            .rows
            .iter()
            .find(|row| row.name == "door.wav")
            .unwrap()
            .id,
        before
            .rows
            .iter()
            .find(|row| row.name == "door.wav")
            .unwrap()
            .id
    );
    assert_eq!(
        catalog
            .count_scope(&Scope::Folders(vec![child.clone()]))
            .unwrap(),
        2
    );
    assert_eq!(catalog.count_scope(&Scope::Folders(Vec::new())).unwrap(), 0);
    let folders = catalog.folders(&root, 0).unwrap();
    assert_eq!(folders.total, 2);
    assert!(
        folders
            .rows
            .iter()
            .find(|folder| folder.source == child)
            .unwrap()
            .has_children
    );
}

#[test]
fn folder_children_are_bounded_and_pages_do_not_overlap() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    let mut catalog = Catalog::open(&temporary.path().join("catalog.sqlite")).unwrap();
    let id = catalog.register_root(&root).unwrap();
    let files: Vec<_> = (0..130)
        .map(|index| indexed(root.join(format!("folder-{index:03}")).join("sound.wav")))
        .collect();
    catalog.upsert_batch(id, &files).unwrap();
    let first = catalog.folders(&root, 0).unwrap();
    let next = catalog.folders(&root, 64).unwrap();
    assert_eq!(first.total, 130);
    assert_eq!(first.rows.len(), 64);
    assert_eq!(next.rows.len(), 64);
    assert_eq!(first.rows[0].name, "folder-000");
    assert_eq!(next.rows[0].name, "folder-064");
    assert_eq!(catalog.folders(&root, 128).unwrap().rows.len(), 2);
}

#[test]
fn bank_registration_survives_restart_rebuild_offline_sources_and_removal() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    let file = root.join("original.wav");
    fs::write(&file, b"original bytes").unwrap();
    let catalog_path = temporary.path().join("catalog.sqlite");
    let registry_path = libraries::data_path(&catalog_path);
    let catalog = Catalog::open(&catalog_path).unwrap();
    let mut store = Store::open(&registry_path).unwrap();
    let id = store.create("Sound effects").unwrap();
    let shared = store.create("Film").unwrap();
    let canonical = store.attach(id, &root).unwrap();
    store.attach(id, &root).unwrap();
    store.attach(shared, &root).unwrap();
    store.rename(id, "Game audio").unwrap();
    assert!(matches!(
        store.create("GAME AUDIO"),
        Err(libraries::Error::NameConflict)
    ));
    assert!(matches!(
        store.create(" \n "),
        Err(libraries::Error::InvalidName)
    ));
    assert_eq!(
        store
            .snapshot(&catalog)
            .unwrap()
            .iter()
            .find(|bank| bank.id == id)
            .unwrap()
            .sources
            .len(),
        1
    );
    drop(catalog);
    drop(store);
    fs::remove_file(&catalog_path).unwrap();
    let catalog = Catalog::open(&catalog_path).unwrap();
    let mut store = Store::open(&registry_path).unwrap();
    let snapshot = store.snapshot(&catalog).unwrap();
    let bank = snapshot.iter().find(|bank| bank.id == id).unwrap();
    assert_eq!(bank.sources[0].path, canonical);
    assert_eq!(bank.count, 0);
    assert_eq!(bank.sources[0].availability, Availability::Online);
    store.detach(id, bank.sources[0].id).unwrap();
    store.remove(id).unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"original bytes");
    fs::rename(&root, temporary.path().join("unmounted")).unwrap();
    assert_eq!(
        store.snapshot(&catalog).unwrap()[0].sources[0].availability,
        Availability::Offline
    );
}

#[test]
fn legacy_registrations_import_once_without_resurrecting_removed_banks() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("missing").join("Audio");
    let second = temporary.path().join("other").join("Audio");
    let catalog = Catalog::open(&temporary.path().join("catalog.sqlite")).unwrap();
    let path = temporary.path().join("libraries.sqlite");
    let mut store = Store::open(&path).unwrap();
    store
        .import_legacy(&[root.clone(), second.clone()])
        .unwrap();
    let snapshot = store.snapshot(&catalog).unwrap();
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot[0].name, "Audio");
    assert_eq!(snapshot[1].name, "Audio (2)");
    store.remove(snapshot[0].id).unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    store.import_legacy(&[root, second]).unwrap();
    assert_eq!(store.snapshot(&catalog).unwrap().len(), 1);
}

#[test]
fn unknown_library_database_is_preserved() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("libraries.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TABLE important(value TEXT); INSERT INTO important VALUES('keep'); PRAGMA user_version=99;").unwrap();
    assert!(matches!(
        Store::open(&path),
        Err(libraries::Error::UnsupportedSchema(99))
    ));
    assert_eq!(
        connection
            .query_row("SELECT value FROM important", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}

#[test]
fn version_one_migration_deduplicates_and_preserves_fts_and_native_tree() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("catalog.sqlite");
    let root = temporary.path().join("sounds");
    let source = root.join("nested").join("door.wav");
    let mut connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../src/catalog/schema.sql"))
        .unwrap();
    let encode = |path: &std::path::Path| -> Vec<u8> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            path.as_os_str()
                .encode_wide()
                .flat_map(u16::to_le_bytes)
                .collect()
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            path.as_os_str().as_bytes().to_vec()
        }
    };
    for id in 1..=2 {
        connection
            .execute(
                "INSERT INTO library_roots(id,source_path) VALUES(?1,?2)",
                rusqlite::params![
                    id,
                    encode(&if id == 1 {
                        root.clone()
                    } else {
                        root.join("nested")
                    })
                ],
            )
            .unwrap();
        connection.execute("INSERT INTO samples(id,root_id,source_path,name,name_key,path,format,size_bytes) VALUES(?1,?1,?2,'door.wav','door.wav','door.wav','wav',44)", rusqlite::params![id, encode(&source)]).unwrap();
    }
    drop(connection);
    let mut catalog = Catalog::open(&path).unwrap();
    let page = catalog
        .search(&SearchQuery::parse("door").unwrap(), 0)
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].id.0, 1);
    assert_eq!(page.rows[0].source, source);
    assert_eq!(catalog.folders(&root, 0).unwrap().rows[0].name, "nested");
    drop(catalog);
    connection = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[cfg(windows)]
#[test]
fn invalid_locator_rolls_back_version_one_migration() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("catalog.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../src/catalog/schema.sql"))
        .unwrap();
    connection.execute_batch("INSERT INTO library_roots VALUES(1,x'01'); INSERT INTO samples(id,root_id,source_path,name,name_key,path,format,size_bytes) VALUES(1,1,x'01','keep','keep','keep','wav',44);").unwrap();
    assert!(matches!(
        Catalog::open(&path),
        Err(sift::Error::InvalidPath)
    ));
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM samples", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
