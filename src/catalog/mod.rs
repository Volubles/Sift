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

/// Native directory ranges; display text is never used to locate or filter files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    All,
    Folders(Vec<PathBuf>),
}

#[derive(Clone, Debug)]
pub struct Folder {
    pub source: PathBuf,
    pub name: String,
    pub has_children: bool,
}

#[derive(Debug)]
pub struct FolderPage {
    pub parent: PathBuf,
    pub offset: usize,
    pub total: usize,
    pub rows: Vec<Folder>,
}

pub const FOLDER_PAGE_SIZE: usize = 64;

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
        if version > 2 {
            return Err(Error::UnsupportedSchema(version));
        }
        if version == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(include_str!("schema.sql"))?;
            transaction.commit()?;
        }
        if version <= 1 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(include_str!("migration-2.sql"))?;
            // Populate in bounded pages. A malformed locator rolls back the entire
            // migration, including duplicate removal and the schema version.
            let mut after = 0i64;
            loop {
                let files: Vec<(i64, Vec<u8>, Vec<u8>)> = {
                    let mut statement = transaction.prepare(
                        "SELECT s.id, s.source_path, r.source_path FROM samples s
                         JOIN library_roots r ON r.id=s.root_id WHERE s.id>?1 ORDER BY s.id LIMIT 128",
                    )?;
                    statement
                        .query_map([after], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                        .collect::<std::result::Result<_, _>>()?
                };
                if files.is_empty() {
                    break;
                }
                for (id, source, root) in files {
                    record_folders(
                        &transaction,
                        &path_codec::decode(source)?,
                        &path_codec::decode(root)?,
                    )?;
                    after = id;
                }
            }
            // Build every original root's hierarchy before discarding duplicate
            // rows: the oldest record may belong to a deeply nested registration.
            transaction.execute_batch(
                "DELETE FROM samples WHERE id NOT IN (SELECT min(id) FROM samples GROUP BY source_path);
                 CREATE UNIQUE INDEX samples_source ON samples(source_path);",
            )?;
            transaction.commit()?;
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(Self { connection })
    }

    pub fn open_reader(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != 2 {
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
        let root = path_codec::decode(transaction.query_row(
            "SELECT source_path FROM library_roots WHERE id=?1",
            [root_id],
            |row| row.get(0),
        )?)?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO samples(root_id, source_path, name, name_key, path, format, size_bytes, duration_seconds, sample_rate, channels, bit_depth, metadata_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(source_path) DO UPDATE SET root_id=excluded.root_id,
                   name=excluded.name, name_key=excluded.name_key, path=excluded.path,
                   format=excluded.format, size_bytes=excluded.size_bytes,
                   duration_seconds=excluded.duration_seconds, sample_rate=excluded.sample_rate,
                   channels=excluded.channels, bit_depth=excluded.bit_depth,
                   metadata_error=excluded.metadata_error",
            )?;
            for file in files {
                record_folders(&transaction, &file.source, &root)?;
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
        self.search_scoped(query, &Scope::All, offset)
    }

    pub fn search_scoped(
        &mut self,
        query: &SearchQuery,
        scope: &Scope,
        offset: usize,
    ) -> Result<SearchPage> {
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
        let condition = format!(
            "({condition}) AND ({})",
            scope_condition(scope, &mut values)?
        );
        // Count and rows share a snapshot even while the indexer commits batches.
        let transaction = self.connection.transaction()?;
        let total: i64 = transaction.query_row(
            &format!("SELECT count(*) FROM samples s WHERE {condition}"),
            params_from_iter(&values),
            |row| row.get(0),
        )?;
        let total = usize::try_from(total).map_err(|_| Error::OutOfRange)?;
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

    pub fn roots(&self) -> Result<Vec<PathBuf>> {
        let mut statement = self
            .connection
            .prepare("SELECT source_path FROM library_roots ORDER BY id")?;
        let mut rows = statement.query([])?;
        let mut roots = Vec::new();
        while let Some(row) = rows.next()? {
            roots.push(path_codec::decode(row.get(0)?)?);
        }
        Ok(roots)
    }

    pub fn count_scope(&self, scope: &Scope) -> Result<usize> {
        let mut values = Vec::new();
        let condition = scope_condition(scope, &mut values)?;
        let count: i64 = self.connection.query_row(
            &format!("SELECT count(*) FROM samples s WHERE {condition}"),
            params_from_iter(values),
            |row| row.get(0),
        )?;
        usize::try_from(count).map_err(|_| Error::OutOfRange)
    }

    pub fn folders(&mut self, parent: &Path, offset: usize) -> Result<FolderPage> {
        let transaction = self.connection.transaction()?;
        let parent_bytes = path_codec::encode(parent);
        let total: i64 = transaction.query_row(
            "SELECT count(*) FROM folders WHERE parent_path=?1",
            [&parent_bytes],
            |row| row.get(0),
        )?;
        let mut output = Vec::with_capacity(FOLDER_PAGE_SIZE);
        {
            let mut statement = transaction.prepare(
                "SELECT f.source_path, f.name, EXISTS(SELECT 1 FROM folders child WHERE child.parent_path=f.source_path)
                 FROM folders f WHERE f.parent_path=?1 ORDER BY f.name_key, f.source_path LIMIT ?2 OFFSET ?3",
            )?;
            let mut rows = statement.query(params![
                parent_bytes,
                FOLDER_PAGE_SIZE as i64,
                i64::try_from(offset).map_err(|_| Error::OutOfRange)?
            ])?;
            while let Some(row) = rows.next()? {
                output.push(Folder {
                    source: path_codec::decode(row.get(0)?)?,
                    name: row.get(1)?,
                    has_children: row.get(2)?,
                });
            }
        }
        transaction.commit()?;
        Ok(FolderPage {
            parent: parent.to_path_buf(),
            offset,
            total: usize::try_from(total).map_err(|_| Error::OutOfRange)?,
            rows: output,
        })
    }
}

fn record_folders(connection: &Connection, source: &Path, root: &Path) -> Result<()> {
    let mut current = source.parent();
    while let Some(folder) = current.filter(|folder| folder.starts_with(root)) {
        let name = folder
            .file_name()
            .unwrap_or(folder.as_os_str())
            .to_string_lossy();
        connection.execute("INSERT INTO folders(source_path, parent_path, name, name_key) VALUES(?1,?2,?3,?4) ON CONFLICT DO NOTHING",
            params![path_codec::encode(folder), folder.parent().map(path_codec::encode), name, name.to_lowercase()])?;
        if folder == root {
            break;
        }
        current = folder.parent();
    }
    Ok(())
}

fn scope_condition(scope: &Scope, values: &mut Vec<Value>) -> Result<String> {
    let Scope::Folders(folders) = scope else {
        return Ok("1".into());
    };
    if folders.len() > 512 {
        return Err(Error::OutOfRange);
    }
    let mut ranges = Vec::new();
    for (index, folder) in folders.iter().enumerate() {
        // Coalesce nested registrations before generating indexed OR ranges.
        if folders.iter().enumerate().any(|(other, ancestor)| {
            other != index && folder.starts_with(ancestor) && (folder != ancestor || other < index)
        }) {
            continue;
        }
        let lower = path_codec::encode(&folder.join(""));
        let mut upper = lower.clone();
        while upper.last() == Some(&255) {
            upper.pop();
        }
        let Some(last) = upper.last_mut() else {
            return Err(Error::InvalidPath);
        };
        *last += 1;
        let start = values.len() + 1;
        values.extend([Value::Blob(lower), Value::Blob(upper)]);
        ranges.push(format!(
            "s.source_path >= ?{start} AND s.source_path < ?{}",
            start + 1
        ));
    }
    Ok(if ranges.is_empty() {
        "0".into()
    } else {
        format!("({})", ranges.join(") OR ("))
    })
}
