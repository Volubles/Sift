use std::{collections::HashSet, path::PathBuf};

use rusqlite::{Transaction, params_from_iter, types::Value};

use super::{MAX_ACTION_SOUNDS, SelectionError, SourceSelection};
use crate::{path_codec, search::selection::MAX_SELECTION_BYTES};

/// Return every selected identity or fail. Callers read metadata or delete rows
/// in this same snapshot; cached UI pages are never the operation's input.
pub(crate) fn selected_rows(
    transaction: &Transaction<'_>,
    table: &str,
    order: &str,
    condition: &str,
    mut values: Vec<Value>,
    selection: &SourceSelection,
) -> Result<Vec<(i64, PathBuf)>, SelectionError> {
    let (all, paths) = match selection {
        SourceSelection::Only(paths) => (false, paths),
        SourceSelection::AllExcept(paths) => (true, paths),
    };
    if paths.len() > crate::search::selection::MAX_SELECTION_KEYS {
        return Err(SelectionError::InvalidPaths);
    }
    if !all && (paths.is_empty() || paths.len() > MAX_ACTION_SOUNDS) {
        return Err(SelectionError::InvalidSize);
    }
    let mut encoded = HashSet::new();
    let mut bytes = 0;
    for path in paths {
        let raw = path_codec::encode(path);
        bytes += raw.len();
        if !path.is_absolute() || raw.len() > 65536 || bytes > MAX_SELECTION_BYTES {
            return Err(SelectionError::InvalidPaths);
        }
        encoded.insert(raw);
    }
    let membership = if all {
        String::new()
    } else {
        let parameters: Vec<_> = encoded
            .iter()
            .map(|path| {
                values.push(Value::Blob(path.clone()));
                format!("?{}", values.len())
            })
            .collect();
        format!(" AND source_path IN ({})", parameters.join(","))
    };
    let mut statement = transaction.prepare(&format!(
        "SELECT id,source_path FROM {table} WHERE ({condition}){membership} ORDER BY {order}"
    ))?;
    let mut rows = statement.query(params_from_iter(&values))?;
    let mut selected = Vec::new();
    while let Some(row) = rows.next()? {
        let raw: Vec<u8> = row.get(1)?;
        if raw.len() > 65536 {
            return Err(SelectionError::InvalidPaths);
        }
        if all && encoded.contains(&raw) {
            continue;
        }
        if selected.len() == MAX_ACTION_SOUNDS {
            return Err(SelectionError::InvalidSize);
        }
        let path = path_codec::decode(raw).map_err(|_| SelectionError::InvalidPaths)?;
        selected.push((row.get(0)?, path));
    }
    if !all && selected.len() != encoded.len() {
        return Err(SelectionError::Changed);
    }
    if selected.is_empty() {
        return Err(SelectionError::InvalidSize);
    }
    Ok(selected)
}
