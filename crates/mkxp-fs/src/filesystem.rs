// Layered virtual file system.
//
// Multiple data sources (directories, archives) can be mounted at
// different mount points.  When a file is requested, sources are
// searched in reverse mount order — the last-mounted source has the
// highest priority.

use crate::mountable::Mountable;
use crate::path_cache::PathCache;
use crate::{FsError, VPath};

/// A layered virtual file system.
///
/// Multiple data sources (directories, archives) can be mounted at
/// different mount points.  When a file is requested, sources are
/// searched in reverse mount order — the last-mounted source has the
/// highest priority.
///
/// # Examples
///
/// ```rust,no_run
/// use mkxp_fs::{FileSystem, VPath};
/// use mkxp_fs::mountable::RealDirectory;
/// use std::path::Path;
///
/// let mut fs = FileSystem::new();
/// let dir = RealDirectory::new(Path::new("game")).unwrap();
/// fs.mount(Box::new(dir), &VPath::new("").unwrap());
/// fs.build_path_cache();
///
/// if fs.exists("Data/Scripts.rxdata") {
///     let data = fs.read("Data/Scripts.rxdata").unwrap();
/// }
/// ```
pub struct FileSystem {
    mounts: Vec<(VPath, Box<dyn Mountable>)>,
    path_cache: Option<PathCache>,
}

impl FileSystem {
    /// Create an empty file system with no mounted sources.
    pub fn new() -> Self {
        Self {
            mounts: Vec::new(),
            path_cache: None,
        }
    }

    /// Mount a data source at `mountpoint`.
    ///
    /// The mount point is typically the root (`""`).  Files inside
    /// `source` that are logically at `Graphics/Titles/title.png` will
    /// be accessible at the same virtual path when mounted at root.
    ///
    /// Sources mounted later shadow earlier ones for overlapping paths.
    pub fn mount(&mut self, source: Box<dyn Mountable>, mountpoint: &VPath) {
        tracing::info!(mountpoint = %mountpoint.as_str(), "mounted source");
        self.mounts.push((mountpoint.clone(), source));
    }

    /// Read the full contents of a file.
    ///
    /// `path` is a virtual path string (forward slashes, relative to
    /// the virtual root).  If a path cache has been built, the lookup is
    /// case-insensitive.
    ///
    /// Sources are searched in reverse mount order.  The first source
    /// that contains the file supplies the content.
    ///
    /// # Errors
    ///
    /// Returns `FsError::NotFound` if no source contains the file.
    pub fn read(&self, path: &str) -> Result<Vec<u8>, FsError> {
        let resolved = self.resolve_path(path)?;
        self.try_read(&resolved)
    }

    /// Check whether a file exists.
    ///
    /// Same lookup logic as [`Self::read`], but skips the actual I/O.
    pub fn exists(&self, path: &str) -> bool {
        let resolved = match self.resolve_path(path) {
            Ok(p) => p,
            Err(_) => return false,
        };
        self.try_exists(&resolved)
    }

    /// List all direct children of `dir`.
    ///
    /// Entries from all mounted sources are merged and deduplicated.
    /// Directory entries are suffixed with `"/"`.
    ///
    /// # Errors
    ///
    /// Returns `FsError` if `dir` is not a valid virtual path or if
    /// enumeration of a mount source fails unexpectedly.
    pub fn read_dir(&self, dir: &str) -> Result<Vec<String>, FsError> {
        let vdir = VPath::new(dir).map_err(|e| FsError::InvalidPath {
            reason: e.to_string(),
        })?;

        let mut entries = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Search in reverse order (last mount wins for shadowing).
        // For directory listing, we want the union, but entries from
        // higher-priority mounts should appear and shadow lower ones.
        for (_mountpoint, source) in self.mounts.iter().rev() {
            if let Ok(mut src_entries) = source.enumerate(&vdir) {
                for name in src_entries.drain(..) {
                    if seen.insert(name.clone()) {
                        entries.push(name);
                    }
                }
            }
        }

        // Reverse back so higher-priority entries come first.
        entries.reverse();
        Ok(entries)
    }

    /// Build the case-insensitive path cache.
    ///
    /// Walks every mounted source to collect all known virtual paths,
    /// then builds a `lowercase → real-case` mapping.  After this call,
    /// [`Self::read`] and [`Self::exists`] become case-insensitive.
    ///
    /// This should be called once after all sources have been mounted.
    pub fn build_path_cache(&mut self) -> Result<(), FsError> {
        let cache = PathCache::build(&self.mounts)?;
        let entry_count = cache.len();
        self.path_cache = Some(cache);
        tracing::info!(entries = entry_count, "built path cache");
        Ok(())
    }

    /// Return the number of mounted sources.
    pub fn mount_count(&self) -> usize {
        self.mounts.len()
    }
}

