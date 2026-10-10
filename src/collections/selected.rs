use rusqlite::types::Value;

use super::{CollectionId, Reference, Result, Store};
use crate::{
    catalog::Scope,
    search::{SearchQuery, SourceSelection},
};

#[derive(Clone, Debug)]
pub enum SelectionOrigin {
    Library { query: SearchQuery, scope: Scope },
    Favorites { query: SearchQuery },
    Collection(CollectionId),
}

#[derive(Clone, Debug)]
pub struct ReferenceSelection {
    pub origin: SelectionOrigin,
    pub sources: SourceSelection,
}

impl Store {
    pub fn selected_references(
        &mut self,
        selection: &ReferenceSelection,
    ) -> Result<Vec<Reference>> {
        let (table, order, condition, values) = match &selection.origin {
            SelectionOrigin::Favorites { query } => {
                let (condition, values) = super::favorites::favorite_condition(query);
                ("favorites", "name_key,id", condition, values)
            }
            SelectionOrigin::Collection(collection) => (
                "entries",
                "id",
                "collection_id=?1",
                vec![Value::Integer(collection.0)],
            ),
            SelectionOrigin::Library { .. } => return Err(super::Error::CatalogUnavailable),
        };
        let transaction = self.connection.transaction()?;
        let identities = crate::search::batch::selected_rows(
            &transaction,
            table,
            order,
            condition,
            values,
            &selection.sources,
        )?;
        let references = {
            let mut statement = transaction.prepare(&format!("SELECT id,source_path,name,display_path,format,size_bytes,modified_ns,duration_seconds,sample_rate,channels,bit_depth,metadata_error FROM {table} WHERE id=?1"))?;
            let mut references = Vec::with_capacity(identities.len());
            for (id, _) in identities {
                let mut rows = statement.query([id])?;
                let row = rows.next()?.ok_or(super::Error::NotFound)?;
                references.push(super::read_reference(row)?);
            }
            references
        };
        transaction.commit()?;
        Ok(references)
    }

    /// Resolve membership and delete in one transaction. Source availability is
    /// irrelevant to removing references; no filesystem operations occur here.
    pub fn remove_selected(
        &mut self,
        collection: CollectionId,
        selection: &SourceSelection,
    ) -> Result<Vec<std::path::PathBuf>> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let identities = crate::search::batch::selected_rows(
            &transaction,
            "entries",
            "id",
            "collection_id=?1",
            vec![Value::Integer(collection.0)],
            selection,
        )?;
        {
            let mut statement =
                transaction.prepare("DELETE FROM entries WHERE collection_id=?1 AND id=?2")?;
            for (id, _) in &identities {
                if statement.execute(rusqlite::params![collection.0, id])? != 1 {
                    return Err(super::Error::NotFound);
                }
            }
        }
        transaction.commit()?;
        Ok(identities.into_iter().map(|(_, source)| source).collect())
    }
}
