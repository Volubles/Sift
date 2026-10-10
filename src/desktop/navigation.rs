use std::collections::VecDeque;

const MAX_VIEWS: usize = 16;
const MAX_RETAINED_BYTES: usize = 8 * 1024 * 1024;

pub(super) trait RetainedState {
    fn retained_bytes(&self) -> usize;
}

/// Inactive session views only. Workers, result pages, and pending edits stay
/// with their owners; returning to a view always requests a fresh SQL snapshot.
pub(super) struct Views<K, V> {
    entries: VecDeque<(K, V)>,
    bytes: usize,
}

impl<K, V> Default for Views<K, V> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
        }
    }
}

impl<K: PartialEq, V: RetainedState> Views<K, V> {
    pub(super) fn switch(&mut self, previous: K, state: V, next: &K) -> Option<V> {
        // Protect a cache hit before saving the departed view can evict it.
        let restored = self.take(next);
        self.remember(previous, state);
        restored
    }

    pub(super) fn remember(&mut self, key: K, value: V) {
        drop(self.take(&key));
        let bytes = value.retained_bytes();
        if bytes > MAX_RETAINED_BYTES {
            return;
        }
        while self.entries.len() >= MAX_VIEWS || self.bytes + bytes > MAX_RETAINED_BYTES {
            if let Some((_, oldest)) = self.entries.pop_front() {
                self.bytes -= oldest.retained_bytes();
            }
        }
        self.bytes += bytes;
        self.entries.push_back((key, value));
    }

    pub(super) fn take(&mut self, key: &K) -> Option<V> {
        let index = self.entries.iter().position(|(stored, _)| stored == key)?;
        let (_, value) = self.entries.remove(index)?;
        self.bytes -= value.retained_bytes();
        Some(value)
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.entries.retain(|(key, _)| keep(key));
        self.bytes = self
            .entries
            .iter()
            .map(|(_, value)| value.retained_bytes())
            .sum();
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/desktop/navigation.rs"
    ));
}
