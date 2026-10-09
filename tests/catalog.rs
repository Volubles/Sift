use std::fs;
use std::path::Path;

use sift::catalog::{Catalog, IndexedFile};
use sift::library::{self, AudioMetadata, Cancellation};
use sift::search::{PAGE_SIZE, SearchQuery};

fn write_wave(path: &Path, sample_rate: u32, frames: u32) {
    let data_bytes = frames * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    bytes.resize(bytes.len() + data_bytes as usize, 0);
    fs::write(path, bytes).unwrap();
}

fn indexed_file(name: &str) -> IndexedFile {
    IndexedFile {
        source: Path::new("synthetic").join(name),
        name: name.into(),
        display_path: name.into(),
        format: "wav".into(),
        size_bytes: 44,
        audio: AudioMetadata::default(),
        metadata_error: None,
    }
}

#[test]
fn scans_real_wave_metadata_without_changing_sources() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    let sound = root.join("Forest_雨.WAV");
    write_wave(&sound, 48_000, 4_800);
    let original = fs::read(&sound).unwrap();
    let database = temporary.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&database).unwrap();
    let report = library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    assert_eq!(report.indexed, 1);
    assert_eq!(report.errors, 0);
    let page = catalog
        .search(&SearchQuery::parse("forest").unwrap(), 0)
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].audio.sample_rate, Some(48_000));
    assert_eq!(page.rows[0].audio.channels, Some(1));
    assert_eq!(page.rows[0].audio.bit_depth, Some(16));
    assert!((page.rows[0].audio.duration_seconds.unwrap() - 0.1).abs() < 1e-9);
    assert_eq!(fs::read(&sound).unwrap(), original);
    let id = page.rows[0].id;
    drop(catalog);

    let mut catalog = Catalog::open(&database).unwrap();
    library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    let page = catalog.search(&SearchQuery::All, 0).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].id, id);
    assert_eq!(page.rows[0].source, sound.canonicalize().unwrap());
}

#[test]
fn malformed_audio_is_a_visible_record_and_does_not_abort_other_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("bad.wav"), b"not a wave").unwrap();
    write_wave(&root.join("good.wav"), 44_100, 44);
    fs::write(root.join("notes.txt"), b"ignored").unwrap();
    let mut catalog = Catalog::open(&temporary.path().join("catalog.sqlite")).unwrap();
    let report = library::scan(&root, &mut catalog, &Cancellation::default(), |_| {}).unwrap();
    assert_eq!(report.indexed, 2);
    assert_eq!(report.errors, 1);
    let page = catalog.search(&SearchQuery::All, 0).unwrap();
    assert!(page.rows[0].metadata_error.is_some());
    assert_eq!(page.rows[0].audio.duration_seconds, None);
    assert!(page.rows[1].metadata_error.is_none());
}

#[test]
fn cancellation_keeps_committed_batches_and_stops_further_work() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("sounds");
    fs::create_dir(&root).unwrap();
    for index in 0..80 {
        write_wave(&root.join(format!("tone_{index:03}.wav")), 8_000, 16);
    }
    let mut catalog = Catalog::open(&temporary.path().join("catalog.sqlite")).unwrap();
    let cancellation = Cancellation::default();
    let report = library::scan(&root, &mut catalog, &cancellation, |progress| {
        if progress.indexed >= 64 {
            cancellation.cancel();
        }
    })
    .unwrap();
    assert!(report.cancelled);
    assert_eq!(report.indexed, 64);
    assert_eq!(catalog.search(&SearchQuery::All, 0).unwrap().total, 64);
}

#[test]
fn indexed_search_handles_punctuation_unicode_and_short_prefixes() {
    let temporary = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(&temporary.path().join("catalog.sqlite")).unwrap();
    let root = catalog.register_root(Path::new("synthetic")).unwrap();
    catalog
        .upsert_batch(
            root,
            &[
                indexed_file("door_wood_001.wav"),
                indexed_file("door-metal.wav"),
                indexed_file("Älpha.wav"),
                indexed_file("door_OR*literal.wav"),
            ],
        )
        .unwrap();
    assert_eq!(
        catalog
            .search(&SearchQuery::parse("door wood").unwrap(), 0)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        catalog
            .search(&SearchQuery::parse("door-metal").unwrap(), 0)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        catalog
            .search(&SearchQuery::parse("Äl").unwrap(), 0)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        catalog
            .search(&SearchQuery::parse("OR*").unwrap(), 0)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        catalog
            .search(&SearchQuery::parse("do").unwrap(), 0)
            .unwrap()
            .total,
        3
    );
}

#[test]
fn pages_are_bounded_stably_sorted_and_visible_to_a_separate_reader() {
    let temporary = tempfile::tempdir().unwrap();
    let database = temporary.path().join("catalog.sqlite");
    let mut writer = Catalog::open(&database).unwrap();
    let root = writer.register_root(Path::new("synthetic")).unwrap();
    let mut reader = Catalog::open_reader(&database).unwrap();
    let files: Vec<_> = (0..300)
        .rev()
        .map(|index| indexed_file(&format!("tone_{index:03}.wav")))
        .collect();
    writer.upsert_batch(root, &files).unwrap();
    let first = reader.search(&SearchQuery::All, 0).unwrap();
    let second = reader.search(&SearchQuery::All, PAGE_SIZE).unwrap();
    assert_eq!(first.total, 300);
    assert_eq!(first.rows.len(), PAGE_SIZE);
    assert_eq!(first.rows[0].name, "tone_000.wav");
    assert_eq!(second.rows[0].name, "tone_128.wav");
    assert_ne!(first.rows.last().unwrap().id, second.rows[0].id);
    let beyond_end = reader.search(&SearchQuery::All, 1024).unwrap();
    assert_eq!(beyond_end.offset, 1024);
    assert!(beyond_end.rows.is_empty());
}

#[test]
fn failed_batch_rolls_back_its_rows_and_fts_entries() {
    let temporary = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(&temporary.path().join("catalog.sqlite")).unwrap();
    let root = catalog.register_root(Path::new("synthetic")).unwrap();
    let mut overflow = indexed_file("overflow.wav");
    overflow.size_bytes = u64::MAX;
    assert!(
        catalog
            .upsert_batch(root, &[indexed_file("first.wav"), overflow])
            .is_err()
    );
    assert_eq!(catalog.search(&SearchQuery::All, 0).unwrap().total, 0);
    assert_eq!(
        catalog
            .search(&SearchQuery::parse("first").unwrap(), 0)
            .unwrap()
            .total,
        0
    );
}

#[test]
fn newer_schema_is_rejected_without_resetting_existing_data() {
    let temporary = tempfile::tempdir().unwrap();
    let database = temporary.path().join("catalog.sqlite");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TABLE user_data(value TEXT); INSERT INTO user_data VALUES('keep'); PRAGMA user_version = 99;").unwrap();
    drop(connection);
    assert!(matches!(
        Catalog::open(&database),
        Err(sift::Error::UnsupportedSchema(99))
    ));
    let connection = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT value FROM user_data", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}
