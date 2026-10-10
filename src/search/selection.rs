use super::{
    SelectionPositions, SelectionSources, SortOrder,
    order::{Boundary, OrderKey},
};
use crate::path_codec;
use rusqlite::{
    Connection, OptionalExtension, Transaction, params_from_iter,
    types::{Type, Value},
};
use std::path::Path;

pub(crate) const MAX_SELECTION_KEYS: usize = 1_000_000;
pub(crate) const MAX_SELECTION_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy)]
pub(crate) enum OrderedTable {
    Samples,
    Favorites,
    Entries,
}

impl OrderedTable {
    fn sql(self) -> &'static str {
        match self {
            Self::Samples => "samples s",
            Self::Favorites => "favorites",
            Self::Entries => "entries",
        }
    }
}

fn endpoint(
    transaction: &Transaction<'_>,
    view: &OrderedView<'_>,
    path: &Path,
) -> rusqlite::Result<Option<OrderKey>> {
    let mut values = view.values.clone();
    let parameter = values.len() + 1;
    values.push(Value::Blob(path_codec::encode(path)));
    let sql = format!(
        "SELECT {},name_key,id FROM {} WHERE ({}) AND source_path=?{parameter}",
        view.order.column(),
        view.table.sql(),
        view.condition
    );
    transaction
        .query_row(&sql, params_from_iter(&values), |row| {
            Ok(OrderKey {
                value: row.get(0)?,
                name: row.get(1)?,
                id: row.get(2)?,
            })
        })
        .optional()
}

pub(crate) struct OrderedView<'a> {
    pub table: OrderedTable,
    pub condition: &'a str,
    pub values: Vec<Value>,
    pub order: SortOrder,
}

/// Endpoints, ordered membership and rows share one snapshot, including NULLs.
pub(crate) fn range_sources(
    connection: &mut Connection,
    view: OrderedView<'_>,
    anchor: &Path,
    target: &Path,
) -> rusqlite::Result<SelectionSources> {
    let transaction = connection.transaction()?;
    let (Some(first), Some(last)) = (
        endpoint(&transaction, &view, anchor)?,
        endpoint(&transaction, &view, target)?,
    ) else {
        return Ok(SelectionSources::MissingEndpoint);
    };
    let mut values = view.values.clone();
    let after_first = view
        .order
        .boundary(&first, Boundary::AtOrAfter, &mut values);
    let before_last = view
        .order
        .boundary(&last, Boundary::AtOrBefore, &mut values);
    let after_last = view.order.boundary(&last, Boundary::AtOrAfter, &mut values);
    let before_first = view
        .order
        .boundary(&first, Boundary::AtOrBefore, &mut values);
    let sql = format!(
        "SELECT source_path FROM {} WHERE ({}) AND (({after_first} AND {before_last}) OR ({after_last} AND {before_first})) ORDER BY {} LIMIT {}",
        view.table.sql(),
        view.condition,
        view.order.sql(),
        MAX_SELECTION_KEYS + 1
    );
    let mut sources = Vec::new();
    let mut bytes = 0;
    {
        let mut statement = transaction.prepare(&sql)?;
        let mut rows = statement.query(params_from_iter(&values))?;
        while let Some(row) = rows.next()? {
            let raw: Vec<u8> = row.get(0)?;
            bytes += raw.len();
            if bytes > MAX_SELECTION_BYTES || sources.len() == MAX_SELECTION_KEYS {
                return Ok(SelectionSources::TooLarge);
            }
            sources.push(path_codec::decode(raw).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Blob, Box::new(error))
            })?);
        }
    }
    transaction.commit()?;
    Ok(SelectionSources::Ready(sources))
}

/// Relocate native identities without loading preceding rows into application memory.
pub(crate) fn positions(
    connection: &mut Connection,
    view: OrderedView<'_>,
    active: &Path,
    anchor: Option<&Path>,
) -> rusqlite::Result<SelectionPositions> {
    let transaction = connection.transaction()?;
    let locate = |path: &Path| -> rusqlite::Result<Option<usize>> {
        let Some(key) = endpoint(&transaction, &view, path)? else {
            return Ok(None);
        };
        let mut parameters = view.values.clone();
        let before = view.order.boundary(&key, Boundary::Before, &mut parameters);
        let sql = format!(
            "SELECT count(*) FROM {} WHERE ({}) AND ({before})",
            view.table.sql(),
            view.condition
        );
        let count: i64 =
            transaction.query_row(&sql, params_from_iter(&parameters), |row| row.get(0))?;
        usize::try_from(count).map(Some).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(0, Type::Integer, Box::new(error))
        })
    };
    let cursor = locate(active)?;
    let anchor = match anchor {
        Some(path) if path == active => cursor,
        Some(path) => locate(path)?,
        None => None,
    };
    transaction.commit()?;
    Ok(SelectionPositions {
        active: cursor,
        anchor,
    })
}
