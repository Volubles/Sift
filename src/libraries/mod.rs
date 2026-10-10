//! Durable named banks of source directories, independent of the derived index.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior, params};

use crate::{
    catalog::{Catalog, Scope},
    path_codec,
};

const APPLICATION_ID: i64 = 0x5346_4c42;
pub const MAX_LIBRARIES: usize = 128;
pub const MAX_SOURCES: usize = 32;
const MAX_TOTAL_SOURCES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LibraryId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceId(pub i64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Online,
    Offline,
    Unavailable(String),
}

#[derive(Clone, Debug)]
pub struct Source {
    pub id: SourceId,
    pub path: PathBuf,
    pub count: usize,
    pub availability: Availability,
}

#[derive(Clone, Debug)]
pub struct Library {
    pub id: LibraryId,
    pub name: String,
    pub sources: Vec<Source>,
    pub count: usize,
}

impl Library {
    pub fn scope(&self) -> Scope {
        Scope::Folders(
            self.sources
                .iter()
                .map(|source| source.path.clone())
                .collect(),
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Library storage failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Source folder could not be read: {0}")]
    Io(#[from] std::io::Error),
    #[error("Library index could not be read: {0}")]
    Catalog(#[from] crate::Error),
    #[error("Unsupported library schema {0}; this application supports schema 1")]
    UnsupportedSchema(i64),
    #[error("This file is not a Sift library database")]
    WrongDatabase,
    #[error("Use a library name of 1 to 120 characters without control characters")]
    InvalidName,
    #[error("A library with that name already exists")]
    NameConflict,
    #[error("The selected library or source folder no longer exists")]
    NotFound,
    #[error("Choose an existing directory without a symbolic link or reparse point")]
    InvalidSource,
    #[error(
        "This prototype supports up to 128 libraries, 32 sources per library, and 512 sources in total"
    )]
    Limit,
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn data_path(catalog: &Path) -> PathBuf {
    catalog.with_extension("libraries.sqlite")
}

pub struct Store {
    connection: Connection,
}

impl Store {
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
        let app: i64 = connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        if version > 1 {
            return Err(Error::UnsupportedSchema(version));
        }
        if version == 0 {
            let tables: i64 = connection.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )?;
            if app != 0 || tables != 0 {
                return Err(Error::WrongDatabase);
            }
            let transaction = connection.transaction()?;
            transaction.execute_batch(include_str!("schema.sql"))?;
            transaction.commit()?;
        } else if app != APPLICATION_ID {
            return Err(Error::WrongDatabase);
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self { connection })
    }

    pub fn create(&mut self, name: &str) -> Result<LibraryId> {
        let name = valid_name(name)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count: i64 =
            transaction.query_row("SELECT count(*) FROM libraries", [], |row| row.get(0))?;
        if count >= MAX_LIBRARIES as i64 {
            return Err(Error::Limit);
        }
        name_available(&transaction, &name, None)?;
        transaction.execute(
            "INSERT INTO libraries(name,name_key) VALUES(?1,?2)",
            params![name, name.to_lowercase()],
        )?;
        let id = LibraryId(transaction.last_insert_rowid());
        transaction.commit()?;
        Ok(id)
    }

    pub fn rename(&mut self, id: LibraryId, name: &str) -> Result<()> {
        let name = valid_name(name)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        name_available(&transaction, &name, Some(id))?;
        if transaction.execute(
            "UPDATE libraries SET name=?1,name_key=?2 WHERE id=?3",
            params![name, name.to_lowercase(), id.0],
        )? != 1
        {
            return Err(Error::NotFound);
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn remove(&mut self, id: LibraryId) -> Result<()> {
        if self
            .connection
            .execute("DELETE FROM libraries WHERE id=?1", [id.0])?
            != 1
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    /// Resolve the chosen directory on the storage worker, never the UI thread.
    pub fn attach(&mut self, id: LibraryId, path: &Path) -> Result<PathBuf> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_dir() || is_link(&metadata) {
            return Err(Error::InvalidSource);
        }
        let path = path.canonicalize()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_source(&transaction, id, &path)?;
        transaction.commit()?;
        Ok(path)
    }

    pub fn detach(&mut self, library: LibraryId, source: SourceId) -> Result<()> {
        if self.connection.execute(
            "DELETE FROM sources WHERE id=?1 AND library_id=?2",
            params![source.0, library.0],
        )? != 1
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    /// The CLI indexes one folder as a bank, reusing an existing registration.
    pub fn register_folder_bank(&mut self, path: &Path) -> Result<PathBuf> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_dir() || is_link(&metadata) {
            return Err(Error::InvalidSource);
        }
        let path = path.canonicalize()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let registered: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sources WHERE source_path=?1)",
            [path_codec::encode(&path)],
            |row| row.get(0),
        )?;
        if !registered {
            let count: i64 =
                transaction.query_row("SELECT count(*) FROM libraries", [], |row| row.get(0))?;
            if count >= MAX_LIBRARIES as i64 {
                return Err(Error::Limit);
            }
            let name = unique_folder_name(&transaction, &path)?;
            transaction.execute(
                "INSERT INTO libraries(name,name_key) VALUES(?1,?2)",
                params![name, name.to_lowercase()],
            )?;
            insert_source(
                &transaction,
                LibraryId(transaction.last_insert_rowid()),
                &path,
            )?;
        }
        transaction.commit()?;
        Ok(path)
    }

    /// Import previous root registrations once. No filesystem access or scan is
    /// needed: an offline bank must remain visible after upgrading or rebuilding.
    pub fn import_legacy(&mut self, roots: &[PathBuf]) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let imported: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE key='legacy_imported')",
            [],
            |row| row.get(0),
        )?;
        if imported {
            return Ok(());
        }
        for path in roots {
            if !path.is_absolute() {
                return Err(Error::InvalidSource);
            }
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM sources WHERE source_path=?1)",
                [path_codec::encode(path)],
                |row| row.get(0),
            )?;
            if exists {
                continue;
            }
            let count: i64 =
                transaction.query_row("SELECT count(*) FROM libraries", [], |row| row.get(0))?;
            if count >= MAX_LIBRARIES as i64 {
                return Err(Error::Limit);
            }
            let name = unique_folder_name(&transaction, path)?;
            transaction.execute(
                "INSERT INTO libraries(name,name_key) VALUES(?1,?2)",
                params![name, name.to_lowercase()],
            )?;
            let id = LibraryId(transaction.last_insert_rowid());
            insert_source(&transaction, id, path)?;
        }
        transaction.execute(
            "INSERT INTO settings(key,value) VALUES('legacy_imported',1)",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn snapshot(&self, catalog: &Catalog) -> Result<Vec<Library>> {
        let mut statement = self
            .connection
            .prepare("SELECT id,name FROM libraries ORDER BY name_key,id LIMIT 129")?;
        let rows = statement.query_map([], |row| {
            Ok((LibraryId(row.get(0)?), row.get::<_, String>(1)?))
        })?;
        let mut libraries = Vec::new();
        let mut total_sources = 0;
        for row in rows {
            if libraries.len() == MAX_LIBRARIES {
                return Err(Error::Limit);
            }
            let (id, name) = row?;
            let mut sources = Vec::new();
            let mut statement = self.connection.prepare(
                "SELECT id,source_path FROM sources WHERE library_id=?1 ORDER BY id LIMIT 33",
            )?;
            let mut rows = statement.query([id.0])?;
            while let Some(row) = rows.next()? {
                if sources.len() == MAX_SOURCES || total_sources == MAX_TOTAL_SOURCES {
                    return Err(Error::Limit);
                }
                let path = path_codec::decode(row.get(1)?)?;
                let count = catalog.count_scope(&Scope::Folders(vec![path.clone()]))?;
                let availability = match std::fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {
                        Availability::Online
                    }
                    Ok(_) => Availability::Unavailable(
                        "The registered source is no longer an ordinary directory".into(),
                    ),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        Availability::Offline
                    }
                    Err(error) => Availability::Unavailable(error.to_string()),
                };
                sources.push(Source {
                    id: SourceId(row.get(0)?),
                    path,
                    count,
                    availability,
                });
                total_sources += 1;
            }
            let mut library = Library {
                id,
                name,
                sources,
                count: 0,
            };
            library.count = catalog.count_scope(&library.scope())?;
            libraries.push(library);
        }
        Ok(libraries)
    }
}

