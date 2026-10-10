use std::path::PathBuf;

use super::DragError;
use crate::{
    catalog::Catalog,
    collections::{ReferenceSelection, SelectionOrigin, Store, data_path},
};

/// Created on the window thread without opening storage. Its lazy read-only
/// connections belong exclusively to the transfer preparation worker.
pub(super) struct SelectionReader {
    path: PathBuf,
    catalog: Option<Catalog>,
    saved: Option<Store>,
}

impl SelectionReader {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            catalog: None,
            saved: None,
        }
    }

    pub(super) fn sources(
        &mut self,
        selection: &ReferenceSelection,
    ) -> Result<Vec<PathBuf>, DragError> {
        let references = match &selection.origin {
            SelectionOrigin::Library { query, scope } => {
                if self.catalog.is_none() {
                    self.catalog = Some(Catalog::open_reader(&self.path).map_err(selection_error)?);
                }
                self.catalog
                    .as_mut()
                    .ok_or_else(unavailable)?
                    .selected_references_ordered(query, scope, selection.order, &selection.sources)
                    .map_err(selection_error)?
            }
            _ => {
                if self.saved.is_none() {
                    self.saved =
                        Some(Store::open_reader(&data_path(&self.path)).map_err(selection_error)?);
                }
                self.saved
                    .as_mut()
                    .ok_or_else(unavailable)?
                    .selected_references(selection)
                    .map_err(selection_error)?
            }
        };
        Ok(references
            .into_iter()
            .map(|reference| reference.source)
            .collect())
    }
}

fn unavailable() -> DragError {
    DragError::Selection("Selection storage is unavailable".into())
}
fn selection_error(error: impl std::fmt::Display) -> DragError {
    DragError::Selection(error.to_string())
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/platform/selection.rs"
    ));
}
