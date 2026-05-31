// Case-insensitive path cache.
//
// On case-sensitive file systems (Linux, macOS with APFS case-sensitive
// volumes), RPG Maker games that mix `Graphics/Titles/Title.png` and
// `graphics/titles/title.png` in their resource references will fail to
// find files.  The path cache solves this by building a full index of
// every known virtual path (lowercased → real mixed-case) and resolving
// queries against it before hitting the mount sources.

use crate::{FsError, VPath, mountable::Mountable};
use std::collections::HashMap;

/// Maps lowercased virtual paths to their true (mixed-case) form.
///
/// Built once at startup by walking every mounted source.  Later mounts
/// override earlier ones for the same lowercased key, matching the
/// reverse-order search semantics of `FileSystem`.
///
/// # Examples
///
/// ```
/// use mkxp_fs::path_cache::PathCache;
/// use std::collections::HashMap;
///
/// // lower → real
/// let mut map = HashMap::new();
/// map.insert("graphics/titles/title.png".into(), "Graphics/Titles/Title.png".into());
/// let cache = PathCache::from_map(map);
/// assert_eq!(
///     cache.resolve("graphics/titles/title.png"),
///     Some("Graphics/Titles/Title.png".as_ref()),
/// );
/// ```
#[derive(Debug, Default)]
pub struct PathCache {
    lower_to_real: HashMap<String, String>,
}

impl PathCache {
    /// Build a path cache by walking every mounted source.
    ///
    /// Sources are visited in reverse order so that later mounts
    /// (higher priority) shadow earlier ones for the same key.
    ///
    /// # Errors
    ///
    /// Returns `FsError` if enumeration of any mount source fails.
    pub fn build(
        mounts: &[(VPath, Box<dyn Mountable>)],
    ) -> Result<Self, FsError> {
        let mut lower_to_real = HashMap::new();

        // Walk in reverse — later mounts win over earlier ones.
        for (mountpoint, source) in mounts.iter().rev() {
            collect_recursive(source.as_ref(), mountpoint, &mut lower_to_real)?;
        }

        Ok(Self { lower_to_real })
    }

    /// Create a path cache from a pre-built map.  Intended for tests.
    #[doc(hidden)]
    pub fn from_map(map: HashMap<String, String>) -> Self {
        Self {
            lower_to_real: map,
        }
    }

    /// Resolve a lowercased path to its real mixed-case form.
    ///
    /// Returns `None` if the path is not in the cache.
    pub fn resolve(&self, lower: &str) -> Option<&str> {
        self.lower_to_real.get(lower).map(|s| s.as_str())
    }

    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        self.lower_to_real.len()
    }

    /// `true` when the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.lower_to_real.is_empty()
    }
}

/// Recursively enumerate a mount source and insert `(lowercase → real)`
/// entries for every file path.
fn collect_recursive(
    source: &dyn Mountable,
    dir: &VPath,
    map: &mut HashMap<String, String>,
) -> Result<(), FsError> {
    let entries = match source.enumerate(dir) {
        Ok(e) => e,
        Err(e) => {
            // If the directory doesn't exist, skip it silently (some
            // mount points may not have every subdirectory).
            if matches!(e, FsError::NotADirectory { .. }) {
                return Ok(());
            }
            return Err(e);
        }
    };

    for entry in entries {
        let is_dir = entry.ends_with('/');
        let name = if is_dir {
            &entry[..entry.len() - 1]
        } else {
            &entry
        };

        let child_vpath = if dir.is_root() {
            VPath::new(name)?
        } else {
            dir.join(name)?
        };

        let child_str = child_vpath.as_str().to_string();
        let lower = child_str.to_lowercase();

        // Later entries in the reverse walk win, so subsequent entries
        // for the same lowercase key are silently dropped.
        map.entry(lower).or_insert(child_str);

        if is_dir {
            collect_recursive(source, &child_vpath, map)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cache() {
        let cache = PathCache::default();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.resolve("anything"), None);
    }

    #[test]
    fn resolve_hit() {
        let mut map = HashMap::new();
        map.insert("a.txt".into(), "A.txt".into());
        let cache = PathCache::from_map(map);
        assert_eq!(cache.resolve("a.txt"), Some("A.txt"));
    }

    #[test]
    fn resolve_miss() {
        let cache = PathCache::from_map(HashMap::new());
        assert_eq!(cache.resolve("nope.txt"), None);
    }

    #[test]
    fn resolve_case_difference() {
        let mut map = HashMap::new();
        map.insert("data/scripts.rxdata".into(), "Data/Scripts.rxdata".into());
        let cache = PathCache::from_map(map);
        assert_eq!(
            cache.resolve("data/scripts.rxdata"),
            Some("Data/Scripts.rxdata"),
        );
    }
}
