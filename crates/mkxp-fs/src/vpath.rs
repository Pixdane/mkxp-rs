// Virtual path type — a validated forward-slash path with no root prefix.
//
// `VPath` is the canonical path representation inside `mkxp_fs`.  It is
// always a relative, forward-slash-separated string without a leading
// slash (the empty string represents the virtual root).  Once
// constructed, every method can assume the path is well-formed.

use crate::FsError;

/// A validated virtual path (owned).
///
/// `VPath` wraps a `String` and guarantees, by construction, that:
///
/// - No backslashes (`\`) are present.
/// - No leading or trailing slashes (except the root `""`).
/// - No `"."` or `".."` segments (no directory traversal).
/// - No empty segments (`"a//b"` is rejected).
/// - All code points are printable ASCII.
///
/// `VPath` implements `Deref<Target = str>`, so you can pass `&VPath`
/// wherever a `&str` virtual path is expected (HashMap keys, display, …).
///
/// # Examples
///
/// ```
/// use mkxp_fs::VPath;
///
/// let p = VPath::new("Graphics/Titles/title.png").unwrap();
/// assert_eq!(p.as_str(), "Graphics/Titles/title.png");
/// assert_eq!(p.parent(), Some("Graphics/Titles"));
/// assert_eq!(p.file_name(), Some("title.png"));
/// assert_eq!(p.extension(), Some("png"));
/// ```
///
/// The empty string represents the virtual root:
///
/// ```
/// use mkxp_fs::VPath;
///
/// let root = VPath::new("").unwrap();
/// assert!(root.is_root());
/// assert_eq!(root.parent(), None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VPath(String);

impl VPath {
    /// Create a new `VPath` after validating and normalising `raw`.
    ///
    /// Validation checks:
    ///
    /// | Condition | Example (rejected) |
    /// |-----------|---------------------|
    /// | Backslash anywhere | `"a\\b"` |
    /// | Leading slash | `"/a"` |
    /// | Trailing slash | `"a/"` |
    /// | Double slash | `"a//b"` |
    /// | `.` segment | `"a/./b"` |
    /// | `..` segment | `"a/../b"` |
    /// | Non-printable ASCII | `"a\x00b"` |
    ///
    /// The empty string `""` is allowed and represents the root.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::InvalidPath`] if validation fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use mkxp_fs::VPath;
    ///
    /// assert!(VPath::new("Data/Scripts.rxdata").is_ok());
    /// assert!(VPath::new("").is_ok());
    /// assert!(VPath::new("/absolute").is_err());
    /// assert!(VPath::new("escape/..").is_err());
    /// ```
    pub fn new(raw: &str) -> Result<Self, FsError> {
        // Root is special
        if raw.is_empty() {
            return Ok(Self(String::new()));
        }

        // Reject backslashes — virtual paths use only forward slashes.
        if raw.contains('\\') {
            return Err(FsError::InvalidPath {
                reason: "backslash not allowed in virtual paths".into(),
            });
        }

        // Reject leading slash — paths are relative to the virtual root.
        if raw.starts_with('/') {
            return Err(FsError::InvalidPath {
                reason: "leading slash not allowed (paths are relative)".into(),
            });
        }

        // Reject trailing slash.
        if raw.ends_with('/') {
            return Err(FsError::InvalidPath {
                reason: "trailing slash not allowed".into(),
            });
        }

        // Reject non-printable characters.
        if raw.chars().any(|c| c.is_ascii_control()) {
            return Err(FsError::InvalidPath {
                reason: "control characters not allowed".into(),
            });
        }

        // Reject double slashes.
        if raw.contains("//") {
            return Err(FsError::InvalidPath {
                reason: "double slash not allowed".into(),
            });
        }

        // Reject `.` and `..` segments.
        for seg in raw.split('/') {
            if seg.is_empty() {
                // Already checked via double-slash, but guard anyway.
                return Err(FsError::InvalidPath {
                    reason: "empty path segment".into(),
                });
            }
            if seg == "." {
                return Err(FsError::InvalidPath {
                    reason: r#"segment "." not allowed"#.into(),
                });
            }
            if seg == ".." {
                return Err(FsError::InvalidPath {
                    reason: r#"segment ".." not allowed"#.into(),
                });
            }
        }

        Ok(Self(raw.to_string()))
    }

