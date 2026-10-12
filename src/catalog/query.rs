use rusqlite::{Transaction, params_from_iter, types::Value};

use super::{Scope, SearchPage, sample_from_row, scope_condition};
use crate::{
    Error, Result,
    search::{
        Boundary, LibraryQuery, OrderKey, PAGE_SIZE, SearchQuery, SortOrder,
        filters::{Channels, Format, Length, MetadataFilters, SampleRate, UnknownLengths},
        selection::OrderedTable,
    },
};

pub(super) struct QueryPlan {
    pub table: OrderedTable,
    pub condition: String,
    pub values: Vec<Value>,
    pub tags: bool,
}

impl QueryPlan {
    pub fn new(query: &LibraryQuery, scope: &Scope) -> Result<Self> {
        query.filters.validate()?;
        let (mut table, condition, mut values) = match &query.text {
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
        let mut condition = if scope == "1" {
            condition.to_owned()
        } else if condition == "1" {
            scope
        } else {
            format!("({condition}) AND ({scope})")
        };
        append_filters(query.filters, &mut condition, &mut values);
        for (index, id) in query.tags.ids().iter().enumerate() {
            values.push(Value::Integer(id.0));
            if index == 0 {
                table = if matches!(table, OrderedTable::SamplesMatchingText) {
                    OrderedTable::TaggedSamplesMatchingText
                } else {
                    OrderedTable::TaggedSamples
                };
                condition = format!("({condition}) AND mt.selected_tag_id=?{}", values.len());
            } else {
                condition = format!(
                    "({condition}) AND EXISTS(SELECT 1 FROM manual_tags.sound_tags st WHERE st.tag_id=?{} AND st.source_path=s.source_path)",
                    values.len()
                );
            }
        }
        Ok(Self {
            table,
            condition,
            values,
            tags: !query.tags.is_empty(),
        })
    }

    pub fn from(&self) -> &'static str {
        self.table.sql()
    }
}

fn append_filters(filters: MetadataFilters, condition: &mut String, values: &mut Vec<Value>) {
    if filters.active_count() == 0 {
        return;
    }
    let mut predicates = Vec::with_capacity(4);
    match filters.length {
        Length::Any => {}
        Length::Unknown => predicates.push("s.duration_seconds IS NULL".to_owned()),
        Length::Range { min, max, unknown } => {
            let mut bounds = Vec::with_capacity(2);
            for (value, operator) in [(min, ">="), (max, "<=")] {
                if let Some(value) = value {
                    values.push(Value::Real(value));
                    bounds.push(format!("s.duration_seconds{operator}?{}", values.len()));
                }
            }
            let range = bounds.join(" AND ");
            predicates.push(match unknown {
                UnknownLengths::Exclude => format!("({range})"),
                UnknownLengths::Include => format!("(s.duration_seconds IS NULL OR ({range}))"),
            });
        }
    }
    if !filters.formats.is_empty() {
        let mut parameters = Vec::with_capacity(5);
        for format in Format::ALL
            .into_iter()
            .filter(|format| filters.formats.contains(*format))
        {
            for extension in format.extensions() {
                values.push(Value::Text((*extension).to_owned()));
                parameters.push(format!("?{}", values.len()));
            }
        }
        predicates.push(format!("s.format IN ({})", parameters.join(",")));
    }
    match filters.channels {
        Channels::Any => {}
        Channels::Unknown => predicates.push("s.channels IS NULL".to_owned()),
        choice => {
            values.push(Value::Integer(if choice == Channels::Mono { 1 } else { 2 }));
            predicates.push(format!("s.channels=?{}", values.len()));
        }
    }
    match filters.sample_rate {
        SampleRate::Any => {}
        SampleRate::Unknown => predicates.push("s.sample_rate IS NULL".to_owned()),
        SampleRate::Hz(rate) => {
            values.push(Value::Integer(i64::from(rate)));
            predicates.push(format!("s.sample_rate=?{}", values.len()));
        }
    }
    if !predicates.is_empty() {
        *condition = format!("({condition}) AND {}", predicates.join(" AND "));
    }
}

const MAX_COUNT_KEY_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct QueryState {
    cached: Option<CachedCount>,
    anchor: Option<PageAnchor>,
}

struct CachedCount {
    from: &'static str,
    condition: String,
    values: Vec<Value>,
    external_revision: i64,
    tag_revision: Option<i64>,
    local_changes: u64,
    total: usize,
}

impl QueryState {
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
        let tag_revision = if plan.tags {
            transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM manual_tags.sound_tags LIMIT 1)",
                [],
                |r| r.get::<_, bool>(0),
            )?;
            Some(
                transaction.pragma_query_value(Some("manual_tags"), "data_version", |r| {
                    r.get::<_, i64>(0)
                })?,
            )
        } else {
            None
        };
        if let Some(cached) = &self.cached
            && cached.external_revision == external_revision
            && cached.local_changes == local_changes
            && cached.tag_revision == tag_revision
            && cached.from == plan.from()
            && cached.condition == plan.condition
            && cached.values == plan.values
        {
            return Ok(cached.total);
        }
        self.anchor = None;
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
                tag_revision,
                local_changes,
                total,
            })
        } else {
            None
        };
        Ok(total)
    }
}

