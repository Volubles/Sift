use rusqlite::types::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortField {
    #[default]
    Filename,
    Format,
    Length,
    Rate,
    Channels,
    Added,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SortOrder {
    pub field: SortField,
    pub direction: SortDirection,
}

pub(crate) enum Boundary {
    Before,
    After,
    AtOrBefore,
    AtOrAfter,
}

enum Traversal {
    Forward,
    Backward,
}

impl SortOrder {
    pub const INSERTION: Self = Self {
        field: SortField::Added,
        direction: SortDirection::Ascending,
    };

    pub fn toggle(&mut self, field: SortField) {
        self.direction = if self.field == field && self.direction == SortDirection::Ascending {
            SortDirection::Descending
        } else {
            SortDirection::Ascending
        };
        self.field = field;
    }

    pub(crate) fn column(self) -> &'static str {
        match self.field {
            SortField::Filename => "name_key",
            SortField::Format => "format",
            SortField::Length => "duration_seconds",
            SortField::Rate => "sample_rate",
            SortField::Channels => "channels",
            SortField::Added => "id",
        }
    }

    /// Only enum-selected identifiers enter SQL. Reversing the whole key gives
    /// deterministic ties and lets the same index serve both directions.
    pub(crate) fn sql(self) -> String {
        self.sql_for(Traversal::Forward)
    }

    pub(crate) fn reverse_sql(self) -> String {
        self.sql_for(Traversal::Backward)
    }

    fn sql_for(self, traversal: Traversal) -> String {
        let direction = match (self.direction, &traversal) {
            (SortDirection::Ascending, Traversal::Forward)
            | (SortDirection::Descending, Traversal::Backward) => "ASC",
            _ => "DESC",
        };
        let unknowns = match traversal {
            Traversal::Forward => "LAST",
            Traversal::Backward => "FIRST",
        };
        let column = self.column();
        match self.field {
            SortField::Added => format!("id {direction}"),
            SortField::Filename => format!("name_key {direction},id {direction}"),
            _ => {
                format!("{column} {direction} NULLS {unknowns},name_key {direction},id {direction}")
            }
        }
    }

    /// A lexicographic boundary with unknown primary values always last.
    /// The same rule owns range selection and relocation of off-page cursors.
    pub(crate) fn boundary(
        self,
        key: &OrderKey,
        boundary: Boundary,
        values: &mut Vec<Value>,
    ) -> String {
        let before = matches!(boundary, Boundary::Before | Boundary::AtOrBefore);
        let inclusive = matches!(boundary, Boundary::AtOrBefore | Boundary::AtOrAfter);
        let column = self.column();
        let less = before == (self.direction == SortDirection::Ascending);
        let strict = if less { "<" } else { ">" };
        let comparison = match (less, inclusive) {
            (true, true) => "<=",
            (false, true) => ">=",
            _ => strict,
        };
        let parameter = values.len() + 1;
        match self.field {
            SortField::Filename => {
                values.extend([Value::Text(key.name.clone()), Value::Integer(key.id)]);
                return format!("(name_key,id){comparison}(?{parameter},?{})", parameter + 1);
            }
            SortField::Added => {
                values.push(Value::Integer(key.id));
                return format!("id{comparison}?{parameter}");
            }
            _ => {}
        }
        values.extend([
            key.value.clone(),
            Value::Text(key.name.clone()),
            Value::Integer(key.id),
        ]);
        let tie = format!(
            "(name_key,id){comparison}(?{},?{})",
            parameter + 1,
            parameter + 2
        );
        if key.value == Value::Null {
            let same = format!("({column} IS NULL AND {tie})");
            if before {
                format!("({column} IS NOT NULL OR {same})")
            } else {
                same
            }
        } else {
            let known = format!(
                "({column},name_key,id){comparison}(?{parameter},?{},?{})",
                parameter + 1,
                parameter + 2
            );
            if before || self.field == SortField::Format {
                known
            } else {
                format!("({column} IS NULL OR {known})")
            }
        }
    }
}

pub(crate) struct OrderKey {
    pub value: Value,
    pub name: String,
    pub id: i64,
}

#[derive(Clone, Debug, Default)]
pub struct SelectionPositions {
    pub active: Option<usize>,
    pub anchor: Option<usize>,
}
