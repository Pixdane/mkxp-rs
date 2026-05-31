//! SE buffer cache — avoids re-decoding frequently played sound effects.
//!
//! mkxp-z maintains a 10MB LRU cache of decoded OpenAL buffers for SE playback.
//! We cache the raw encoded bytes rather than decoded PCM, because kira's
//! `StaticSoundData` is consumed on play (cannot be shared).  The cache avoids
//! repeated file-system reads; symphonia decoding is fast enough for small SE
//! files that re-decoding on each play is cheap.
//!
//! Cache eviction is LRU: the least recently used entry is evicted when the
//! total stored size exceeds `max_bytes`.

use std::collections::{HashMap, VecDeque};

/// Default cache size matching mkxp-z's `SE_CACHE_MEM` (10 MB).
pub const DEFAULT_SE_CACHE_BYTES: usize = 10 * 1024 * 1024;

/// Least-recently-used cache for SE raw audio data.
///
/// Maps file path → encoded bytes, with a size cap and LRU eviction.
#[derive(Debug)]
pub struct SeCache {
    /// LRU order: front = most recently used, back = least recently used.
    order: VecDeque<String>,
    /// Cached data keyed by file path.
    entries: HashMap<String, Vec<u8>>,
    /// Current total bytes stored.
    current_bytes: usize,
    /// Maximum total bytes before eviction.
    max_bytes: usize,
}

impl SeCache {
    /// Create a cache with the given byte limit.
    pub fn new(max_bytes: usize) -> Self {
        Self {
            order: VecDeque::new(),
            entries: HashMap::new(),
            current_bytes: 0,
            max_bytes,
        }
    }

    /// Look up a cached entry by path.  Returns `None` on miss.
    /// On hit, the entry is moved to the front of the LRU order.
    pub fn get(&mut self, path: &str) -> Option<&[u8]> {
        if self.entries.contains_key(path) {
            // Move to front of LRU
            self.order.retain(|p| p != path);
            self.order.push_front(path.to_string());
            Some(self.entries[path].as_slice())
        } else {
            None
        }
    }

    /// Insert an entry.  If the path already exists, it is replaced
    /// (and bytes are updated accordingly).  If the total size after
    /// insertion exceeds `max_bytes`, LRU entries are evicted until
    /// the limit is satisfied — but the newly inserted entry is
    /// never evicted.
    pub fn insert(&mut self, path: &str, data: Vec<u8>) {
        let data_len = data.len();

        // Remove old entry for this path if present
        if let Some(old) = self.entries.remove(path) {
            self.current_bytes = self.current_bytes.saturating_sub(old.len());
            self.order.retain(|p| p != path);
        }

        // Evict LRU entries if needed
        while self.current_bytes + data_len > self.max_bytes && !self.order.is_empty() {
            if let Some(evict_path) = self.order.pop_back() {
                if let Some(evicted) = self.entries.remove(&evict_path) {
                    self.current_bytes = self.current_bytes.saturating_sub(evicted.len());
                }
            }
        }

        self.current_bytes += data_len;
        self.order.push_front(path.to_string());
        self.entries.insert(path.to_string(), data);
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
        self.current_bytes = 0;
    }

    /// Number of cached entries.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Current total bytes stored.
    #[allow(dead_code)]
    pub fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    /// Returns `true` if the cache is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for SeCache {
    fn default() -> Self {
        Self::new(DEFAULT_SE_CACHE_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_returns_data() {
        let mut cache = SeCache::new(1024);
        cache.insert("se1.ogg", vec![1, 2, 3]);
        assert_eq!(cache.get("se1.ogg"), Some(&[1, 2, 3][..]));
    }

    #[test]
    fn cache_miss_returns_none() {
        let mut cache = SeCache::new(1024);
        assert_eq!(cache.get("nonexistent"), None);
    }

    #[test]
    fn cache_hit_moves_to_front() {
        // Cache with 3-byte limit: exactly a(1) + b(2) = 3 bytes.
        // Inserting c(1) after touching a should evict b (LRU).
        let mut cache = SeCache::new(3);
        cache.insert("a.ogg", vec![1]);       // 1 byte
        cache.insert("b.ogg", vec![2, 2]);    // 2 bytes, total = 3
        cache.get("a.ogg"); // touch a → LRU order: a, b
        cache.insert("c.ogg", vec![3]);       // 1 byte → evict LRU (b)
        assert!(cache.get("a.ogg").is_some()); // a survived (was MRU)
        assert!(cache.get("b.ogg").is_none());  // b was LRU, evicted
        assert!(cache.get("c.ogg").is_some()); // c inserted
    }

    #[test]
    fn lru_eviction_respects_limit() {
        let mut cache = SeCache::new(3); // 3 bytes max
        cache.insert("a", vec![1]);       // 1 byte
        cache.insert("b", vec![2, 2]);    // 2 bytes → total 3
        cache.insert("c", vec![3]);       // 1 byte → must evict a (1 byte)
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
        assert!(cache.current_bytes() <= 3);
    }

    #[test]
    fn replace_existing_entry_updates_bytes() {
        let mut cache = SeCache::new(10);
        cache.insert("x", vec![1, 2, 3]); // 3 bytes
        cache.insert("x", vec![4, 5]);     // 2 bytes, replaces
        assert_eq!(cache.current_bytes(), 2);
        assert_eq!(cache.get("x"), Some(&[4, 5][..]));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn clear_removes_all() {
        let mut cache = SeCache::new(1024);
        cache.insert("a", vec![1]);
        cache.insert("b", vec![2]);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.current_bytes(), 0);
    }

    #[test]
    fn default_cache_is_10mb() {
        let cache = SeCache::default();
        assert_eq!(cache.max_bytes, DEFAULT_SE_CACHE_BYTES);
    }
}
