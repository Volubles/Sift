//! Global saved references, independent of catalog IDs and collection membership.

use std::path::{Path, PathBuf};

use rusqlite::{params, params_from_iter, types::Value};

use super::{Availability, Error, PAGE_SIZE, Page, Reference, Result, Store};
use crate::{path_codec, search::SearchQuery};

// Catalog cache, one collection page, and the active preview reference.
pub const MAX_STATUS_PATHS: usize = (crate::search::MAX_CACHED_PAGES + 1) * PAGE_SIZE + 1;
const MAX_STATUS_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FavoriteId(pub i64);

#[derive(Clone, Debug)]
pub struct Favorite {
    pub id: FavoriteId,
    pub reference: Reference,
    pub availability: Availability,
}

impl Store {
    pub fn favorite(&mut self, reference: &Reference) -> Result<()> {
        super::validate_reference(reference)?;
        let (size, modified) = super::revision(&reference.source)?;
        if size != reference.size_bytes {
            return Err(Error::SourceChanged);
        }
        // A repeated explicit save is idempotent; it does not replace the saved
        // revision or allocate a new identity. Collection membership is unrelated.
        let transaction = self.connection.transaction()?;
        insert_favorite(&transaction, reference, size, modified)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn toggle_favorite(&mut self, reference: &Reference) -> Result<bool> {
        validate_path(&reference.source)?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM favorites WHERE source_path=?1)",
            [path_codec::encode(&reference.source)],
            |row| row.get(0),
        )?;
        if exists {
            transaction.execute(
                "DELETE FROM favorites WHERE source_path=?1",
                [path_codec::encode(&reference.source)],
            )?;
        } else {
            super::validate_reference(reference)?;
            let (size, modified) = super::revision(&reference.source)?;
            if size != reference.size_bytes {
                return Err(Error::SourceChanged);
            }
            insert_favorite(&transaction, reference, size, modified)?;
        }
        transaction.commit()?;
        Ok(!exists)
    }

    pub fn unfavorite(&mut self, source: &Path) -> Result<()> {
        validate_path(source)?;
        // Removal still works when the original file or its entire bank is offline.
        self.connection.execute(
            "DELETE FROM favorites WHERE source_path=?1",
            [path_codec::encode(source)],
        )?;
        Ok(())
    }

    pub fn favorite_status(&mut self, paths: &[PathBuf]) -> Result<Vec<(PathBuf, bool)>> {
        if paths.len() > MAX_STATUS_PATHS {
            return Err(Error::InvalidStatusBatch);
        }
        let mut bytes = 0usize;
        for path in paths {
            validate_path(path)?;
            bytes = bytes.saturating_add(path_codec::encode(path).len());
            if bytes > MAX_STATUS_BYTES {
                return Err(Error::InvalidStatusBatch);
            }
        }
        let transaction = self.connection.transaction()?;
        let status = {
            let mut statement = transaction
                .prepare_cached("SELECT EXISTS(SELECT 1 FROM favorites WHERE source_path=?1)")?;
            paths
                .iter()
                .map(|path| {
                    Ok((
                        path.clone(),
                        statement.query_row([path_codec::encode(path)], |row| row.get(0))?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?
        };
        transaction.commit()?;
        Ok(status)
    }

    pub fn favorite_selection_sources(
        &mut self,
        query: &SearchQuery,
        anchor: &Path,
        target: &Path,
    ) -> Result<crate::search::SelectionSources> {
        self.favorite_selection_sources_ordered(
            query,
            crate::search::SortOrder::default(),
            anchor,
            target,
        )
    }

    pub fn favorite_selection_sources_ordered(
        &mut self,
        query: &SearchQuery,
        order: crate::search::SortOrder,
        anchor: &Path,
        target: &Path,
    ) -> Result<crate::search::SelectionSources> {
        let (condition, values) = favorite_condition(query);
        Ok(crate::search::selection::range_sources(
            &mut self.connection,
            crate::search::selection::OrderedView {
                table: crate::search::selection::OrderedTable::Favorites,
                condition,
                values,
                order,
            },
            anchor,
            target,
        )?)
    }

    pub fn favorite_selection_positions(
        &mut self,
        query: &SearchQuery,
        order: crate::search::SortOrder,
        active: &Path,
        anchor: Option<&Path>,
    ) -> Result<crate::search::SelectionPositions> {
        let (condition, values) = favorite_condition(query);
        Ok(crate::search::selection::positions(
            &mut self.connection,
            crate::search::selection::OrderedView {
                table: crate::search::selection::OrderedTable::Favorites,
                condition,
                values,
                order,
            },
            active,
            anchor,
        )?)
    }

    pub fn favorites(&mut self, query: &SearchQuery, offset: usize) -> Result<Page<Favorite>> {
        self.favorites_ordered(query, crate::search::SortOrder::default(), offset)
    }

    pub fn favorites_ordered(
        &mut self,
        query: &SearchQuery,
        order: crate::search::SortOrder,
        offset: usize,
    ) -> Result<Page<Favorite>> {
        let order = order.sql();
        let (condition, mut values) = favorite_condition(query);
        let transaction = self.connection.transaction()?;
        let total = transaction.query_row(
            &format!("SELECT count(*) FROM favorites WHERE {condition}"),
            params_from_iter(&values),
            |row| super::sql_count(row, 0),
        )?;
        let limit = values.len() + 1;
        let skip = values.len() + 2;
        values.push(Value::Integer(PAGE_SIZE as i64));
        values.push(Value::Integer(
            i64::try_from(offset).map_err(|_| Error::InvalidReference)?,
        ));
        let raw = {
            let sql = format!(
                "SELECT id,source_path,name,display_path,format,size_bytes,modified_ns,duration_seconds,sample_rate,channels,bit_depth,metadata_error FROM favorites WHERE {condition} ORDER BY {order} LIMIT ?{limit} OFFSET ?{skip}"
            );
            let mut statement = transaction.prepare(&sql)?;
            let mut rows = statement.query(params_from_iter(&values))?;
            let mut raw = Vec::with_capacity(PAGE_SIZE);
            while let Some(row) = rows.next()? {
                raw.push((
                    FavoriteId(row.get(0)?),
                    super::read_reference(row)?,
                    row.get::<_, Option<i64>>(6)?,
                ));
            }
            raw
        };
        transaction.commit()?;
        let rows = raw
            .into_iter()
            .map(|(id, reference, modified)| Favorite {
                id,
                availability: super::availability(&reference, modified),
                reference,
            })
            .collect();
        Ok(Page {
            total,
            offset,
            rows,
        })
    }
}

fn validate_path(path: &Path) -> Result<()> {
    if !path.is_absolute() || path_codec::encode(path).len() > 65536 {
        return Err(Error::InvalidReference);
    }
    Ok(())
}

pub(super) fn favorite_condition(query: &SearchQuery) -> (&'static str, Vec<Value>) {
    let (condition, values) = match query {
        SearchQuery::All => ("1", vec![]),
        SearchQuery::FilenamePrefix { lower, upper } => (
            "name_key>=?1 AND name_key<?2",
            vec![Value::Text(lower.clone()), Value::Text(upper.clone())],
        ),
        SearchQuery::Substrings(expression) => (
            "id IN (SELECT rowid FROM favorites_fts WHERE favorites_fts MATCH ?1)",
            vec![Value::Text(expression.clone())],
        ),
    };
    (condition, values)
}

fn insert_favorite(
    transaction: &rusqlite::Transaction<'_>,
    reference: &Reference,
    size: u64,
    modified: Option<i64>,
) -> Result<()> {
    transaction.execute("INSERT INTO favorites(source_path,name,name_key,display_path,format,size_bytes,modified_ns,duration_seconds,sample_rate,channels,bit_depth,metadata_error)
            SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12 WHERE NOT EXISTS(SELECT 1 FROM favorites WHERE source_path=?1)",
            params![path_codec::encode(&reference.source), reference.name, reference.name.to_lowercase(),
                reference.display_path, reference.format, size as i64, modified,
                reference.audio.duration_seconds, reference.audio.sample_rate, reference.audio.channels,
                reference.audio.bit_depth, reference.metadata_error])?;
    Ok(())
}
