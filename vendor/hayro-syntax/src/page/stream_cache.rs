//! Byte-weighted LRU for decoded page contents, shared by one PDF's pages.
use crate::sync::{Arc, FxHashMap};
use alloc::vec::Vec;

pub(super) const BUDGET: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 512;

type Stream = Option<Arc<Vec<u8>>>;

struct Entry {
    stream: Stream,
    older: Option<usize>,
    newer: Option<usize>,
}

/// Links use page numbers, never addresses into the map. Hits and evictions
/// take a fixed number of map lookups without allocating access-order records.
pub(super) struct StreamCache {
    entries: FxHashMap<usize, Entry>,
    oldest: Option<usize>,
    newest: Option<usize>,
    bytes: usize,
    budget: usize,
}

impl Default for StreamCache {
    fn default() -> Self {
        Self::new(BUDGET)
    }
}

impl StreamCache {
    fn new(budget: usize) -> Self {
        Self {
            entries: FxHashMap::default(),
            oldest: None,
            newest: None,
            bytes: 0,
            budget,
        }
    }

    fn detach(&mut self, page: usize) {
        let entry = &self.entries[&page];
        let (older, newer) = (entry.older, entry.newer);
        if let Some(older) = older {
            self.entries.get_mut(&older).unwrap().newer = newer;
        } else {
            self.oldest = newer;
        }
        if let Some(newer) = newer {
            self.entries.get_mut(&newer).unwrap().older = older;
        } else {
            self.newest = older;
        }
    }

    fn attach_newest(&mut self, page: usize) {
        let entry = self.entries.get_mut(&page).unwrap();
        entry.older = self.newest;
        entry.newer = None;
        if let Some(newest) = self.newest {
            self.entries.get_mut(&newest).unwrap().newer = Some(page);
        } else {
            self.oldest = Some(page);
        }
        self.newest = Some(page);
    }

    /// The outer option distinguishes a cache miss from a cached decode failure.
    pub(super) fn get(&mut self, page: usize) -> Option<Stream> {
        let stream = self.entries.get(&page)?.stream.clone();
        if self.newest != Some(page) {
            self.detach(page);
            self.attach_newest(page);
        }
        Some(stream)
    }

    pub(super) fn insert(&mut self, page: usize, stream: Stream) {
        let bytes = stream.as_ref().map_or(0, |stream| stream.capacity());
        // An oversized page remains usable through its caller's Arc, without
        // flushing useful neighbors or retaining it after the operation ends.
        if bytes > self.budget {
            return;
        }
        if self.entries.contains_key(&page) {
            self.remove(page);
        }
        while self.bytes > self.budget - bytes || self.entries.len() >= MAX_ENTRIES {
            self.remove(self.oldest.unwrap());
        }
        self.entries.insert(
            page,
            Entry {
                stream,
                older: None,
                newer: None,
            },
        );
        self.attach_newest(page);
        self.bytes += bytes;
    }

    fn remove(&mut self, page: usize) {
        self.detach(page);
        let entry = self.entries.remove(&page).unwrap();
        self.bytes -= entry.stream.as_ref().map_or(0, |stream| stream.capacity());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{collections::VecDeque, vec};

    fn stream(bytes: usize) -> Stream {
        Some(Arc::new(vec![42; bytes]))
    }

    #[test]
    fn charges_capacity_and_evicts_least_recently_used_streams() {
        let mut cache = StreamCache::new(10);
        let mut data = Vec::with_capacity(6);
        data.push(1);
        cache.insert(0, Some(Arc::new(data)));
        cache.insert(1, stream(4));
        assert_eq!(cache.bytes, 10);
        cache.get(0).unwrap();
        cache.insert(2, stream(4));
        assert!(cache.get(0).is_some());
        assert!(cache.get(1).is_none());
        assert!(cache.get(2).is_some());
        assert_eq!(cache.bytes, 10);
    }

    #[test]
    fn eviction_and_cache_drop_preserve_active_snapshots_then_release_them() {
        let mut cache = StreamCache::new(4);
        let original = stream(4).unwrap();
        let weak = Arc::downgrade(&original);
        cache.insert(0, Some(original));
        let active = cache.get(0).unwrap().unwrap();
        cache.insert(1, stream(4));
        drop(cache);
        assert_eq!(active.as_slice(), &[42; 4]);
        assert!(weak.upgrade().is_some());
        drop(active);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn oversized_streams_bypass_admission_without_flushing_useful_entries() {
        let mut cache = StreamCache::new(4);
        cache.insert(0, stream(4));
        let large = stream(5).unwrap();
        let weak = Arc::downgrade(&large);
        cache.insert(1, Some(large));
        assert!(cache.get(0).is_some());
        assert!(cache.get(1).is_none());
        assert!(weak.upgrade().is_none());
        assert_eq!(cache.bytes, 4);
    }

    #[test]
    fn empty_and_failed_pages_have_a_bounded_entry_count_too() {
        let mut cache = StreamCache::new(0);
        for page in 0..MAX_ENTRIES * 2 {
            cache.insert(page, if page % 2 == 0 { stream(0) } else { None });
        }
        assert_eq!(cache.bytes, 0);
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.get(0).is_none());
        assert!(matches!(cache.get(MAX_ENTRIES), Some(Some(_))));
        assert!(matches!(cache.get(MAX_ENTRIES + 1), Some(None)));
    }

    #[test]
    fn mixed_hits_replacements_and_multi_evictions_match_a_simple_lru_model() {
        let mut cache = StreamCache::new(97);
        let mut model = VecDeque::<(usize, usize)>::new();
        let mut seed = 17_u64;
        for step in 0..10_000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let page = (seed >> 32) as usize % 71;
            if step % 3 == 0 {
                let found = model.iter().position(|&(key, _)| key == page);
                assert_eq!(cache.get(page).is_some(), found.is_some());
                if let Some(index) = found {
                    let entry = model.remove(index).unwrap();
                    model.push_back(entry);
                }
            } else {
                let bytes = (seed >> 16) as usize % 110;
                cache.insert(page, stream(bytes));
                if bytes <= 97 {
                    model.retain(|&(key, _)| key != page);
                    while model.iter().map(|&(_, bytes)| bytes).sum::<usize>() + bytes > 97 {
                        model.pop_front().unwrap();
                    }
                    model.push_back((page, bytes));
                }
            }
            assert_eq!(cache.bytes, model.iter().map(|&(_, bytes)| bytes).sum());
            assert_eq!(cache.entries.len(), model.len());
            let mut next = cache.oldest;
            let mut previous = None;
            for &(key, bytes) in &model {
                assert_eq!(next, Some(key));
                let entry = &cache.entries[&key];
                assert_eq!(entry.older, previous);
                assert_eq!(entry.stream.as_ref().unwrap().capacity(), bytes);
                previous = Some(key);
                next = entry.newer;
            }
            assert_eq!(next, None);
            assert_eq!(cache.newest, previous);
        }
    }
}
