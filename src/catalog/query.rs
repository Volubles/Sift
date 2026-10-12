use rusqlite::{Transaction, params_from_iter, types::Value};

use super::{Scope, SearchPage, sample_from_row, scope_condition};
use crate::{
    Error, Result,
    search::{PAGE_SIZE, SearchQuery, SortOrder, selection::OrderedTable},
};

pub(super) struct QueryPlan {
    pub table: OrderedTable,
    pub condition: String,
    pub values: Vec<Value>,
}

impl QueryPlan {
    pub fn new(query: &SearchQuery, scope: &Scope) -> Result<Self> {
        let (table, condition, mut values) = match query {
            SearchQuery::All => (OrderedTable::Samples, "1", vec![]),
            SearchQuery::FilenamePrefix { lower, upper } => (
                OrderedTable::Samples,
                "s.name_key>=?1 AND s.name_key<?2",
                vec![Value::Text(lower.clone()), Value::Text(upper.clone())],
            ),
            SearchQuery::Substrings(expression) => (
                OrderedTable::SamplesMatchingText,
                "samples_fts MATCH ?1",
                vec![Value::Text(expression.clone())],
            ),
        };
        let scope = scope_condition(scope, &mut values)?;
        let condition = if scope == "1" {
            condition.to_owned()
        } else if condition == "1" {
            scope
        } else {
            format!("({condition}) AND ({scope})")
        };
        Ok(Self {
            table,
            condition,
            values,
        })
    }

    pub fn from(&self) -> &'static str {
        self.table.sql()
    }
}

const MAX_COUNT_KEY_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct QueryCount {
    cached: Option<CachedCount>,
}

struct CachedCount {
    from: &'static str,
    condition: String,
    values: Vec<Value>,
    external_revision: i64,
    local_changes: u64,
    total: usize,
}

impl QueryCount {
    /// Read a main-database snapshot before the connection-local revision.
    /// Both external commits and this connection's own writes invalidate counts.
    pub fn get(&mut self, transaction: &Transaction<'_>, plan: &QueryPlan) -> Result<usize> {
        transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.samples LIMIT 1)",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        let external_revision =
            transaction
                .pragma_query_value(Some("main"), "data_version", |row| row.get::<_, i64>(0))?;
        let local_changes = transaction.total_changes();
        if let Some(cached) = &self.cached
            && cached.external_revision == external_revision
            && cached.local_changes == local_changes
            && cached.from == plan.from()
            && cached.condition == plan.condition
            && cached.values == plan.values
        {
            return Ok(cached.total);
        }
        let sql = if plan.condition == "1" {
            format!("SELECT count(*) FROM {}", plan.from())
        } else {
            format!(
                "SELECT count(*) FROM {} WHERE {}",
                plan.from(),
                plan.condition
            )
        };
        let count: i64 =
            transaction.query_row(&sql, params_from_iter(&plan.values), |row| row.get(0))?;
        let total = usize::try_from(count).map_err(|_| Error::OutOfRange)?;
        // Compare generated predicates and native BLOB bindings, not Path equality:
        // Windows path components can compare equal while locator bytes differ.
        let bytes = plan
            .values
            .iter()
            .fold(plan.condition.len(), |size, value| {
                size.saturating_add(match value {
                    Value::Text(text) => text.len(),
                    Value::Blob(blob) => blob.len(),
                    _ => size_of::<Value>(),
                })
                .saturating_add(size_of::<Value>())
            });
        self.cached = if bytes <= MAX_COUNT_KEY_BYTES {
            Some(CachedCount {
                from: plan.from(),
                condition: plan.condition.clone(),
                values: plan.values.clone(),
                external_revision,
                local_changes,
                total,
            })
        } else {
            None
        };
        Ok(total)
    }
}

pub(super) fn page(
    transaction: &Transaction<'_>,
    plan: &QueryPlan,
    order: SortOrder,
    offset: usize,
    total: usize,
) -> Result<SearchPage> {
    let offset_sql = i64::try_from(offset).map_err(|_| Error::OutOfRange)?;
    let mut values = plan.values.clone();
    let limit = values.len() + 1;
    let skip = values.len() + 2;
    values.extend([Value::Integer(PAGE_SIZE as i64), Value::Integer(offset_sql)]);
    let order = order.sql();
    // Sort compact IDs first. Fetch wide metadata only for the bounded page;
    // the final order also makes join output deterministic.
    let sql=format!("WITH page AS MATERIALIZED (SELECT s.id AS sample_id FROM {} WHERE {} ORDER BY {order} LIMIT ?{limit} OFFSET ?{skip})
        SELECT s.id,s.source_path,s.name,s.path,s.format,s.size_bytes,s.duration_seconds,s.sample_rate,s.channels,s.bit_depth,s.metadata_error,s.modified_ns,s.created_ns
        FROM page JOIN samples s ON s.id=page.sample_id ORDER BY {order}",plan.from(),plan.condition);
    let mut statement = transaction.prepare_cached(&sql)?;
    let mut rows = statement.query(params_from_iter(&values))?;
    let mut result = Vec::with_capacity(PAGE_SIZE);
    while let Some(row) = rows.next()? {
        result.push(sample_from_row(row)?);
    }
    Ok(SearchPage {
        total,
        offset,
        rows: result,
    })
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/catalog/query.rs"
    ));
}
