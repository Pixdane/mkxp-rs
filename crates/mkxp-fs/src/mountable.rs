//! Mountable trait and built-in data sources (real directories, archives).
//!
//! Stub — implementation pending.
pub trait Mountable: Send + Sync {
    fn read(&self, path: &str) -> Result<Vec<u8>, crate::FsError>;
    fn exists(&self, path: &str) -> bool;
    fn enumerate(&self, dir: &str) -> Result<Vec<String>, crate::FsError>;
}
