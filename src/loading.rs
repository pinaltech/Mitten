//! UI-independent limits shared by the grid and stress benchmarks.
use std::collections::{HashMap, VecDeque};

pub const PAGE_SIZE: usize = 200;
pub const THUMBNAIL_JOBS: usize = 4;
pub const THUMBNAIL_CACHE: usize = 256;
pub const VIEWER_CACHE: usize = 3;

/// Bounded access-ordered cache. Values are paths, never decoded originals.
pub struct BoundedCache<T> {
    capacity: usize,
    values: HashMap<String, T>,
    order: VecDeque<String>,
}
impl<T> BoundedCache<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            capacity,
            values: HashMap::new(),
            order: VecDeque::new(),
        }
    }
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
        while self.values.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.values.remove(&old);
            }
        }
    }
    pub fn peek(&self, key: &str) -> Option<&T> {
        self.values.get(key)
    }
    pub fn get(&mut self, key: &str) -> Option<&T> {
        if self.values.contains_key(key) {
            self.order.retain(|k| k != key);
            self.order.push_back(key.to_owned());
        }
        self.values.get(key)
    }
    pub fn contains_key(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }
    pub fn insert(&mut self, key: String, value: T) {
        self.order.retain(|k| k != &key);
        self.order.push_back(key.clone());
        self.values.insert(key, value);
        while self.values.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.values.remove(&old);
            }
        }
    }
    pub fn clear(&mut self) {
        self.values.clear();
        self.order.clear();
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Convert just the requested virtual rows to a bounded photo range.
pub fn visible_range(
    rows: std::ops::Range<usize>,
    columns: usize,
    count: usize,
) -> std::ops::Range<usize> {
    rows.start.saturating_mul(columns).min(count)..rows.end.saturating_mul(columns).min(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scrolling_a_million_photos_keeps_caches_bounded() {
        let mut thumbs = BoundedCache::new(THUMBNAIL_CACHE);
        let mut previews = BoundedCache::new(VIEWER_CACHE);
        for row in (0..200_000).step_by(7) {
            let range = visible_range(row..row + 7, 5, 1_000_000);
            assert!(range.len() <= 35);
            for id in range {
                thumbs.insert(id.to_string(), id);
            }
            assert!(thumbs.len() <= THUMBNAIL_CACHE);
            assert!(
                previews.is_empty(),
                "scrolling must never request a viewer image"
            );
        }
        for id in 0..100 {
            previews.insert(id.to_string(), id);
        }
        assert_eq!(previews.len(), VIEWER_CACHE);
    }
    #[test]
    fn least_recently_used_entries_are_evicted() {
        let mut c = BoundedCache::new(2);
        c.insert("a".into(), 1);
        c.insert("b".into(), 2);
        assert_eq!(c.get("a"), Some(&1));
        c.insert("c".into(), 3);
        assert!(!c.contains_key("b"));
        assert!(c.contains_key("a"));
    }
}
