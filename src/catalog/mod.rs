use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, params, params_from_iter, types::Value};

use crate::library::{AudioMetadata, FileStamp, PROBE_VERSION};
use crate::search::{LibraryQuery, SearchQuery};
use crate::{Error, Result, path_codec};

mod query;
use query::{QueryPlan, QueryState};

pub(crate) const SCHEMA_VERSION: i64 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub stamp: Option<FileStamp>,
}

pub type SearchPage = crate::search::Page<Sample>;

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
    pub stamp: Option<FileStamp>,
}

pub struct Catalog {
    connection: Connection,
    query_state: QueryState,
    tag_database: PathBuf,
    tags_attached: bool,
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
        if version > SCHEMA_VERSION {
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
                    record_legacy_folders(
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
        if version <= 2 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(include_str!("migration-3.sql"))?;
            let mut after = 0i64;
            loop {
                let folders: Vec<(i64, Vec<u8>)> = {
                    let mut statement = transaction.prepare("SELECT rowid,source_path FROM folders WHERE rowid>?1 ORDER BY rowid LIMIT 128")?;
                    statement
                        .query_map([after], |row| Ok((row.get(0)?, row.get(1)?)))?
                        .collect::<std::result::Result<_, _>>()?
                };
                if folders.is_empty() {
                    break;
                }
                for (id, bytes) in folders {
                    let (lower, upper) = directory_range(&path_codec::decode(bytes)?)?;
                    transaction.execute(
                        "UPDATE folders SET prefix_lower=?1,prefix_upper=?2 WHERE rowid=?3",
                        params![lower, upper, id],
                    )?;
                    after = id;
                }
            }
            transaction.commit()?;
        }
        if version <= 3 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(include_str!("migration-4.sql"))?;
            transaction.commit()?;
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        Ok(Self {
            connection,
            query_state: QueryState::default(),
            tag_database: crate::tags::data_path(path),
            tags_attached: false,
        })
    }

