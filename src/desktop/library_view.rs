use super::{
    libraries::Location,
    navigation::{RetainedState, Views},
    selection::Selection,
};
use crate::search::SearchQuery;

#[derive(Default)]
pub(super) struct LibraryView {
    pub selection: Selection,
    pub query_text: String,
    pub query: SearchQuery,
    pub total: usize,
    pub order: crate::search::SortOrder,
    location: Location,
    saved: Views<Location, SavedView>,
}

struct SavedView {
    selection: Selection,
    query_text: String,
    total: usize,
    order: crate::search::SortOrder,
}

impl RetainedState for SavedView {
    fn retained_bytes(&self) -> usize {
        self.selection.retained_bytes() + self.query_text.capacity()
    }
}

impl LibraryView {
    pub(super) fn change(&mut self, location: Location) -> bool {
        if self.location == location {
            return false;
        }
        let previous_generation = self.selection.generation();
        self.selection.suspend();
        let query_text = std::mem::take(&mut self.query_text);
        // A new folder inherits a search within the same bank. Revisiting one
        // restores its own query; a different bank starts with an empty query.
        let initial_query = if self.location.library == location.library {
            query_text.clone()
        } else {
            String::new()
        };
        let initial_order = if self.location.library == location.library {
            self.order
        } else {
            crate::search::SortOrder::default()
        };
        let previous = std::mem::replace(&mut self.location, location);
        let restored = self.saved.switch(
            previous,
            SavedView {
                selection: std::mem::take(&mut self.selection),
                query_text,
                total: self.total,
                order: self.order,
            },
            &self.location,
        );
        let restored = restored.unwrap_or_else(|| SavedView {
            selection: Selection::default(),
            query_text: initial_query,
            total: 0,
            order: initial_order,
        });
        self.selection = restored.selection;
        self.selection.resume(previous_generation);
        self.query_text = restored.query_text;
        self.total = restored.total;
        self.order = restored.order;
        true
    }

    pub(super) fn retain(&mut self, valid: impl FnMut(&Location) -> bool) {
        self.saved.retain(valid);
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/library_view.rs"
    ));
}