const MAX_PAGE_KEY_BYTES: usize = 2048;

struct PageAnchor {
    order: SortOrder,
    offset: usize,
    length: usize,
    first: OrderKey,
    last: OrderKey,
}

impl PageAnchor {
    fn within_budget(&self) -> bool {
        [&self.first, &self.last].iter().all(|key| {
            let primary = match &key.value {
                Value::Text(text) => text.len(),
                _ => size_of::<Value>(),
            };
            key.name.len().saturating_add(primary) <= MAX_PAGE_KEY_BYTES
        })
    }
}

impl QueryState {
    pub fn page(
        &mut self,
        transaction: &Transaction<'_>,
        plan: &QueryPlan,
        order: SortOrder,
        offset: usize,
        total: usize,
    ) -> Result<SearchPage> {
        let offset_sql = i64::try_from(offset).map_err(|_| Error::OutOfRange)?;
        if offset >= total {
            return Ok(SearchPage {
                total,
                offset,
                rows: vec![],
            });
        }
        let mut values = plan.values.clone();
        // An unrestricted nullable index's OFFSET scan beat the disjoint
        // known/NULL seek in the 100k comparison. Keep that indexed path.
        let direct_nullable = plan.condition == "1"
            && matches!(
                order.field,
                crate::search::SortField::Length
                    | crate::search::SortField::Rate
                    | crate::search::SortField::Channels
            );
        let anchor = self
            .anchor
            .as_ref()
            .filter(|anchor| anchor.order == order && self.cached.is_some() && !direct_nullable);
        let (condition, traversal, skip) = if let Some(anchor) = anchor
            && anchor.offset.checked_add(anchor.length) == Some(offset)
        {
            let after = order.boundary(&anchor.last, Boundary::After, &mut values);
            (
                format!("({}) AND ({after})", plan.condition),
                order.sql(),
                0,
            )
        } else if let Some(anchor) = anchor
            && offset.checked_add(PAGE_SIZE) == Some(anchor.offset)
        {
            let before = order.boundary(&anchor.first, Boundary::Before, &mut values);
            (
                format!("({}) AND ({before})", plan.condition),
                order.reverse_sql(),
                0,
            )
        } else {
            // Counts and page membership share the verified snapshot. Start
            // from the nearer end for random access; re-sort the bounded page
            // into its visible order after the metadata lookup.
            let length = (total - offset).min(PAGE_SIZE);
            let reverse_offset = total - offset - length;
            if reverse_offset < offset {
                (
                    plan.condition.clone(),
                    order.reverse_sql(),
                    i64::try_from(reverse_offset).map_err(|_| Error::OutOfRange)?,
                )
            } else {
                (plan.condition.clone(), order.sql(), offset_sql)
            }
        };
        let limit_parameter = values.len() + 1;
        let skip_parameter = values.len() + 2;
        values.extend([
            Value::Integer((total - offset).min(PAGE_SIZE) as i64),
            Value::Integer(skip),
        ]);
        let sql=format!(
            "WITH page AS MATERIALIZED (SELECT s.id AS sample_id FROM {} WHERE {condition} ORDER BY {traversal} LIMIT ?{limit_parameter} OFFSET ?{skip_parameter})
             SELECT s.id,s.source_path,s.name,s.path,s.format,s.size_bytes,s.duration_seconds,s.sample_rate,s.channels,s.bit_depth,s.metadata_error,s.modified_ns,s.created_ns,s.name_key,{}
             FROM page JOIN samples s ON s.id=page.sample_id ORDER BY {}",
            plan.from(),order.column(),order.sql()
        );
        let mut statement = transaction.prepare_cached(&sql)?;
        let mut rows = statement.query(params_from_iter(&values))?;
        let mut result = Vec::with_capacity(PAGE_SIZE);
        let mut first = None;
        let mut last = None;
        while let Some(row) = rows.next()? {
            let sample = sample_from_row(row)?;
            let key = OrderKey {
                value: row.get(14)?,
                name: row.get(13)?,
                id: sample.id.0,
            };
            if first.is_none() {
                first = Some(OrderKey {
                    value: key.value.clone(),
                    name: key.name.clone(),
                    id: key.id,
                });
            }
            last = Some(key);
            result.push(sample);
        }
        self.anchor = match (first, last) {
            (Some(first), Some(last)) if self.cached.is_some() => {
                let anchor = PageAnchor {
                    order,
                    offset,
                    length: result.len(),
                    first,
                    last,
                };
                anchor.within_budget().then_some(anchor)
            }
            _ => None,
        };
        Ok(SearchPage {
            total,
            offset,
            rows: result,
        })
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/catalog/query.rs"
    ));
}