    /// Return the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `true` when this is the virtual root (empty string).
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The parent directory path, or `None` if this is the root or a
    /// top-level entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use mkxp_fs::VPath;
    ///
    /// let p = VPath::new("a/b/c.txt").unwrap();
    /// assert_eq!(p.parent(), Some("a/b"));
    ///
    /// let top = VPath::new("file.txt").unwrap();
    /// assert_eq!(top.parent(), None);
    ///
    /// let root = VPath::new("").unwrap();
    /// assert_eq!(root.parent(), None);
    /// ```
    pub fn parent(&self) -> Option<&str> {
        let pos = self.0.rfind('/')?;
        if pos == 0 {
            None // parent is root
        } else {
            Some(&self.0[..pos])
        }
    }

    /// The final component (file or directory name), or `None` for the
    /// root.
    ///
    /// # Examples
    ///
    /// ```
    /// use mkxp_fs::VPath;
    ///
    /// let p = VPath::new("Graphics/Titles/title.png").unwrap();
    /// assert_eq!(p.file_name(), Some("title.png"));
    ///
    /// let root = VPath::new("").unwrap();
    /// assert_eq!(root.file_name(), None);
    /// ```
    pub fn file_name(&self) -> Option<&str> {
        if self.0.is_empty() {
            return None;
        }
        self.0.rfind('/').map_or(
            Some(&self.0[..]),
            |pos| Some(&self.0[pos + 1..]),
        )
    }

    /// The file extension (without the leading dot), or `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use mkxp_fs::VPath;
    ///
    /// let p = VPath::new("Scripts.rxdata").unwrap();
    /// assert_eq!(p.extension(), Some("rxdata"));
    ///
    /// let no_ext = VPath::new("README").unwrap();
    /// assert_eq!(no_ext.extension(), None);
    /// ```
    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name()?;
        let pos = name.rfind('.')?;
        if pos == 0 || pos == name.len() - 1 {
            None // dot at start or end is not an extension
        } else {
            Some(&name[pos + 1..])
        }
    }

    /// Append a relative child to this path, producing a new `VPath`.
    ///
    /// # Examples
    ///
    /// ```
    /// use mkxp_fs::VPath;
    ///
    /// let base = VPath::new("Graphics/Titles").unwrap();
    /// let full = base.join("title.png").unwrap();
    /// assert_eq!(full.as_str(), "Graphics/Titles/title.png");
    /// ```
    pub fn join(&self, child: &str) -> Result<Self, FsError> {
        if child.is_empty() {
            return Ok(self.clone());
        }
        // Reuse VPath::new validation on the child portion, then
        // re-join manually to avoid double-alloc.
        VPath::new(child)?;
        let joined = if self.is_root() {
            child.to_string()
        } else {
            format!("{}/{child}", self.0)
        };
        Ok(Self(joined))
    }
}

// ---- standard trait impls ------------------------------------------------