// ---- internal helpers ----------------------------------------------------

impl FileSystem {
    /// Resolve a raw path string to a VPath, applying the path cache if
    /// available.
    fn resolve_path(&self, path: &str) -> Result<VPath, FsError> {
        if let Some(ref cache) = self.path_cache {
            let lower = path.to_lowercase();
            if let Some(real) = cache.resolve(&lower) {
                if real != path {
                    tracing::warn!(requested = %path, actual = %real, "case mismatch in path");
                }
                VPath::new(real)
            } else {
                VPath::new(path)
            }
        } else {
            VPath::new(path)
        }
    }

    /// Try to read `vp` from any mounted source (reverse order).
    fn try_read(&self, vp: &VPath) -> Result<Vec<u8>, FsError> {
        for (_, source) in self.mounts.iter().rev() {
            if source.exists(vp) {
                return source.read(vp);
            }
        }
        Err(FsError::NotFound {
            path: vp.as_str().to_string(),
        })
    }

    /// Check whether `vp` exists in any mounted source (reverse order).
    fn try_exists(&self, vp: &VPath) -> bool {
        self.mounts
            .iter()
            .rev()
            .any(|(_, source)| source.exists(vp))
    }
}

impl Default for FileSystem {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mountable::RealDirectory;

    /// Helper: create a temporary directory with the given file contents.
    fn temp_real_dir(files: &[(&str, &[u8])]) -> (TempDir, RealDirectory) {
        let d = TempDir::new();
        for (name, data) in files {
            let path = d.path().join(name);
            if name.ends_with('/') {
                std::fs::create_dir_all(&path).unwrap();
            } else {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&path, data).unwrap();
            }
        }
        let rd = RealDirectory::new(d.path()).unwrap();
        (d, rd)
    }

    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            use std::time::{SystemTime, UNIX_EPOCH};
            let t = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join("mkxp_fs_tests")
                .join(format!("test_{t:016x}"));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    // ---- basic operations ------------------------------------------------

    #[test]
    fn read_file_from_single_source() {
        let (_td, rd) = temp_real_dir(&[("hello.txt", b"world")]);
        let mut fs = FileSystem::new();
        fs.mount(Box::new(rd), &VPath::new("").unwrap());
        assert_eq!(fs.read("hello.txt").unwrap(), b"world");
    }

    #[test]
    fn read_nonexistent_returns_not_found() {
        let (_td, rd) = temp_real_dir(&[]);
        let mut fs = FileSystem::new();
        fs.mount(Box::new(rd), &VPath::new("").unwrap());
        assert!(matches!(
            fs.read("nope.txt").unwrap_err(),
            FsError::NotFound { .. }
        ));
    }

    #[test]
    fn exists_true_and_false() {
        let (_td, rd) = temp_real_dir(&[("a.txt", b"")]);
        let mut fs = FileSystem::new();
        fs.mount(Box::new(rd), &VPath::new("").unwrap());
        assert!(fs.exists("a.txt"));
        assert!(!fs.exists("b.txt"));
    }

    // ---- mount priority --------------------------------------------------

    #[test]
    fn later_mount_shadows_earlier() {
        let (_td1, rd1) = temp_real_dir(&[("shared.txt", b"lower")]);
        let (_td2, rd2) = temp_real_dir(&[("shared.txt", b"higher")]);

        let mut fs = FileSystem::new();
        fs.mount(Box::new(rd1), &VPath::new("").unwrap());
        fs.mount(Box::new(rd2), &VPath::new("").unwrap());

        assert_eq!(fs.read("shared.txt").unwrap(), b"higher");
    }

    #[test]
    fn later_mount_only_shadows_overlap() {
        let (_td1, rd1) = temp_real_dir(&[("a.txt", b"aaa")]);
        let (_td2, rd2) = temp_real_dir(&[("b.txt", b"bbb")]);

        let mut fs = FileSystem::new();
        fs.mount(Box::new(rd1), &VPath::new("").unwrap());
        fs.mount(Box::new(rd2), &VPath::new("").unwrap());

        assert_eq!(fs.read("a.txt").unwrap(), b"aaa");
        assert_eq!(fs.read("b.txt").unwrap(), b"bbb");
    }

    // ---- read_dir --------------------------------------------------------

    #[test]
    fn read_dir_merges_sources() {
        let (_td1, rd1) = temp_real_dir(&[("a.txt", b""), ("b.txt", b"")]);
        let (_td2, rd2) = temp_real_dir(&[("c.txt", b""), ("sub/", &[])]);

        let mut fs = FileSystem::new();
        fs.mount(Box::new(rd1), &VPath::new("").unwrap());
        fs.mount(Box::new(rd2), &VPath::new("").unwrap());

        let mut entries = fs.read_dir("").unwrap();
        entries.sort();
        assert_eq!(entries, vec!["a.txt", "b.txt", "c.txt", "sub/"]);
    }

    #[test]
    fn read_dir_dedup_same_name() {
        let (_td1, rd1) = temp_real_dir(&[("common.txt", b"v1")]);
        let (_td2, rd2) = temp_real_dir(&[("common.txt", b"v2")]);

        let mut fs = FileSystem::new();
        fs.mount(Box::new(rd1), &VPath::new("").unwrap());
        fs.mount(Box::new(rd2), &VPath::new("").unwrap());

        let entries = fs.read_dir("").unwrap();
        assert_eq!(entries.iter().filter(|e| *e == "common.txt").count(), 1);
    }

    #[test]
    fn read_dir_invalid_path_returns_error() {
        let fs = FileSystem::new();
        assert!(fs.read_dir("/bad").is_err());
    }

    // ---- path cache ------------------------------------------------------

    #[test]
    fn path_cache_resolves_case_difference() {
        let (_td, rd) = temp_real_dir(&[("Graphics/Titles/Title.png", b"png")]);
        let mut fs = FileSystem::new();
        fs.mount(Box::new(rd), &VPath::new("").unwrap());
        fs.build_path_cache().unwrap();

        assert!(fs.exists("graphics/titles/title.png"));
        let data = fs.read("GRAPHICS/TITLES/TITLE.PNG").unwrap();
        assert_eq!(data, b"png");
    }

    #[test]
    fn path_cache_resolves_directory_listing() {
        let (_td, rd) = temp_real_dir(&[
            ("Data/Scripts.rxdata", b"scripts"),
            ("Data/Map001.rxdata", b"map"),
        ]);
        let mut fs = FileSystem::new();
        fs.mount(Box::new(rd), &VPath::new("").unwrap());
        fs.build_path_cache().unwrap();

        let entries = fs.read_dir("data").unwrap();
        assert!(entries.contains(&"Scripts.rxdata".to_string()));
        assert!(entries.contains(&"Map001.rxdata".to_string()));
    }

    #[test]
    fn path_cache_maps_multiple_variants() {
        let (_td, rd) = temp_real_dir(&[("A.txt", b"hello")]);
        let mut fs = FileSystem::new();
        fs.mount(Box::new(rd), &VPath::new("").unwrap());
        fs.build_path_cache().unwrap();

        assert_eq!(fs.read("A.txt").unwrap(), b"hello");
        assert_eq!(fs.read("a.txt").unwrap(), b"hello");
        assert_eq!(fs.read("A.TXT").unwrap(), b"hello");
    }

    // ---- RGSS archive integration ---------------------------------------

    #[test]
    fn mount_rgss_archive() {
        use crate::rgss::RgssArchive;

        // Build a synthetic RGSS archive and mount it
        let raw = crate::rgss::build_rgss1(&[
            ("Graphics/Titles/title.png", b"pngdata"),
            ("Data/Scripts.rxdata", b"rubyscripts"),
        ]);
        let archive = RgssArchive::parse(raw).unwrap();

        let mut fs = FileSystem::new();
        fs.mount(Box::new(archive), &VPath::new("").unwrap());

        // Read through FileSystem
        assert!(fs.exists("Graphics/Titles/title.png"));
        assert_eq!(fs.read("Graphics/Titles/title.png").unwrap(), b"pngdata");
        assert_eq!(fs.read("Data/Scripts.rxdata").unwrap(), b"rubyscripts");
        assert!(matches!(
            fs.read("nope.txt").unwrap_err(),
            FsError::NotFound { .. }
        ));
    }

    #[test]
    fn mount_rgss_with_real_directory() {
        use crate::rgss::RgssArchive;

        // Real directory has a.txt
        let (_td, rd) = temp_real_dir(&[("a.txt", b"real")]);

        // RGSS archive has b.txt
        let raw = crate::rgss::build_rgss1(&[("b.txt", b"archive")]);
        let archive = RgssArchive::parse(raw).unwrap();

        let mut fs = FileSystem::new();
        fs.mount(Box::new(rd), &VPath::new("").unwrap());
        fs.mount(Box::new(archive), &VPath::new("").unwrap());

        // Both sources are visible
        assert_eq!(fs.read("a.txt").unwrap(), b"real");
        assert_eq!(fs.read("b.txt").unwrap(), b"archive");

        // If both have the same file, the RGSS archive (higher priority) wins
        let raw2 = crate::rgss::build_rgss1(&[("a.txt", b"overridden")]);
        let archive2 = RgssArchive::parse(raw2).unwrap();
        fs.mount(Box::new(archive2), &VPath::new("").unwrap());

        assert_eq!(fs.read("a.txt").unwrap(), b"overridden");
    }
}
