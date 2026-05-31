// Mountable trait and the built-in real-directory data source.

use crate::{FsError, VPath};
use mkxp_types::MkxpError;

use std::path::Path;

/// A data source that can be mounted into the virtual file system.
pub trait Mountable: Send + Sync {
    /// Read the full contents of a file at `path`.
    fn read(&self, path: &VPath) -> Result<Vec<u8>, FsError>;

    /// Check whether a file exists at `path`.
    fn exists(&self, path: &VPath) -> bool;

    /// List all direct children (files and directories) inside `dir`.
    ///
    /// Directory entries are suffixed with a trailing slash; file entries
    /// are bare names.  Pass the root `""` to list the mount-point root.
    fn enumerate(&self, dir: &VPath) -> Result<Vec<String>, FsError>;
}

// ---------------------------------------------------------------------------
// RealDirectory
// ---------------------------------------------------------------------------

/// A real directory on the local file system.
pub struct RealDirectory {
    root: String,
}

impl RealDirectory {
    /// Create from a filesystem path.  Returns an error if `root` is not
    /// a readable directory.
    pub fn new(root: &Path) -> Result<Self, FsError> {
        let canonical = root.canonicalize().map_err(|e| FsError::io(e))?;
        if !canonical.is_dir() {
            return Err(FsError::NotADirectory {
                path: root.display().to_string(),
            });
        }
        Ok(Self {
            root: canonical.to_string_lossy().into_owned(),
        })
    }

    /// Join root with a virtual path (pure string operation).
    fn physical(&self, vp: &VPath) -> std::path::PathBuf {
        Path::new(&self.root).join(vp.as_str())
    }

    /// Canonicalize a physical path and verify it stays under the root.
    /// Used for defence-in-depth (symlink escape prevention).
    fn checked(&self, vp: &VPath) -> Result<std::path::PathBuf, FsError> {
        let p = self.physical(vp);
        let resolved = p.canonicalize().map_err(|e| FsError::io(e))?;
        if !resolved.starts_with(&self.root) {
            return Err(FsError::PathEscape {
                path: vp.as_str().to_string(),
            });
        }
        Ok(resolved)
    }
}

impl Mountable for RealDirectory {
    fn read(&self, path: &VPath) -> Result<Vec<u8>, FsError> {
        // Defence-in-depth: canonicalize to detect symlink escapes.
        let real = match self.checked(path) {
            Ok(r) => r,
            Err(e) => {
                // If canonicalize failed because the file doesn't exist,
                // convert to NotFound (checked is for security, not for
                // existence — the std::fs::read below is the real
                // existence check).
                if matches!(&e, FsError::Mkxp(MkxpError::Io(io))
                    if io.kind() == std::io::ErrorKind::NotFound)
                {
                    return Err(FsError::NotFound {
                        path: path.as_str().to_string(),
                    });
                }
                return Err(e);
            }
        };

        std::fs::read(&real).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FsError::NotFound {
                    path: path.as_str().to_string(),
                }
            } else {
                FsError::io(e)
            }
        })
    }

    fn exists(&self, path: &VPath) -> bool {
        let real = self.physical(path);
        if !real.is_file() {
            return false;
        }
        // Defence-in-depth check (only canonicalizes if the file exists).
        self.checked(path).is_ok()
    }

    fn enumerate(&self, dir: &VPath) -> Result<Vec<String>, FsError> {
        let real = self.checked(dir)?;
        if !real.is_dir() {
            return Err(FsError::NotADirectory {
                path: dir.as_str().to_string(),
            });
        }
        let rd = std::fs::read_dir(&real).map_err(|e| FsError::io(e))?;
        let mut entries = Vec::new();
        for entry in rd {
            let entry = entry.map_err(|e| FsError::io(e))?;
            let ft = entry.file_type().map_err(|e| FsError::io(e))?;
            if let Some(name) = entry.file_name().to_str() {
                let suffix = if ft.is_dir() { "/" } else { "" };
                entries.push(format!("{name}{suffix}"));
            }
        }
        Ok(entries)
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
struct TempDir {
    path: std::path::PathBuf,
}

#[cfg(test)]
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

    fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(s: &str) -> VPath {
        VPath::new(s).unwrap()
    }

    #[test]
    fn read_file() {
        let d = TempDir::new();
        std::fs::write(d.path().join("test.txt"), b"hello").unwrap();
        let rd = RealDirectory::new(d.path()).unwrap();
        assert_eq!(rd.read(&vp("test.txt")).unwrap(), b"hello");
    }

    #[test]
    fn read_nonexistent_is_not_found() {
        let d = TempDir::new();
        let rd = RealDirectory::new(d.path()).unwrap();
        assert!(matches!(
            rd.read(&vp("nope.txt")).unwrap_err(),
            FsError::NotFound { .. }
        ));
    }

    #[test]
    fn exists_true_and_false() {
        let d = TempDir::new();
        std::fs::write(d.path().join("a.txt"), b"").unwrap();
        let rd = RealDirectory::new(d.path()).unwrap();
        assert!(rd.exists(&vp("a.txt")));
        assert!(!rd.exists(&vp("nope.txt")));
    }

    #[test]
    fn enumerate_root() {
        let d = TempDir::new();
        std::fs::write(d.path().join("a.txt"), b"").unwrap();
        std::fs::write(d.path().join("b.txt"), b"").unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        let rd = RealDirectory::new(d.path()).unwrap();
        let mut e = rd.enumerate(&vp("")).unwrap();
        e.sort();
        assert_eq!(e, vec!["a.txt", "b.txt", "sub/"]);
    }

    #[test]
    fn enumerate_nonexistent_is_error() {
        let d = TempDir::new();
        let rd = RealDirectory::new(d.path()).unwrap();
        assert!(rd.enumerate(&vp("nope")).is_err());
    }

    #[test]
    fn reject_nonexistent_root() {
        assert!(RealDirectory::new(Path::new("/nonexistent/xyz123")).is_err());
    }

    #[test]
    fn reject_file_as_root() {
        let d = TempDir::new();
        let f = d.path().join("not_dir.txt");
        std::fs::write(&f, b"").unwrap();
        assert!(RealDirectory::new(&f).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn symlink_escape_is_blocked() {
        let inside = TempDir::new();
        let outside = TempDir::new();
        std::fs::write(outside.path().join("secret.txt"), b"leaked").unwrap();
        std::os::unix::fs::symlink(outside.path(), inside.path().join("escape")).unwrap();

        let rd = RealDirectory::new(inside.path()).unwrap();
        assert!(matches!(
            rd.read(&vp("escape")).unwrap_err(),
            FsError::PathEscape { .. }
        ));
    }
}
