use super::{
    filters::Editor,
    libraries::Location,
    navigation::{RetainedState, Views},
    pages::Pages,
    selection::Selection,
};
use crate::{
    catalog::{Sample, Scope, SearchPage},
    collections::Reference,
    search::{LibraryQuery, PAGE_SIZE, SearchQuery},
    worker::Workers,
};

#[derive(Default)]
pub(super) struct LibraryView {
    pub selection: Selection,
    pub query_text: String,
    pub query: LibraryQuery,
    pub total: usize,
    pub order: crate::search::SortOrder,
    pub pages: Pages<Sample>,
    pub view_generation: u64,
    pub error: Option<String>,
    pub filter_editor: Editor,
    pub(super) generation: u64,
    location: Location,
    saved: Views<Location, SavedView>,
}

struct SavedView {
    selection: Selection,
    query_text: String,
    total: usize,
    order: crate::search::SortOrder,
    filters: crate::search::filters::MetadataFilters,
    tags: crate::tags::Filter,
}

impl RetainedState for SavedView {
    fn retained_bytes(&self) -> usize {
        self.selection.retained_bytes()
            + self.query_text.capacity()
            + size_of::<crate::search::filters::MetadataFilters>()
            + self.tags.retained_bytes()
    }
}

impl LibraryView {
    pub(super) fn apply_filters(
        &mut self,
        workers: &Workers,
        filters: crate::search::filters::MetadataFilters,
    ) -> crate::Result<Option<usize>> {
        filters.validate()?;
        if self.query.filters == filters {
            return Ok(None);
        }
        self.query.filters = filters;
        Ok(self.reset_query(workers))
    }

    pub(super) fn reset_query(&mut self, workers: &Workers) -> Option<usize> {
        self.selection.reset();
        self.view_generation += 1;
        self.total = 0;
        self.refresh(workers)
    }

    pub(super) fn refresh(&mut self, workers: &Workers) -> Option<usize> {
        self.selection.refresh();
        workers.invalidate_selection(self.selection.generation());
        self.generation += 1;
        self.pages.clear();
        match SearchQuery::parse(&self.query_text) {
            Ok(query) => {
                self.query.text = query;
                self.error = None;
                Some(self.selection.first_visible() / PAGE_SIZE * PAGE_SIZE)
            }
            Err(error) => {
                self.error = Some(error.to_string());
                None
            }
        }
    }

    pub(super) fn reorder(&mut self, workers: &Workers) {
        self.selection.reorder();
        workers.invalidate_selection(self.selection.generation());
        self.generation += 1;
        self.pages.clear();
    }

    pub(super) fn request_positions(&mut self, workers: &Workers, scope: Scope) {
        if let Some(request) = self.selection.position_request()
            && workers.positions(
                request.generation,
                self.query.clone(),
                scope,
                self.order,
                request.active,
                request.anchor,
            )
        {
            self.selection.position_sent();
        }
    }

    pub(super) fn request_page(&mut self, workers: &Workers, scope: Scope, offset: usize) {
        if self.error.is_none()
            && self.pages.needs(offset)
            && workers.query_ordered(
                self.generation,
                self.query.clone(),
                scope,
                self.order,
                offset,
            )
        {
            self.pages.requested(offset);
        }
    }

    pub(super) fn page(
        &mut self,
        generation: u64,
        result: Result<SearchPage, String>,
    ) -> Option<&[Sample]> {
        if generation != self.generation {
            return None;
        }
        match result {
            Ok(page) => {
                self.total = page.total;
                for (position, sample) in page.rows.iter().enumerate() {
                    self.selection.observe(
                        super::SoundKey::Library(sample.id),
                        page.offset + position,
                        &sample.source,
                    );
                    if self.selection.active_source() == Some(sample.source.as_path()) {
                        self.selection.remember(Reference::from(sample));
                    }
                }
                let offset = page.offset;
                self.pages.insert(page);
                self.pages
                    .iter()
                    .find(|page| page.offset == offset)
                    .map(|page| page.rows.as_slice())
            }
            Err(message) => {
                self.pages.clear_pending();
                eprintln!("Sift search failed: {message}");
                self.error = Some(format!("Search failed: {message}"));
                None
            }
        }
    }

    pub(super) fn change(&mut self, location: Location) -> bool {
        if self.location == location {
            return false;
        }
        let previous_generation = self.selection.generation();
        self.selection.suspend();
        let query_text = std::mem::take(&mut self.query_text);
        // A new folder inherits search criteria and ordering within the same
        // bank. Revisiting one restores its settings; a different bank starts fresh.
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
        let initial_filters = if self.location.library == location.library {
            self.query.filters
        } else {
            Default::default()
        };
        let initial_tags = if self.location.library == location.library {
            self.query.tags.clone()
        } else {
            Default::default()
        };
        let previous = std::mem::replace(&mut self.location, location);
        let restored = self.saved.switch(
            previous,
            SavedView {
                selection: std::mem::take(&mut self.selection),
                query_text,
                total: self.total,
                order: self.order,
                filters: self.query.filters,
                tags: self.query.tags.clone(),
            },
            &self.location,
        );
        let restored = restored.unwrap_or_else(|| SavedView {
            selection: Selection::default(),
            query_text: initial_query,
            total: 0,
            order: initial_order,
            filters: initial_filters,
            tags: initial_tags,
        });
        self.selection = restored.selection;
        self.selection.resume(previous_generation);
        self.query_text = restored.query_text;
        self.total = restored.total;
        self.order = restored.order;
        self.query.filters = restored.filters;
        self.query.tags = restored.tags;
        self.filter_editor = Editor::default();
        self.view_generation += 1;
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