fn valid_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err(Error::InvalidName);
    }
    Ok(name.to_owned())
}

fn unique_folder_name(connection: &Connection, path: &Path) -> Result<String> {
    let base = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    let base: String = base
        .chars()
        .filter(|ch| !ch.is_control())
        .take(100)
        .collect();
    let base = if base.trim().is_empty() {
        "Imported library"
    } else {
        base.trim()
    };
    let mut name = base.to_owned();
    let mut suffix = 2;
    loop {
        match name_available(connection, &name, None) {
            Ok(()) => return Ok(name),
            Err(Error::NameConflict) => {
                name = format!("{base} ({suffix})");
                suffix += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn name_available(connection: &Connection, name: &str, except: Option<LibraryId>) -> Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM libraries WHERE name_key=?1 AND id!=?2)",
        params![name.to_lowercase(), except.map_or(-1, |id| id.0)],
        |row| row.get(0),
    )?;
    if exists {
        Err(Error::NameConflict)
    } else {
        Ok(())
    }
}

fn insert_source(connection: &Connection, library: LibraryId, path: &Path) -> Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM libraries WHERE id=?1)",
        [library.0],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(Error::NotFound);
    }
    let bytes = path_codec::encode(path);
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sources WHERE library_id=?1 AND source_path=?2)",
        params![library.0, bytes],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(());
    }
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM sources WHERE library_id=?1",
        [library.0],
        |row| row.get(0),
    )?;
    let total: i64 = connection.query_row("SELECT count(*) FROM sources", [], |row| row.get(0))?;
    if count >= MAX_SOURCES as i64 || total >= MAX_TOTAL_SOURCES as i64 {
        return Err(Error::Limit);
    }
    connection.execute(
        "INSERT INTO sources(library_id,source_path) VALUES(?1,?2)",
        params![library.0, bytes],
    )?;
    Ok(())
}

fn is_link(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