    pub fn open_reader(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            return Err(Error::UnsupportedSchema(version));
        }
        Ok(Self {
            connection,
            query_state: QueryState::default(),
            tag_database: crate::tags::data_path(path),
            tags_attached: false,
        })
    }

    fn attach_tags(&mut self, query: &LibraryQuery) -> Result<()> {
        if query.tags.is_empty() || self.tags_attached {
            return Ok(());
        }
        let uri = crate::tags::readonly_uri(&self.tag_database)?;
        self.connection
            .execute("ATTACH DATABASE ?1 AS manual_tags", [uri])?;
        if let Err(validation) = crate::tags::validate_schema(&self.connection, "manual_tags") {
            if let Err(cleanup) = self.connection.execute("DETACH DATABASE manual_tags", []) {
                return Err(Error::InvalidQuery(format!(
                    "Tag data validation failed: {validation}; detaching unusable data also failed: {cleanup}"
                )));
            }
            return Err(validation.into());
        }
        self.tags_attached = true;
        Ok(())
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

    pub fn upsert_batch(&mut self, root_id: i64, files: &[IndexedFile]) -> Result<usize> {
        if files.is_empty() {
            return Ok(0);
        }
        let transaction = self.connection.transaction()?;
        let mut written = 0;
        let mut recorded_parents = std::collections::BTreeSet::new();
        let root = path_codec::decode(transaction.query_row(
            "SELECT source_path FROM library_roots WHERE id=?1",
            [root_id],
            |row| row.get(0),
        )?)?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO samples(id,root_id, source_path, name, name_key, path, format, size_bytes, duration_seconds, sample_rate, channels, bit_depth, metadata_error,modified_ns,created_ns,probe_version)
                 VALUES ((SELECT id FROM samples WHERE source_path=?2),?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,?13,?14,?15)
                 ON CONFLICT(source_path) DO UPDATE SET root_id=excluded.root_id,
                   name=excluded.name, name_key=excluded.name_key, path=excluded.path,
                   format=excluded.format, size_bytes=excluded.size_bytes,
                   duration_seconds=excluded.duration_seconds, sample_rate=excluded.sample_rate,
                   channels=excluded.channels, bit_depth=excluded.bit_depth,
                   metadata_error=excluded.metadata_error,modified_ns=excluded.modified_ns,
                   created_ns=excluded.created_ns,probe_version=excluded.probe_version
                 WHERE samples.root_id IS NOT excluded.root_id OR samples.name IS NOT excluded.name
                   OR samples.path IS NOT excluded.path OR samples.format IS NOT excluded.format
                   OR samples.size_bytes IS NOT excluded.size_bytes OR samples.duration_seconds IS NOT excluded.duration_seconds
                   OR samples.sample_rate IS NOT excluded.sample_rate OR samples.channels IS NOT excluded.channels
                   OR samples.bit_depth IS NOT excluded.bit_depth OR samples.metadata_error IS NOT excluded.metadata_error
                   OR samples.modified_ns IS NOT excluded.modified_ns OR samples.created_ns IS NOT excluded.created_ns
                   OR samples.probe_version IS NOT excluded.probe_version",
            )?;
            for file in files {
                // Enumeration batches usually share a directory. Insert its
                // hierarchy once per batch, rather than repeating SQL per sound.
                if let Some(parent) = file.source.parent()
                    && recorded_parents.insert(parent)
                {
                    record_folders(&transaction, &file.source, &root)?;
                }
                let size = i64::try_from(file.size_bytes).map_err(|_| Error::OutOfRange)?;
                written += statement.execute(params![
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
                    file.metadata_error,
                    file.stamp.as_ref().and_then(|stamp| stamp.modified_ns),
                    file.stamp.as_ref().and_then(|stamp| stamp.created_ns),
                    if file.stamp.is_some() {
                        PROBE_VERSION
                    } else {
                        0
                    }
                ])?;
            }
        }
        transaction.commit()?;
        Ok(written)
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
        self.search_ordered(
            &query.clone().into(),
            scope,
            crate::search::SortOrder::default(),
            offset,
        )
    }

    pub fn search_ordered(
        &mut self,
        query: &LibraryQuery,
        scope: &Scope,
        order: crate::search::SortOrder,
        offset: usize,
    ) -> Result<SearchPage> {
        let plan = QueryPlan::new(query, scope)?;
        self.attach_tags(query)?;
        // Counts, memo invalidation and rows share a main-database snapshot.
        let transaction = self.connection.transaction()?;
        let total = self.query_state.get(&transaction, &plan)?;
        let page = self
            .query_state
            .page(&transaction, &plan, order, offset, total)?;
        transaction.commit()?;
        Ok(page)
    }

    pub fn selection_sources(
        &mut self,
        query: &SearchQuery,
        scope: &Scope,
        anchor: &Path,
        target: &Path,
    ) -> Result<crate::search::SelectionSources> {
        self.selection_sources_ordered(
            &query.clone().into(),
            scope,
            crate::search::SortOrder::default(),
            anchor,
            target,
        )
    }

    pub fn selection_sources_ordered(
        &mut self,
        query: &LibraryQuery,
        scope: &Scope,
        order: crate::search::SortOrder,
        anchor: &Path,
        target: &Path,
    ) -> Result<crate::search::SelectionSources> {
        let plan = QueryPlan::new(query, scope)?;
        self.attach_tags(query)?;
        Ok(crate::search::selection::range_sources(
            &mut self.connection,
            crate::search::selection::OrderedView {
                table: plan.table,
                condition: &plan.condition,
                values: plan.values,
                order,
            },
            anchor,
            target,
        )?)
    }

    pub fn selection_positions(
        &mut self,
        query: &LibraryQuery,
        scope: &Scope,
        order: crate::search::SortOrder,
        active: &Path,
        anchor: Option<&Path>,
    ) -> Result<crate::search::SelectionPositions> {
        let plan = QueryPlan::new(query, scope)?;
        self.attach_tags(query)?;
        Ok(crate::search::selection::positions(
            &mut self.connection,
            crate::search::selection::OrderedView {
                table: plan.table,
                condition: &plan.condition,
                values: plan.values,
                order,
            },
            active,
            anchor,
        )?)
    }

    pub fn selected_references(
        &mut self,
        query: &SearchQuery,
        scope: &Scope,
        selection: &crate::search::SourceSelection,
    ) -> Result<Vec<crate::collections::Reference>> {
        self.selected_references_ordered(
            &query.clone().into(),
            scope,
            crate::search::SortOrder::default(),
            selection,
        )
    }

    pub fn selected_references_ordered(
        &mut self,
        query: &LibraryQuery,
        scope: &Scope,
        order: crate::search::SortOrder,
        selection: &crate::search::SourceSelection,
    ) -> Result<Vec<crate::collections::Reference>> {
        let plan = QueryPlan::new(query, scope)?;
        self.attach_tags(query)?;
        let transaction = self.connection.transaction()?;
        let identities = crate::search::batch::selected_rows(
            &transaction,
            plan.from(),
            &order.sql(),
            &plan.condition,
            plan.values,
            selection,
        )?;
        let references = {
            let mut statement = transaction.prepare("SELECT id,source_path,name,path,format,size_bytes,duration_seconds,sample_rate,channels,bit_depth,metadata_error,modified_ns,created_ns FROM samples WHERE id=?1")?;
            let mut references = Vec::with_capacity(identities.len());
            for (id, _) in identities {
                let mut rows = statement.query([id])?;
                let row = rows.next()?.ok_or(crate::search::SelectionError::Changed)?;
                references.push(crate::collections::Reference::from(&sample_from_row(row)?));
            }
            references
        };
        transaction.commit()?;
        Ok(references)
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

    pub(crate) fn sample(&self, source: &Path) -> Result<Option<Sample>> {
        let mut statement=self.connection.prepare_cached("SELECT id,source_path,name,path,format,size_bytes,duration_seconds,sample_rate,channels,bit_depth,metadata_error,modified_ns,created_ns FROM samples WHERE source_path=?1")?;
        let mut rows = statement.query([path_codec::encode(source)])?;
        rows.next()?.map(sample_from_row).transpose()
    }

    pub(crate) fn unchanged(&self, source: &Path, stamp: &FileStamp) -> Result<bool> {
        if stamp.modified_ns.is_none() {
            return Ok(false);
        }
        let exists: bool = self
            .connection
            .prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM samples WHERE source_path=?1 AND size_bytes=?2
             AND modified_ns=?3 AND created_ns IS ?4 AND probe_version=?5)",
            )?
            .query_row(
                params![
                    path_codec::encode(source),
                    i64::try_from(stamp.size_bytes).map_err(|_| Error::OutOfRange)?,
                    stamp.modified_ns,
                    stamp.created_ns,
                    PROBE_VERSION
                ],
                |row| row.get(0),
            )?;
        Ok(exists)
    }

    pub(crate) fn begin_inventory(&self) -> Result<()> {
        self.connection.execute_batch(
            "PRAGMA temp_store=FILE;
             CREATE TEMP TABLE IF NOT EXISTS scan_seen(source_path BLOB PRIMARY KEY) STRICT;
             PRAGMA temp.cache_size=-1024;
             DELETE FROM scan_seen;",
        )?;
        Ok(())
    }

    pub(crate) fn record_seen(&mut self, paths: &[PathBuf]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        {
            let mut statement = transaction
                .prepare_cached("INSERT OR IGNORE INTO scan_seen(source_path) VALUES(?1)")?;
            for path in paths {
                statement.execute([path_codec::encode(path)])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn finish_inventory(&mut self, root: &Path) -> Result<usize> {
        let transaction = self.connection.transaction()?;
        let (lower, upper) = directory_range(root)?;
        let removed = transaction.execute(
            "DELETE FROM samples WHERE source_path>=?1 AND source_path<?2
             AND NOT EXISTS(SELECT 1 FROM scan_seen WHERE scan_seen.source_path=samples.source_path)",params![lower,upper],
        )?;
        transaction.execute(
            "DELETE FROM folders WHERE (source_path=?1 OR (source_path>=?2 AND source_path<?3))
             AND NOT EXISTS(SELECT 1 FROM samples WHERE samples.source_path>=folders.prefix_lower AND samples.source_path<folders.prefix_upper)",
            params![path_codec::encode(root),lower,upper],
        )?;
        transaction.execute("DELETE FROM scan_seen", [])?;
        transaction.commit()?;
        Ok(removed)
    }

    pub(crate) fn remove_cached_path(&mut self, root: &Path, path: &Path) -> Result<usize> {
        let transaction = self.connection.transaction()?;
        let (lower, upper) = directory_range(path)?;
        let removed = transaction.execute(
            "DELETE FROM samples WHERE source_path=?1 OR (source_path>=?2 AND source_path<?3)",
            params![path_codec::encode(path), lower, upper],
        )?;
        transaction.execute(
            "DELETE FROM folders WHERE source_path=?1 OR (source_path>=?2 AND source_path<?3)",
            params![path_codec::encode(path), lower, upper],
        )?;
        for ancestor in path
            .ancestors()
            .skip(1)
            .take_while(|ancestor| ancestor.starts_with(root))
        {
            let (lower, upper) = directory_range(ancestor)?;
            transaction.execute("DELETE FROM folders WHERE source_path=?1 AND NOT EXISTS(SELECT 1 FROM samples WHERE source_path>=?2 AND source_path<?3)",params![path_codec::encode(ancestor),lower,upper])?;
        }
        transaction.commit()?;
        Ok(removed)
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

fn sample_from_row(row: &rusqlite::Row<'_>) -> Result<Sample> {
    let size_bytes = u64::try_from(row.get::<_, i64>(5)?).map_err(|_| Error::OutOfRange)?;
    let modified_ns: Option<i64> = row.get(11)?;
    let created_ns: Option<i64> = row.get(12)?;
    Ok(Sample {
        id: SampleId(row.get(0)?),
        source: path_codec::decode(row.get(1)?)?,
        name: row.get(2)?,
        display_path: row.get(3)?,
        format: row.get(4)?,
        size_bytes,
        audio: AudioMetadata {
            duration_seconds: row.get(6)?,
            sample_rate: row.get(7)?,
            channels: row.get(8)?,
            bit_depth: row.get(9)?,
        },
        metadata_error: row.get(10)?,
        stamp: modified_ns.map(|modified| FileStamp {
            size_bytes,
            modified_ns: Some(modified),
            created_ns,
        }),
    })
}

fn record_folders(connection: &Connection, source: &Path, root: &Path) -> Result<()> {
    let mut current = source.parent();
    while let Some(folder) = current.filter(|folder| folder.starts_with(root)) {
        let name = folder
            .file_name()
            .unwrap_or(folder.as_os_str())
            .to_string_lossy();
        let (lower, upper) = directory_range(folder)?;
        connection.prepare_cached("INSERT INTO folders(source_path,parent_path,name,name_key,prefix_lower,prefix_upper) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT DO NOTHING")?.execute(params![path_codec::encode(folder),folder.parent().map(path_codec::encode),name,name.to_lowercase(),lower,upper])?;
        if folder == root {
            break;
        }
        current = folder.parent();
    }
    Ok(())
}

fn record_legacy_folders(connection: &Connection, source: &Path, root: &Path) -> Result<()> {
    for folder in source
        .ancestors()
        .skip(1)
        .take_while(|folder| folder.starts_with(root))
    {
        let name = folder
            .file_name()
            .unwrap_or(folder.as_os_str())
            .to_string_lossy();
        connection.execute("INSERT INTO folders(source_path,parent_path,name,name_key) VALUES(?1,?2,?3,?4) ON CONFLICT DO NOTHING",params![path_codec::encode(folder),folder.parent().map(path_codec::encode),name,name.to_lowercase()])?;
        if folder == root {
            break;
        }
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
        let (lower, upper) = directory_range(folder)?;
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

fn directory_range(folder: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    let lower = path_codec::encode(&folder.join(""));
    let mut upper = lower.clone();
    while upper.last() == Some(&255) {
        upper.pop();
    }
    let Some(last) = upper.last_mut() else {
        return Err(Error::InvalidPath);
    };
    *last += 1;
    Ok((lower, upper))
}
