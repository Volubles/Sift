use super::{Error, Filter, MAX_NAME_CHARS, Result, Store, Tag, TagId, TagUse, read_tag};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

pub const MANAGEMENT_PAGE_SIZE: usize = 32;

/// An opaque keyset cursor. Display names never become source-file locators.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    name_key: String,
    id: TagId,
}

#[derive(Clone, Debug)]
pub struct Page {
    pub rows: Vec<TagUse>,
    pub next: Option<Cursor>,
}

#[derive(Clone, Debug)]
pub enum Mutation {
    Rename { id: TagId, name: String },
    Merge { source: TagId, target: TagId },
    Delete(TagId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Renamed(Tag),
    Merged { removed: TagId, kept: Tag },
    Deleted(TagId),
}

impl Change {
    /// Includes filters containing only the merge destination: its result set grows too.
    pub fn affects(&self, filter: &Filter) -> bool {
        match self {
            Self::Renamed(_) => false,
            Self::Merged { removed, kept } => filter.contains(*removed) || filter.contains(kept.id),
            Self::Deleted(id) => filter.contains(*id),
        }
    }

    pub fn apply(&self, filter: &mut Filter) {
        let (removed, replacement) = match self {
            Self::Renamed(_) => return,
            Self::Merged { removed, kept } => (*removed, Some(kept.id)),
            Self::Deleted(id) => (*id, None),
        };
        if let Ok(index) = filter.0.binary_search(&removed) {
            filter.0.remove(index);
            if let Some(id) = replacement
                && let Err(index) = filter.0.binary_search(&id)
            {
                // Replacement cannot grow a valid bounded filter.
                filter.0.insert(index, id);
            }
        }
    }
}

impl Store {
    pub fn tag_page(&mut self, prefix: &str, after: Option<&Cursor>) -> Result<Page> {
        if prefix.chars().count() > MAX_NAME_CHARS || prefix.chars().any(char::is_control) {
            return Err(Error::InvalidName);
        }
        let lower = prefix.trim().to_lowercase();
        let upper = crate::search::prefix_successor(&lower);
        let transaction = self.connection.transaction()?;
        let mut rows = {
            let mut statement = transaction.prepare(
                "SELECT id,name,name_key,(SELECT count(*) FROM sound_tags WHERE tag_id=tags.id)
                 FROM tags WHERE name_key>=?1 AND (?2 IS NULL OR name_key<?2)
                 AND (?3 IS NULL OR (name_key,id)>(?3,?4))
                 ORDER BY name_key,id LIMIT ?5",
            )?;
            statement
                .query_map(
                    params![
                        lower,
                        upper,
                        after.map(|cursor| &cursor.name_key),
                        after.map(|cursor| cursor.id.0),
                        (MANAGEMENT_PAGE_SIZE + 1) as i64,
                    ],
                    |row| {
                        let count: i64 = row.get(3)?;
                        let count = usize::try_from(count).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                3,
                                rusqlite::types::Type::Integer,
                                Box::new(error),
                            )
                        })?;
                        let used = TagUse {
                            tag: read_tag(row)?,
                            count,
                        };
                        let cursor = Cursor {
                            name_key: row.get(2)?,
                            id: used.tag.id,
                        };
                        Ok((used, cursor))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let has_more = rows.len() > MANAGEMENT_PAGE_SIZE;
        rows.truncate(MANAGEMENT_PAGE_SIZE);
        let next = has_more
            .then(|| rows.last().map(|(_, cursor)| cursor.clone()))
            .flatten();
        let rows = rows.into_iter().map(|(used, _)| used).collect();
        transaction.commit()?;
        Ok(Page { rows, next })
    }

    /// Definition changes and all their associations commit atomically on the tag owner.
    pub fn manage(&mut self, mutation: &Mutation) -> Result<Change> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let find = |id: TagId| -> Result<Tag> {
            if id.0 <= 0 {
                return Err(Error::InvalidTag);
            }
            transaction
                .query_row("SELECT id,name FROM tags WHERE id=?1", [id.0], read_tag)
                .optional()?
                .ok_or(Error::NotFound)
        };
        let change = match mutation {
            Mutation::Rename { id, name } => {
                find(*id)?;
                let name = crate::names::normalize(name)
                    .filter(|name| name.chars().count() <= MAX_NAME_CHARS)
                    .ok_or(Error::InvalidName)?;
                let key = name.to_lowercase();
                let exists: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM tags WHERE name_key=?1 AND id<>?2)",
                    params![key, id.0],
                    |row| row.get(0),
                )?;
                if exists {
                    return Err(Error::NameExists);
                }
                transaction.execute(
                    "UPDATE tags SET name=?1,name_key=?2 WHERE id=?3",
                    params![name, key, id.0],
                )?;
                Change::Renamed(Tag {
                    id: *id,
                    name: name.to_owned(),
                })
            }
            Mutation::Merge { source, target } => {
                if source == target {
                    return Err(Error::SameTag);
                }
                find(*source)?;
                let kept = find(*target)?;
                // Replace one identity with another. Overlap is deduplicated, so final
                // per-source membership can never exceed its pre-merge size.
                transaction.execute(
                    "INSERT INTO sound_tags(source_path,tag_id)
                     SELECT source_path,?1 FROM sound_tags WHERE tag_id=?2
                     ON CONFLICT(source_path,tag_id) DO NOTHING",
                    params![target.0, source.0],
                )?;
                transaction.execute("DELETE FROM tags WHERE id=?1", [source.0])?;
                Change::Merged {
                    removed: *source,
                    kept,
                }
            }
            Mutation::Delete(id) => {
                find(*id)?;
                transaction.execute("DELETE FROM tags WHERE id=?1", [id.0])?;
                Change::Deleted(*id)
            }
        };
        transaction.commit()?;
        Ok(change)
    }
}
