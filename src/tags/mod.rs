//! Durable manual labels keyed by native source locators, independent of catalog IDs.

pub mod worker;

use crate::path_codec;
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const SCHEMA_VERSION: i64 = 1;
const APPLICATION_ID: i64 = 0x5346_5447;
pub const MAX_NAME_CHARS: usize = 40;
pub const MAX_SOUND_TAGS: usize = 32;
pub const MAX_FILTER_TAGS: usize = 8;
pub const SUGGESTION_LIMIT: usize = 24;
pub const MAX_STATUS_SOURCES: usize =
    (crate::search::MAX_CACHED_PAGES + 1) * crate::search::PAGE_SIZE + 1;
const MAX_SOURCE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TagId(pub i64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub id: TagId,
    pub name: String,
}

/// Every selected tag must match. IDs are positive, unique and deliberately bounded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter(Vec<TagId>);

impl Filter {
    pub fn ids(&self) -> &[TagId] {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn contains(&self, id: TagId) -> bool {
        self.0.contains(&id)
    }
    pub fn set(&mut self, id: TagId, included: bool) -> Result<()> {
        if id.0 <= 0 {
            return Err(Error::InvalidTag);
        }
        match self.0.binary_search(&id) {
            Ok(index) if !included => {
                self.0.remove(index);
            }
            Err(index) if included => {
                if self.0.len() == MAX_FILTER_TAGS {
                    return Err(Error::FilterLimit);
                }
                self.0.insert(index, id);
            }
            _ => {}
        }
        Ok(())
    }
    pub fn retained_bytes(&self) -> usize {
        self.0.capacity() * size_of::<TagId>()
    }
}

#[derive(Clone, Debug)]
pub struct SourceTags {
    pub source: PathBuf,
    pub tags: Vec<Tag>,
}

#[derive(Clone, Debug)]
pub struct Membership {
    pub sources: Vec<SourceTags>,
    pub filter_tags: Vec<Tag>,
}
#[derive(Clone, Debug)]
pub struct TagUse {
    pub tag: Tag,
    pub count: usize,
}
#[derive(Clone, Debug)]
pub struct Summary {
    pub sources: usize,
    pub tags: Vec<TagUse>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Tag storage failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Tag data could not be opened: {0}")]
    Io(#[from] std::io::Error),
    #[error("Use a tag name of 1 to {MAX_NAME_CHARS} characters without control characters")]
    InvalidName,
    #[error("Invalid tag identity")]
    InvalidTag,
    #[error("The tag no longer exists")]
    NotFound,
    #[error("Each sound can have at most {MAX_SOUND_TAGS} tags")]
    SoundLimit,
    #[error("Use at most {MAX_FILTER_TAGS} tag filters")]
    FilterLimit,
    #[error(
        "Choose between 1 and {} sounds for tag edits",
        crate::search::MAX_ACTION_SOUNDS
    )]
    InvalidBatch,
    #[error("Tag requests exceed their native path or byte limit")]
    InvalidPaths,
    #[error("Unsupported tag schema {0}; this application supports schema {SCHEMA_VERSION}")]
    UnsupportedSchema(i64),
    #[error("This file is not a Sift tag database")]
    WrongDatabase,
    #[error("The tag database path cannot be represented without loss")]
    InvalidDatabasePath,
}
pub type Result<T> = std::result::Result<T, Error>;

pub fn data_path(catalog: &Path) -> PathBuf {
    catalog.with_extension("tags.sqlite")
}

pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn definitions(&self, ids: &[TagId]) -> Result<Vec<Tag>> {
        if ids.len() > MAX_FILTER_TAGS || ids.iter().any(|id| id.0 <= 0) {
            return Err(Error::InvalidTag);
        }
        let mut statement = self
            .connection
            .prepare("SELECT id,name FROM tags WHERE id=?1")?;
        let mut tags = Vec::with_capacity(ids.len());
        for id in ids {
            let mut rows = statement.query([id.0])?;
            if let Some(row) = rows.next()? {
                tags.push(read_tag(row)?);
            }
        }
        Ok(tags)
    }
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let application: i64 =
            connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
        if version == 0 && application == 0 {
            // A populated foreign database is never adopted or reset.
            let tables: i64 = connection.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )?;
            if tables != 0 {
                return Err(Error::WrongDatabase);
            }
            let transaction = connection.transaction()?;
            transaction.execute_batch(include_str!("schema.sql"))?;
            transaction.commit()?;
        } else {
            validate_schema(&connection, "main")?;
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(Self { connection })
    }
    pub fn open_reader(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        validate_schema(&connection, "main")?;
        Ok(Self { connection })
    }
    pub fn suggestions(&self, prefix: &str) -> Result<Vec<Tag>> {
        if prefix.chars().count() > MAX_NAME_CHARS {
            return Err(Error::InvalidName);
        }
        let lower = prefix.trim().to_lowercase();
        let upper = crate::search::prefix_successor(&lower);
        let mut statement = self.connection.prepare("SELECT id,name FROM tags WHERE name_key>=?1 AND (?2 IS NULL OR name_key<?2) ORDER BY name_key,id LIMIT ?3")?;
        Ok(statement
            .query_map(params![lower, upper, SUGGESTION_LIMIT as i64], read_tag)?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn status(&mut self, sources: &[PathBuf]) -> Result<Vec<SourceTags>> {
        let paths = encode_sources(sources, Batch::Status)?;
        let transaction = self.connection.transaction()?;
        let rows = {
            let mut statement = transaction.prepare("SELECT t.id,t.name FROM sound_tags st JOIN tags t ON t.id=st.tag_id WHERE st.source_path=?1 ORDER BY t.name_key,t.id LIMIT ?2")?;
            let mut rows = Vec::with_capacity(paths.len());
            for (source, path) in sources.iter().zip(paths) {
                let tags = statement
                    .query_map(params![path, (MAX_SOUND_TAGS + 1) as i64], read_tag)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                if tags.len() > MAX_SOUND_TAGS {
                    return Err(Error::SoundLimit);
                }
                rows.push(SourceTags {
                    source: source.clone(),
                    tags,
                });
            }
            rows
        };
        transaction.commit()?;
        Ok(rows)
    }
    pub fn summary(&mut self, sources: &[PathBuf]) -> Result<Summary> {
        encode_sources(sources, Batch::Edit)?;
        let rows = self.status(sources)?;
        let mut uses = std::collections::BTreeMap::new();
        for row in rows {
            for tag in row.tags {
                let entry = uses.entry(tag.id).or_insert(TagUse { tag, count: 0 });
                entry.count += 1;
            }
        }
        let mut tags: Vec<_> = uses.into_values().collect();
        tags.sort_by_cached_key(|used| (used.tag.name.to_lowercase(), used.tag.id));
        Ok(Summary {
            sources: sources.len(),
            tags,
        })
    }
    /// A batch adds only this association. All limits are checked in the same write transaction.
    pub fn add(&mut self, name: &str, sources: &[PathBuf]) -> Result<Tag> {
        let name = crate::names::normalize(name)
            .filter(|n| n.chars().count() <= MAX_NAME_CHARS)
            .ok_or(Error::InvalidName)?;
        let paths = encode_sources(sources, Batch::Edit)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO tags(name,name_key) VALUES(?1,?2) ON CONFLICT(name_key) DO NOTHING",
            params![name, name.to_lowercase()],
        )?;
        let tag = transaction.query_row(
            "SELECT id,name FROM tags WHERE name_key=?1",
            [name.to_lowercase()],
            read_tag,
        )?;
        for path in paths {
            let (count, exists): (i64, bool) = transaction.query_row(
                "SELECT count(*),coalesce(max(tag_id=?2),0) FROM sound_tags WHERE source_path=?1",
                params![path, tag.id.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if !exists && count >= MAX_SOUND_TAGS as i64 {
                return Err(Error::SoundLimit);
            }
            transaction.execute(
                "INSERT INTO sound_tags(source_path,tag_id) VALUES(?1,?2) ON CONFLICT DO NOTHING",
                params![path, tag.id.0],
            )?;
        }
        transaction.commit()?;
        Ok(tag)
    }
    pub fn remove(&mut self, id: TagId, sources: &[PathBuf]) -> Result<()> {
        if id.0 <= 0 {
            return Err(Error::InvalidTag);
        }
        let paths = encode_sources(sources, Batch::Edit)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM tags WHERE id=?1)",
            [id.0],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(Error::NotFound);
        }
        for path in paths {
            transaction.execute(
                "DELETE FROM sound_tags WHERE source_path=?1 AND tag_id=?2",
                params![path, id.0],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}

fn read_tag(row: &rusqlite::Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: TagId(row.get(0)?),
        name: row.get(1)?,
    })
}

pub(crate) enum Batch {
    Status,
    Edit,
}

pub(crate) fn encode_sources(sources: &[PathBuf], batch: Batch) -> Result<Vec<Vec<u8>>> {
    let limit = match batch {
        Batch::Status => MAX_STATUS_SOURCES,
        Batch::Edit => crate::search::MAX_ACTION_SOUNDS,
    };
    if sources.len() > limit || (matches!(batch, Batch::Edit) && sources.is_empty()) {
        return Err(Error::InvalidBatch);
    }
    let mut total = 0usize;
    let mut seen = BTreeSet::new();
    let mut encoded = Vec::with_capacity(sources.len());
    for source in sources {
        let bytes = path_codec::encode(source);
        total = total.saturating_add(bytes.len());
        if !source.is_absolute()
            || bytes.is_empty()
            || bytes.len() > 65536
            || total > MAX_SOURCE_BYTES
            || !seen.insert(bytes.clone())
        {
            return Err(Error::InvalidPaths);
        }
        encoded.push(bytes);
    }
    Ok(encoded)
}

pub(crate) fn validate_schema(connection: &Connection, schema: &str) -> Result<()> {
    let version: i64 = connection.pragma_query_value(Some(schema), "user_version", |r| r.get(0))?;
    if version != SCHEMA_VERSION {
        return Err(Error::UnsupportedSchema(version));
    }
    let application: i64 =
        connection.pragma_query_value(Some(schema), "application_id", |r| r.get(0))?;
    if application != APPLICATION_ID {
        return Err(Error::WrongDatabase);
    }
    Ok(())
}

pub(crate) fn readonly_uri(path: &Path) -> Result<String> {
    let absolute = std::path::absolute(path)?;
    #[cfg(windows)]
    let bytes = {
        let native = absolute.to_str().ok_or(Error::InvalidDatabasePath)?;
        let native = if let Some(unc) = native.strip_prefix(r"\\?\UNC\") {
            format!("//{unc}")
        } else {
            native.strip_prefix(r"\\?\").unwrap_or(native).to_owned()
        };
        let native = native.replace('\\', "/");
        let native = if native.starts_with("//") {
            format!("//{native}")
        } else {
            format!("/{native}")
        };
        native.into_bytes()
    };
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        absolute.as_os_str().as_bytes().to_vec()
    };
    let mut uri = String::from("file:");
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || b"/:-_.".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(uri, "%{byte:02X}").map_err(|_| Error::InvalidDatabasePath)?;
        }
    }
    uri.push_str("?mode=ro");
    Ok(uri)
}