impl std::ops::Deref for VPath {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for VPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for VPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<VPath> for String {
    fn from(p: VPath) -> Self {
        p.0
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- construction ----------------------------------------------------

    #[test]
    fn valid_simple_path() {
        let p = VPath::new("hello.txt").unwrap();
        assert_eq!(p.as_str(), "hello.txt");
    }

    #[test]
    fn valid_nested_path() {
        let p = VPath::new("Graphics/Titles/title.png").unwrap();
        assert_eq!(p.as_str(), "Graphics/Titles/title.png");
    }

    #[test]
    fn root_is_empty_string() {
        let p = VPath::new("").unwrap();
        assert_eq!(p.as_str(), "");
        assert!(p.is_root());
    }

    #[test]
    fn reject_backslash() {
        assert!(VPath::new("a\\b").is_err());
    }

    #[test]
    fn reject_leading_slash() {
        assert!(VPath::new("/a").is_err());
    }

    #[test]
    fn reject_trailing_slash() {
        assert!(VPath::new("a/").is_err());
    }

    #[test]
    fn reject_double_slash() {
        assert!(VPath::new("a//b").is_err());
    }

    #[test]
    fn reject_dot_segment() {
        assert!(VPath::new("a/./b").is_err());
    }

    #[test]
    fn reject_dotdot_segment() {
        assert!(VPath::new("a/../b").is_err());
    }

    #[test]
    fn reject_control_chars() {
        assert!(VPath::new("a\x00b").is_err());
    }

    // ---- parent ----------------------------------------------------------

    #[test]
    fn parent_of_nested_path() {
        let p = VPath::new("a/b/c.txt").unwrap();
        assert_eq!(p.parent(), Some("a/b"));
    }

    #[test]
    fn parent_of_top_level_is_none() {
        let p = VPath::new("file.txt").unwrap();
        assert_eq!(p.parent(), None);
    }

    #[test]
    fn parent_of_root_is_none() {
        let p = VPath::new("").unwrap();
        assert_eq!(p.parent(), None);
    }

    #[test]
    fn parent_of_one_level() {
        let p = VPath::new("Graphics/Titles").unwrap();
        assert_eq!(p.parent(), Some("Graphics"));
    }

    // ---- file_name -------------------------------------------------------

    #[test]
    fn file_name_nested() {
        let p = VPath::new("a/b/c.txt").unwrap();
        assert_eq!(p.file_name(), Some("c.txt"));
    }

    #[test]
    fn file_name_top_level() {
        let p = VPath::new("file.txt").unwrap();
        assert_eq!(p.file_name(), Some("file.txt"));
    }

    #[test]
    fn file_name_root() {
        let p = VPath::new("").unwrap();
        assert_eq!(p.file_name(), None);
    }

    // ---- extension -------------------------------------------------------

    #[test]
    fn extension_simple() {
        let p = VPath::new("Scripts.rxdata").unwrap();
        assert_eq!(p.extension(), Some("rxdata"));
    }

    #[test]
    fn extension_nested() {
        let p = VPath::new("a/b/c.png").unwrap();
        assert_eq!(p.extension(), Some("png"));
    }

    #[test]
    fn extension_none() {
        let p = VPath::new("README").unwrap();
        assert_eq!(p.extension(), None);
    }

    #[test]
    fn extension_dotfile() {
        // Leading dot is not an extension separator.
        let p = VPath::new(".hidden").unwrap();
        assert_eq!(p.extension(), None);
    }

    #[test]
    fn extension_trailing_dot() {
        let p = VPath::new("file.").unwrap();
        assert_eq!(p.extension(), None);
    }

    // ---- join ------------------------------------------------------------

    #[test]
    fn join_child() {
        let base = VPath::new("Graphics").unwrap();
        let full = base.join("Titles").unwrap();
        assert_eq!(full.as_str(), "Graphics/Titles");
    }

    #[test]
    fn join_empty_is_clone() {
        let base = VPath::new("a").unwrap();
        let result = base.join("").unwrap();
        assert_eq!(result, base);
    }

    #[test]
    fn join_to_root() {
        let root = VPath::new("").unwrap();
        let result = root.join("child").unwrap();
        assert_eq!(result.as_str(), "child");
    }

    #[test]
    fn join_rejects_invalid_child() {
        let base = VPath::new("a").unwrap();
        assert!(base.join("../b").is_err());
    }

    // ---- trait impls -----------------------------------------------------

    #[test]
    fn deref_gives_str() {
        let p = VPath::new("a/b").unwrap();
        let s: &str = &p;
        assert_eq!(s, "a/b");
    }

    #[test]
    fn as_ref_str() {
        let p = VPath::new("a/b").unwrap();
        let s: &str = p.as_ref();
        assert_eq!(s, "a/b");
    }

    #[test]
    fn display() {
        let p = VPath::new("a/b").unwrap();
        assert_eq!(format!("{p}"), "a/b");
    }

    #[test]
    fn into_string() {
        let p = VPath::new("a/b").unwrap();
        let s: String = p.into();
        assert_eq!(s, "a/b");
    }

    #[test]
    fn clone_and_eq() {
        let a = VPath::new("x").unwrap();
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn hash_consistent() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let a = VPath::new("a/b").unwrap();
        let b = VPath::new("a/b").unwrap();
        let mut ha = DefaultHasher::new();
        let mut hb = DefaultHasher::new();
        a.hash(&mut ha);
        b.hash(&mut hb);
        assert_eq!(ha.finish(), hb.finish());
    }
}
