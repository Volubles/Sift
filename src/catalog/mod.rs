use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, params, params_from_iter, types::Value};

use crate::library::AudioMetadata;
use crate::search::{PAGE_SIZE, SearchQuery};
use crate::{Error, Result, path_codec};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SampleId(pub i64);

#[derive(Clone, Debug)]
pub struct Sample {
    pub id: SampleId,
    pub source: PathBuf,
    pub name: String,
    pub display_path: String,
    pub format: String,
    pub size_bytes: u64,
    pub audio: AudioMetadata,
    pub metadata_error: Option<String>,
}

#[derive(Debug)]
pub struct SearchPage {
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<Sample>,
}

pub struct IndexedFile {
    pub source: PathBuf,
    pub name: String,
    pub display_path: String,
    pub format: String,
    pub size_bytes: u64,
    pub audio: AudioMetadata,
    pub metadata_error: Option<String>,
}

pub struct Catalog {
    connection: Connection,
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 1 {
            return Err(Error::UnsupportedSchema(version));
        }
        if version == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(include_str!("schema.sql"))?;
            transaction.commit()?;
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(Self { connection })
    }

    pub fn open_reader(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != 1 {
            return Err(Error::UnsupportedSchema(version));
        }
        Ok(Self { connection })
    }

    pub fn register_root(&mut self, root: &Path) -> Result<i64> {
        let bytes = path_codec::encode(root);
        self.connection.execute(
            "INSERT INTO library_roots(source_path) VALUES (?1) ON CONFLICT DO NOTHING",
            [&bytes],
        )?;
        Ok(self.connection.query_row(
            "SELECT id FROM library_roots WHERE source_path = ?1",
            [&bytes],
            |row| row.get(0),
        )?)
    }

    pub fn upsert_batch(&mut self, root_id: i64, files: &[IndexedFile]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO samples(root_id, source_path, name, name_key, path, format, size_bytes, duration_seconds, sample_rate, channels, bit_depth, metadata_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(root_id, source_path) DO UPDATE SET
                   name=excluded.name, name_key=excluded.name_key, path=excluded.path,
                   format=excluded.format, size_bytes=excluded.size_bytes,
                   duration_seconds=excluded.duration_seconds, sample_rate=excluded.sample_rate,
                   channels=excluded.channels, bit_depth=excluded.bit_depth,
                   metadata_error=excluded.metadata_error",
            )?;
            for file in files {
                let size = i64::try_from(file.size_bytes).map_err(|_| Error::OutOfRange)?;
                statement.execute(params![
                    root_id,
                    path_codec::encode(&file.source),
                    file.name,
                    file.name.to_lowercase(),
                    file.display_path,
                    file.format,
                    size,
                    file.audio.duration_seconds,
                    file.audio.sample_rate,
                    file.audio.channels,
                    file.audio.bit_depth,
                    file.metadata_error
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn search(&mut self, query: &SearchQuery, offset: usize) -> Result<SearchPage> {
        let (condition, mut values) = match query {
            SearchQuery::All => ("1", vec![]),
            SearchQuery::FilenamePrefix { lower, upper } => (
                "s.name_key >= ?1 AND s.name_key < ?2",
                vec![Value::Text(lower.clone()), Value::Text(upper.clone())],
            ),
            SearchQuery::Substrings(expression) => (
                "s.id IN (SELECT rowid FROM samples_fts WHERE samples_fts MATCH ?1)",
                vec![Value::Text(expression.clone())],
            ),
        };
        // Count and rows share a snapshot even while the indexer commits batches.
        let transaction = self.connection.transaction()?;
        let total: i64 = transaction.query_row(
            &format!("SELECT count(*) FROM samples s WHERE {condition}"),
            params_from_iter(&values),
            |row| row.get(0),
        )?;
        let total = usize::try_from(total).map_err(|_| Error::OutOfRange)?;
        let offset = offset.min(total);
        let limit_parameter = values.len() + 1;
        let offset_parameter = values.len() + 2;
        values.push(Value::Integer(PAGE_SIZE as i64));
        values.push(Value::Integer(
            i64::try_from(offset).map_err(|_| Error::OutOfRange)?,
        ));
        let sql = format!(
            "SELECT s.id, s.source_path, s.name, s.path, s.format, s.size_bytes, s.duration_seconds, s.sample_rate, s.channels, s.bit_depth, s.metadata_error FROM samples s WHERE {condition} ORDER BY s.name_key, s.id LIMIT ?{limit_parameter} OFFSET ?{offset_parameter}"
        );
        let mut rows_out = Vec::with_capacity(PAGE_SIZE);
        {
            let mut statement = transaction.prepare(&sql)?;
            let mut rows = statement.query(params_from_iter(&values))?;
            while let Some(row) = rows.next()? {
                rows_out.push(Sample {
                    id: SampleId(row.get(0)?),
                    source: path_codec::decode(row.get(1)?)?,
                    name: row.get(2)?,
                    display_path: row.get(3)?,
                    format: row.get(4)?,
                    size_bytes: u64::try_from(row.get::<_, i64>(5)?)
                        .map_err(|_| Error::OutOfRange)?,
                    audio: AudioMetadata {
                        duration_seconds: row.get(6)?,
                        sample_rate: row.get(7)?,
                        channels: row.get(8)?,
                        bit_depth: row.get(9)?,
                    },
                    metadata_error: row.get(10)?,
                });
            }
        }
        transaction.commit()?;
        Ok(SearchPage {
            total,
            offset,
            rows: rows_out,
        })
    }
}
