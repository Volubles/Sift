use super::SelectionSources;
use crate::path_codec;
use rusqlite::{
    Connection, OptionalExtension, params_from_iter,
    types::{Type, Value},
};
use std::path::Path;

pub(crate) const MAX_SELECTION_KEYS: usize = 1_000_000;
pub(crate) const MAX_SELECTION_BYTES: usize = 8 * 1024 * 1024;
pub(crate) enum OrderedTable {
    Samples,
    Favorites,
    Entries,
}

/// Resolve exact native locators in one snapshot. Rebuilding the catalog can
/// replace its IDs; concurrent edits can also shift previously displayed rows.
pub(crate) fn range_sources(
    connection: &mut Connection,
    table: OrderedTable,
    condition: &str,
    mut values: Vec<Value>,
    anchor: &Path,
    target: &Path,
) -> rusqlite::Result<SelectionSources> {
    let (table, name) = match table {
        OrderedTable::Samples => ("samples s", "name_key"),
        OrderedTable::Favorites => ("favorites", "name_key"),
        OrderedTable::Entries => ("entries", "''"),
    };
    let transaction = connection.transaction()?;
    let endpoint_parameter = values.len() + 1;
    let endpoint_sql = format!(
        "SELECT {name},id FROM {table} WHERE ({condition}) AND source_path=?{endpoint_parameter}"
    );
    let mut endpoint = |path| {
        values.push(Value::Blob(path_codec::encode(path)));
        let result = transaction
            .query_row(&endpoint_sql, params_from_iter(&values), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .optional();
        values.pop();
        result
    };
    let (Some(first), Some(last)) = (endpoint(anchor)?, endpoint(target)?) else {
        return Ok(SelectionSources::MissingEndpoint);
    };
    let (first, last) = if first <= last {
        (first, last)
    } else {
        (last, first)
    };
    let start = values.len() + 1;
    values.extend([
        Value::Text(first.0),
        Value::Integer(first.1),
        Value::Text(last.0),
        Value::Integer(last.1),
    ]);
    let sql = format!(
        "SELECT source_path FROM {table} WHERE ({condition}) AND ({name},id)>=(?{start},?{}) AND ({name},id)<=(?{},?{}) ORDER BY {name},id LIMIT {}",
        start + 1,
        start + 2,
        start + 3,
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
