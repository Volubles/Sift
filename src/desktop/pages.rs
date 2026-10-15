use std::collections::{BTreeSet, VecDeque};

use crate::search::{MAX_CACHED_PAGES, Page};

/// Requested pages in arrival order. Selection, query generations and errors
/// belong to the view; a refresh discards both cached pages and pending reads.
pub(super) struct Pages<T> {
    entries: VecDeque<Page<T>>,
    pending: BTreeSet<usize>,
}

impl<T> Default for Pages<T> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            pending: BTreeSet::new(),
        }
    }
}

impl<T> Pages<T> {
    pub(super) fn iter(&self) -> impl Iterator<Item = &Page<T>> {
        self.entries.iter()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(super) fn needs(&self, offset: usize) -> bool {
        !self.pending.contains(&offset) && !self.entries.iter().any(|page| page.offset == offset)
    }

    pub(super) fn requested(&mut self, offset: usize) {
        self.pending.insert(offset);
    }

    pub(super) fn insert(&mut self, page: Page<T>) {
        self.pending.remove(&page.offset);
        self.entries.retain(|cached| cached.offset != page.offset);
        self.entries.push_back(page);
        // Revisiting a lower offset must keep that new page, not the highest
        // numerical offsets. No cached page owns an entire result set.
        while self.entries.len() > MAX_CACHED_PAGES {
            self.entries.pop_front();
        }
    }

    pub(super) fn clear_pending(&mut self) {
        self.pending.clear();
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.clear_pending();
    }
}
