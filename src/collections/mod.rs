//! Durable, source-preserving saved selections, independent of the audio index.

pub mod favorites;
mod selected;
pub mod worker;
pub use selected::{ReferenceSelection, SelectionOrigin};

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, TransactionBehavior, params};

use crate::{catalog::Sample, library::AudioMetadata, path_codec};

use crate::search::MAX_ACTION_SOUNDS;
pub use crate::search::PAGE_SIZE;
pub(crate) const SCHEMA_VERSION: i64 = 3;
const APPLICATION_ID: i64 = 0x5346_434c;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CollectionId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntryId(pub i64);

#[derive(Clone, Debug)]
pub struct Collection {
    pub id: CollectionId,
    pub name: String,
    pub count: usize,
}

#[derive(Clone, Debug)]
pub struct Reference {
    pub source: PathBuf,
    pub name: String,
    pub display_path: String,
    pub format: String,
    pub size_bytes: u64,
    pub audio: AudioMetadata,
    pub metadata_error: Option<String>,
}

impl From<&Sample> for Reference {
    fn from(sample: &Sample) -> Self {
        Self {
            source: sample.source.clone(),
            name: sample.name.clone(),
            display_path: sample.display_path.clone(),
            format: sample.format.clone(),
            size_bytes: sample.size_bytes,
            audio: sample.audio.clone(),
            metadata_error: sample.metadata_error.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Ready,
    Changed,
    Missing,
    Unavailable(String),
}

impl Availability {
    pub fn usable(&self) -> bool {
        matches!(self, Self::Ready | Self::Changed)
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: EntryId,
    pub reference: Reference,
    pub availability: Availability,
}

#[derive(Debug)]
pub struct Page<T> {
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<T>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Catalog access failed: {0}")]
    Catalog(#[from] crate::Error),
    #[error("Catalog access is unavailable for this selection")]
    CatalogUnavailable,
    #[error(transparent)]
    Selection(#[from] crate::search::SelectionError),
    #[error("Saved sound storage failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Source could not be read: {0}")]
    Io(#[from] std::io::Error),
    #[error("Unsupported collection schema {0}; this application supports schema {SCHEMA_VERSION}")]
    UnsupportedSchema(i64),
    #[error("This file is not a Sift collection database")]
    WrongDatabase,
    #[error(
        "Use a collection name of 1 to {} characters without control characters",
        crate::names::MAX_CHARS
    )]
    InvalidName,
    #[error("A collection with that name already exists")]
    NameConflict,
    #[error("The selected collection or item no longer exists")]
    NotFound,
    #[error("Invalid saved sound reference")]
    InvalidReference,
    #[error("The source has changed since indexing; refresh its library before saving it")]
    SourceChanged,
    #[error("Choose between 1 and {MAX_ACTION_SOUNDS} sounds")]
    InvalidBatch,
    #[error("Favorite status requests exceed their bounded path or byte limit")]
    InvalidStatusBatch,
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn data_path(catalog: &Path) -> PathBuf {
    catalog.with_extension("collections.sqlite")
}

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open_reader(path: &Path) -> Result<Self> {
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let application: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            return Err(Error::UnsupportedSchema(version));
        }
        if application != APPLICATION_ID {
            return Err(Error::WrongDatabase);
        }
        Ok(Self { connection })
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let application: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::UnsupportedSchema(version));
        }
        if version == 0 && application == 0 {
            let tables: i64 = connection.query_row("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |row| row.get(0))?;
            if tables != 0 {
                return Err(Error::WrongDatabase);
            }
            let transaction = connection.transaction()?;
            transaction.execute_batch(include_str!("schema.sql"))?;
            transaction.execute_batch(include_str!("migration-2.sql"))?;
            transaction.commit()?;
        } else if !(1..=3).contains(&version) || application != APPLICATION_ID {
            return Err(Error::WrongDatabase);
        } else if version == 1 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(include_str!("migration-2.sql"))?;
            transaction.commit()?;
        }
        if version <= 2 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "ALTER TABLE entries ADD COLUMN name_key TEXT NOT NULL DEFAULT ''",
            )?;
            let mut after = 0i64;
            loop {
                let rows: Vec<(i64, String)> = {
                    let mut statement = transaction
                        .prepare("SELECT id,name FROM entries WHERE id>?1 ORDER BY id LIMIT 128")?;
                    statement
                        .query_map([after], |row| Ok((row.get(0)?, row.get(1)?)))?
                        .collect::<std::result::Result<_, _>>()?
                };
                if rows.is_empty() {
                    break;
                }
                for (id, name) in rows {
                    transaction.execute(
                        "UPDATE entries SET name_key=?1 WHERE id=?2",
                        params![name.to_lowercase(), id],
                    )?;
                    after = id;
                }
            }
            transaction.execute_batch(include_str!("migration-3.sql"))?;
            transaction.commit()?;
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self { connection })
    }

    pub fn create(&mut self, name: &str) -> Result<CollectionId> {
        let name = valid_name(name)?;
        if self.name_exists(&name, None)? {
            return Err(Error::NameConflict);
        }
        self.connection.execute(
            "INSERT INTO collections(name,name_key) VALUES (?1,?2)",
            params![name, name.to_lowercase()],
        )?;
        Ok(CollectionId(self.connection.last_insert_rowid()))
    }

    pub fn rename(&mut self, id: CollectionId, name: &str) -> Result<()> {
        let name = valid_name(name)?;
        if self.name_exists(&name, Some(id))? {
            return Err(Error::NameConflict);
        }
        if self.connection.execute(
            "UPDATE collections SET name=?1,name_key=?2 WHERE id=?3",
            params![name, name.to_lowercase(), id.0],
        )? == 0
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    pub fn delete(&mut self, id: CollectionId) -> Result<()> {
        if self
            .connection
            .execute("DELETE FROM collections WHERE id=?1", [id.0])?
            == 0
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    fn name_exists(&self, name: &str, except: Option<CollectionId>) -> Result<bool> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM collections WHERE name_key=?1 AND (?2 IS NULL OR id != ?2))", params![name.to_lowercase(), except.map(|id| id.0)], |row| row.get(0))?)
    }

    pub fn list(&mut self, offset: usize) -> Result<Page<Collection>> {
        let sql_offset = i64::try_from(offset).map_err(|_| Error::InvalidReference)?;
        let transaction = self.connection.transaction()?;
        let total = transaction.query_row("SELECT count(*) FROM collections", [], |row| {
            sql_count(row, 0)
        })?;
        let rows = {
            let mut statement = transaction.prepare("SELECT c.id,c.name,(SELECT count(*) FROM entries e WHERE e.collection_id=c.id) FROM collections c ORDER BY c.id LIMIT ?1 OFFSET ?2")?;
            statement
                .query_map(params![PAGE_SIZE as i64, sql_offset], |row| {
                    Ok(Collection {
                        id: CollectionId(row.get(0)?),
                        name: row.get(1)?,
                        count: sql_count(row, 2)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        transaction.commit()?;
        Ok(Page {
            total,
            offset,
            rows,
        })
    }

    pub fn add(&mut self, collection: CollectionId, references: &[Reference]) -> Result<()> {
        if references.is_empty() || references.len() > MAX_ACTION_SOUNDS {
            return Err(Error::InvalidBatch);
        }
        let mut revisions = Vec::with_capacity(references.len());
        for reference in references {
            validate_reference(reference)?;
            let revision = revision(&reference.source)?;
            if revision.0 != reference.size_bytes {
                return Err(Error::SourceChanged);
            }
            revisions.push(revision);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM collections WHERE id=?1)",
            [collection.0],
            |row| row.get::<_, bool>(0),
        )? {
            return Err(Error::NotFound);
        }
        for (reference, (size, modified)) in references.iter().zip(revisions) {
            transaction.execute("INSERT INTO entries(collection_id,source_path,name,display_path,format,size_bytes,modified_ns,duration_seconds,sample_rate,channels,bit_depth,metadata_error,name_key) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(collection_id,source_path) DO NOTHING",
                params![collection.0, path_codec::encode(&reference.source), reference.name,
                    reference.display_path, reference.format, size as i64, modified,
                    reference.audio.duration_seconds, reference.audio.sample_rate,
                    reference.audio.channels, reference.audio.bit_depth, reference.metadata_error, reference.name.to_lowercase()])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn remove(&mut self, collection: CollectionId, entry: EntryId) -> Result<()> {
        if self.connection.execute(
            "DELETE FROM entries WHERE collection_id=?1 AND id=?2",
            params![collection.0, entry.0],
        )? == 0
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    pub fn entry_selection_sources(
        &mut self,
        collection: CollectionId,
        anchor: &Path,
        target: &Path,
    ) -> Result<crate::search::SelectionSources> {
        self.entry_selection_sources_ordered(
            collection,
            crate::search::SortOrder::INSERTION,
            anchor,
            target,
        )
    }

    pub fn entry_selection_sources_ordered(
        &mut self,
        collection: CollectionId,
        order: crate::search::SortOrder,
        anchor: &Path,
        target: &Path,
    ) -> Result<crate::search::SelectionSources> {
        Ok(crate::search::selection::range_sources(
            &mut self.connection,
            crate::search::selection::OrderedView {
                table: crate::search::selection::OrderedTable::Entries,
                condition: "collection_id=?1",
                values: vec![rusqlite::types::Value::Integer(collection.0)],
                order,
            },
            anchor,
            target,
        )?)
    }

    pub fn entry_selection_positions(
        &mut self,
        collection: CollectionId,
        order: crate::search::SortOrder,
        active: &Path,
        anchor: Option<&Path>,
    ) -> Result<crate::search::SelectionPositions> {
        Ok(crate::search::selection::positions(
            &mut self.connection,
            crate::search::selection::OrderedView {
                table: crate::search::selection::OrderedTable::Entries,
                condition: "collection_id=?1",
                values: vec![rusqlite::types::Value::Integer(collection.0)],
                order,
            },
            active,
            anchor,
        )?)
    }

    pub fn entries(&mut self, collection: CollectionId, offset: usize) -> Result<Page<Entry>> {
        self.entries_ordered(collection, crate::search::SortOrder::INSERTION, offset)
    }

    pub fn entries_ordered(
        &mut self,
        collection: CollectionId,
        order: crate::search::SortOrder,
        offset: usize,
    ) -> Result<Page<Entry>> {
        let order = order.sql();
        let sql_offset = i64::try_from(offset).map_err(|_| Error::InvalidReference)?;
        let transaction = self.connection.transaction()?;
        if !transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM collections WHERE id=?1)",
            [collection.0],
            |row| row.get::<_, bool>(0),
        )? {
            return Err(Error::NotFound);
        }
        let total = transaction.query_row(
            "SELECT count(*) FROM entries WHERE collection_id=?1",
            [collection.0],
            |row| sql_count(row, 0),
        )?;
        let raw = {
            let mut statement = transaction.prepare(&format!("SELECT id,source_path,name,display_path,format,size_bytes,modified_ns,duration_seconds,sample_rate,channels,bit_depth,metadata_error FROM entries WHERE collection_id=?1 ORDER BY {order} LIMIT ?2 OFFSET ?3"))?;
            let mut rows = statement.query(params![collection.0, PAGE_SIZE as i64, sql_offset])?;
            let mut raw = Vec::new();
            while let Some(row) = rows.next()? {
                let reference = read_reference(row)?;
                raw.push((
                    EntryId(row.get(0)?),
                    reference,
                    row.get::<_, Option<i64>>(6)?,
                ));
            }
            raw
        };
        transaction.commit()?;
        let rows = raw
            .into_iter()
            .map(|(id, reference, modified)| {
                let availability = availability(&reference, modified);
                Entry {
                    id,
                    reference,
                    availability,
                }
            })
            .collect();
        Ok(Page {
            total,
            offset,
            rows,
        })
    }
}

fn sql_count(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<usize> {
    usize::try_from(row.get::<_, i64>(index)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn stored_text(row: &rusqlite::Row<'_>, index: usize, limit: usize) -> Result<String> {
    let value = row
        .get_ref(index)?
        .as_str()
        .map_err(|_| Error::InvalidReference)?;
    if value.len() > limit {
        return Err(Error::InvalidReference);
    }
    Ok(value.to_owned())
}

fn valid_name(name: &str) -> Result<String> {
    crate::names::normalize(name)
        .map(str::to_owned)
        .ok_or(Error::InvalidName)
}

fn validate_reference(reference: &Reference) -> Result<()> {
    if !reference.source.is_absolute()
        || path_codec::encode(&reference.source).len() > 65536
        || reference.name.is_empty()
        || reference.name.len() > 512
        || reference.display_path.len() > 32768
        || reference.format.is_empty()
        || reference.format.len() > 16
        || reference.size_bytes > i64::MAX as u64
        || reference
            .audio
            .duration_seconds
            .is_some_and(|duration| !duration.is_finite() || duration < 0.0)
        || [
            reference.audio.sample_rate,
            reference.audio.channels,
            reference.audio.bit_depth,
        ]
        .into_iter()
        .flatten()
        .any(|value| value == 0)
        || reference
            .metadata_error
            .as_ref()
            .is_some_and(|error| error.len() > 4096)
    {
        return Err(Error::InvalidReference);
    }
    Ok(())
}

fn revision(path: &Path) -> Result<(u64, Option<i64>)> {
    let metadata = path.symlink_metadata()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::InvalidReference);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Error::InvalidReference);
        }
    }
    let modified = timestamp(metadata.modified()?);
    Ok((metadata.len(), modified))
}

fn timestamp(time: SystemTime) -> Option<i64> {
    let signed = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::try_from(duration.as_nanos()).ok()?,
        Err(error) => -i128::try_from(error.duration().as_nanos()).ok()?,
    };
    i64::try_from(signed).ok()
}

fn read_reference(row: &rusqlite::Row<'_>) -> Result<Reference> {
    let path = row
        .get_ref(1)?
        .as_blob()
        .map_err(|_| Error::InvalidReference)?;
    if path.len() > 65536 {
        return Err(Error::InvalidReference);
    }
    let reference = Reference {
        source: path_codec::decode(path.to_vec()).map_err(|_| Error::InvalidReference)?,
        name: stored_text(row, 2, 512)?,
        display_path: stored_text(row, 3, 32768)?,
        format: stored_text(row, 4, 16)?,
        size_bytes: u64::try_from(row.get::<_, i64>(5)?).map_err(|_| Error::InvalidReference)?,
        audio: AudioMetadata {
            duration_seconds: row.get(7)?,
            sample_rate: row.get(8)?,
            channels: row.get(9)?,
            bit_depth: row.get(10)?,
        },
        metadata_error: if row.get_ref(11)?.data_type() == rusqlite::types::Type::Null {
            None
        } else {
            Some(stored_text(row, 11, 4096)?)
        },
    };
    validate_reference(&reference)?;
    Ok(reference)
}

fn availability(reference: &Reference, modified: Option<i64>) -> Availability {
    match revision(&reference.source) {
        Ok((size, current))
            if size != reference.size_bytes
                || (modified.is_some() && current.is_some() && modified != current) =>
        {
            Availability::Changed
        }
        Ok(_) => Availability::Ready,
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Availability::Missing
        }
        Err(error) => Availability::Unavailable(error.to_string()),
    }
}
