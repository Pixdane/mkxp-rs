// RGSS encrypted archive support.
//
// Implements the three RGSS archive formats used by RPG Maker XP, VX,
// and VX Ace.  All formats use an XOR cipher based on a linear
// congruential generator (LCG): `next = prev * 7 + 3`, seeded from
// the magic constant `0xDEAD_CAFE`.
//
// ## Format comparison
//
// | Format   | Extension | RPG Maker | Magic header    | Layout              |
// |----------|-----------|-----------|-----------------|---------------------|
// | RGSS1    | .rgssad   | XP        | `RGSSAD\x00\x01`| Interleaved entries |
// | RGSS2    | .rgss2a   | VX        | `RGSSAD\x00\x02`| Interleaved entries |
// | RGSS3    | .rgss3a   | VX Ace    | `RGSSAD\x00\x03`| Separate index+data |
//
// RGSS1 and RGSS2 share the same internal structure (interleaved
// entry headers + file data, with a continuous LCG stream for the
// index).  RGSS3 uses a separate index section with an additional
// base-key XOR layer.
//
// ## Archive layout (RGSS1 / RGSS2 — interleaved)
//
// ```text
// [Header: 8 bytes (magic + version)]
// [Entry 1: nameLen(4) | name(n) | size(4)]  ← all XOR'd with LCG stream
// [Entry 1 data: size bytes]                 ← XOR'd from snapshot of LCG
// [Entry 2: ...]
// [Entry 2 data: ...]
// ...
// [EOF]
// ```
//
// ## Archive layout (RGSS3 — separate index)
//
// ```text
// [Header: 8 bytes (magic + version)]
// [Base key: 4 bytes]
// [Entry 1: offset(4) | size(4) | key(4) | nameLen(4) | name(n)]  ← XOR'd
// [Entry 2: ...]
// [u32 zero offset → end of index]
// [Entry 1 data at absolute offset]
// [Entry 2 data at absolute offset]
// ...
// ```

use crate::FsError;
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Magic headers
// ---------------------------------------------------------------------------

const RGSS1_HEADER: &[u8; 8] = b"RGSSAD\x00\x01";
const RGSS2_HEADER: &[u8; 8] = b"RGSSAD\x00\x02";
const RGSS3_HEADER: &[u8; 8] = b"RGSSAD\x00\x03";

/// Seed value for the LCG-based XOR stream.
const LCG_SEED: u32 = 0xDEAD_CAFE;

// ---------------------------------------------------------------------------
// LCG XOR stream
// ---------------------------------------------------------------------------

/// Advance the LCG and return the **old** value.
///
/// `lcg = lcg * 7 + 3` (wrapping).  This matches mkxp-z's
/// `advanceMagic` — the returned value is used for XOR.
#[inline]
fn advance_lcg(magic: &mut u32) -> u32 {
    let old = *magic;
    *magic = magic.wrapping_mul(7).wrapping_add(3);
    old
}

// ---------------------------------------------------------------------------
// RgssArchive
// ---------------------------------------------------------------------------

/// An RGSS encrypted archive parsed into memory.
///
/// Constructed from the raw bytes of a `.rgssad`, `.rgss2a`, or
/// `.rgss3a` file.  The entire archive must fit in memory.
///
/// # Examples
///
/// ```text
/// // See the `tests` module for roundtrip examples with synthetic
/// // RGSS1 and RGSS3 archives, and ignored integration tests that
/// // verify parsing against real RPG Maker game files.
/// ```
#[derive(Debug)]
pub struct RgssArchive {
    data: Vec<u8>,
    entries: HashMap<String, RgssEntry>,
    directories: HashSet<String>,
}

#[derive(Debug)]
struct RgssEntry {
    /// Absolute byte offset into `data`.
    offset: usize,
    /// File size in bytes.
    size: usize,
    /// LCG state at the start of this file's data.
    start_magic: u32,
}

impl RgssArchive {
    /// Parse raw RGSS archive bytes.  The format (RGSS1/2/3) is
    /// auto-detected from the 8-byte header.
    ///
    /// # Errors
    ///
    /// Returns `FsError::UnsupportedArchive` for unknown headers.
    /// Returns `FsError::Parse` (via `MkxpError`) for truncated data.
    pub fn parse(raw: Vec<u8>) -> Result<Self, FsError> {
        if raw.len() < 8 {
            return Err(FsError::parse(
                "RGSS archive too small (expected >= 8 bytes)",
            ));
        }

        let header = &raw[..8];

        let result = if header == RGSS1_HEADER || header == RGSS2_HEADER {
            Self::parse_rgss1(raw)
        } else if header == RGSS3_HEADER {
            Self::parse_rgss3(raw)
        } else {
            Err(FsError::UnsupportedArchive(format!(
                "unknown RGSS header: {:02X?}",
                header
            )))
        };

        if let Ok(ref archive) = result {
            tracing::info!(files = archive.file_count(), "parsed RGSS archive");
        }

        result
    }

    /// Read and decrypt a file from the archive.
    ///
    /// Returns `FsError::NotFound` if the path is not in the index.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, FsError> {
        let entry = self.entries.get(path).ok_or_else(|| FsError::NotFound {
            path: path.to_string(),
        })?;
        let raw = &self.data[entry.offset..entry.offset + entry.size];
        Ok(decrypt_data(raw, entry.start_magic))
    }

    /// Check whether a file exists in the archive index.
    pub fn file_exists(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }

    /// List immediate children (files and directories) inside `dir`.
    ///
    /// Directory entries are suffixed with `"/"`.  Pass `""` (root) to
    /// list top-level entries.
    ///
    /// Returns `FsError::NotADirectory` if `dir` is not a known
    /// directory.
    pub fn enumerate_dir(&self, dir: &str) -> Result<Vec<String>, FsError> {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            if !self.directories.contains(dir)
                && !self
                    .entries
                    .keys()
                    .any(|k| k.starts_with(&format!("{dir}/")))
            {
                return Err(FsError::NotADirectory {
                    path: dir.to_string(),
                });
            }
            format!("{dir}/")
        };

        let mut seen = HashSet::new();
        let mut result = Vec::new();

        for name in self.entries.keys() {
            if !name.starts_with(&prefix) {
                continue;
            }
            let rest = &name[prefix.len()..];
            if let Some(slash_pos) = rest.find('/') {
                let dir_name = &rest[..slash_pos];
                if seen.insert(dir_name) {
                    result.push(format!("{dir_name}/"));
                }
            } else if seen.insert(rest) {
                result.push(rest.to_string());
            }
        }

        // Sort for deterministic output.
        result.sort();
        Ok(result)
    }

    /// Return all file paths (for path-cache building).
    pub fn all_paths(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    /// Number of files in the archive.
    pub fn file_count(&self) -> usize {
        self.entries.len()
    }
}

// ---------------------------------------------------------------------------
// Mountable trait — makes RgssArchive usable with FileSystem
// ---------------------------------------------------------------------------

impl crate::mountable::Mountable for RgssArchive {
    fn read(&self, path: &crate::VPath) -> Result<Vec<u8>, FsError> {
        self.read_file(path.as_str())
    }

    fn exists(&self, path: &crate::VPath) -> bool {
        self.file_exists(path.as_str())
    }

    fn enumerate(&self, dir: &crate::VPath) -> Result<Vec<String>, FsError> {
        self.enumerate_dir(dir.as_str())
    }
}

// ---- internal parsers -----------------------------------------------

impl RgssArchive {
    /// Parse RGSS1 / RGSS2 format (interleaved entries + data).
    fn parse_rgss1(raw: Vec<u8>) -> Result<Self, FsError> {
        let mut entries = HashMap::new();
        let mut directories = HashSet::new();
        let mut pos: usize = 8; // past header
        let mut magic = LCG_SEED;

        loop {
            // --- Read name length (u32 LE, XOR'd) ---
            if pos + 4 > raw.len() {
                break; // EOF — no more entries
            }
            let name_len_raw = read_u32_le(&raw, pos);
            pos += 4;
            let name_len = (name_len_raw ^ advance_lcg(&mut magic)) as usize;

            // --- Read filename (XOR'd byte by byte) ---
            if pos + name_len > raw.len() {
                return Err(FsError::parse("RGSS filename truncated"));
            }
            let mut name_bytes = Vec::with_capacity(name_len);
            for _ in 0..name_len {
                let c = raw[pos] ^ (advance_lcg(&mut magic) as u8);
                pos += 1;
                // mkxp-z normalises backslashes to forward slashes.
                name_bytes.push(if c == b'\\' { b'/' } else { c });
            }

            // --- Read file size (u32 LE, XOR'd) ---
            if pos + 4 > raw.len() {
                return Err(FsError::parse("RGSS entry size truncated"));
            }
            let size_raw = read_u32_le(&raw, pos);
            pos += 4;
            let size = (size_raw ^ advance_lcg(&mut magic)) as usize;

            // --- Entry data ---
            let data_offset = pos;
            let start_magic = magic; // snapshot of LCG at data start

            let name = decode_filename(&name_bytes)?;

            // Track directory structure.
            for (i, _) in name.match_indices('/') {
                let dir = &name[..i];
                directories.insert(dir.to_string());
            }

            entries.insert(
                name,
                RgssEntry {
                    offset: data_offset,
                    size: size as usize,
                    start_magic,
                },
            );

            // Skip past the file data to the next entry header.
            pos = data_offset + size as usize;
            if pos > raw.len() {
                return Err(FsError::parse(
                    "RGSS file data extends past archive end".to_string(),
                ));
            }
        }

        Ok(Self {
            data: raw,
            entries,
            directories,
        })
    }

    /// Parse RGSS3 format (separate index + data).
    fn parse_rgss3(raw: Vec<u8>) -> Result<Self, FsError> {
        let mut entries = HashMap::new();
        let mut directories = HashSet::new();
        let mut pos: usize = 8; // past header

        if pos + 4 > raw.len() {
            return Err(FsError::parse("RGSS3 base key missing"));
        }
        let base_key_raw = read_u32_le(&raw, pos);
        pos += 4;
        let base_key = base_key_raw.wrapping_mul(9).wrapping_add(3);

        loop {
            // --- Read offset (u32 LE, XOR'd with baseKey) ---
            if pos + 4 > raw.len() {
                return Err(FsError::parse("RGSS3 entry offset truncated"));
            }
            let entry_offset = read_u32_le(&raw, pos) ^ base_key;
            pos += 4;

            // Zero offset = end of index.
            if entry_offset == 0 {
                break;
            }

            // --- Read size ---
            if pos + 4 > raw.len() {
                return Err(FsError::parse("RGSS3 entry size truncated"));
            }
            let entry_size = (read_u32_le(&raw, pos) ^ base_key) as usize;
            pos += 4;

            // --- Read per-file key ---
            if pos + 4 > raw.len() {
                return Err(FsError::parse("RGSS3 entry key truncated"));
            }
            let entry_key = read_u32_le(&raw, pos) ^ base_key;
            pos += 4;

            // --- Read name length ---
            if pos + 4 > raw.len() {
                return Err(FsError::parse("RGSS3 entry name length truncated"));
            }
            let name_len = (read_u32_le(&raw, pos) ^ base_key) as usize;
            pos += 4;

            // --- Read filename (XOR'd with baseKey bytes) ---
            if pos + name_len > raw.len() {
                return Err(FsError::parse("RGSS3 filename truncated"));
            }
            let base_key_bytes = base_key.to_le_bytes();
            let mut name_bytes = Vec::with_capacity(name_len);
            for i in 0..name_len {
                let c = raw[pos + i] ^ base_key_bytes[i % 4];
                name_bytes.push(if c == b'\\' { b'/' } else { c });
            }
            pos += name_len;

            let name = decode_filename(&name_bytes)?;

            for (i, _) in name.match_indices('/') {
                let dir = &name[..i];
                directories.insert(dir.to_string());
            }

            entries.insert(
                name,
                RgssEntry {
                    offset: entry_offset as usize,
                    size: entry_size,
                    start_magic: entry_key,
                },
            );
        }

        Ok(Self {
            data: raw,
            entries,
            directories,
        })
    }
}

// ---------------------------------------------------------------------------
// Decryption
// ---------------------------------------------------------------------------

/// Decrypt file data using the LCG XOR stream.
///
/// `start_magic` is the LCG state at the beginning of the data, as
/// captured during index parsing.
fn decrypt_data(data: &[u8], start_magic: u32) -> Vec<u8> {
    let mut result = data.to_vec();
    let mut magic = start_magic;
    let dword_count = data.len() / 4;

    // Aligned dwords: XOR each with the old value of advance_lcg.
    for i in 0..dword_count {
        let off = i * 4;
        let dword = u32::from_le_bytes(result[off..off + 4].try_into().unwrap());
        let key = advance_lcg(&mut magic);
        result[off..off + 4].copy_from_slice(&(dword ^ key).to_le_bytes());
    }

    // Remaining bytes (0–3): XOR with the first bytes of the current
    // magic value.  This matches mkxp-z's postAlign logic.
    let remaining_start = dword_count * 4;
    let remaining = data.len() - remaining_start;
    if remaining > 0 {
        let key_bytes = magic.to_le_bytes();
        for i in 0..remaining {
            result[remaining_start + i] ^= key_bytes[i];
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn read_u32_le(data: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap())
}

/// Decode a Shift_JIS filename, falling back to lossy UTF-8.
fn decode_filename(raw: &[u8]) -> Result<String, FsError> {
    if let Ok(s) = std::str::from_utf8(raw) {
        return Ok(s.to_string());
    }

    let (cow, _enc, had_errors) = encoding_rs::SHIFT_JIS.decode(raw);
    if had_errors {
        // Last resort — lossy UTF-8.
        return Ok(String::from_utf8_lossy(raw).into_owned());
    }
    Ok(cow.into_owned())
}

// ---------------------------------------------------------------------------
// Tests — synthetic archive builders
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(unused_variables)]
pub fn build_rgss1(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(RGSS1_HEADER);

    let mut magic = LCG_SEED;

    for (name, data) in files {
        let name_bytes = name.as_bytes();

        // nameLen (XOR'd)
        let name_len_raw = (name_bytes.len() as u32) ^ advance_lcg(&mut magic);
        buf.extend_from_slice(&name_len_raw.to_le_bytes());

        // name (each byte XOR'd)
        for &b in name_bytes {
            let c = b ^ (advance_lcg(&mut magic) as u8);
            buf.push(c);
        }

        // size (XOR'd)
        let size_raw = (data.len() as u32) ^ advance_lcg(&mut magic);
        buf.extend_from_slice(&size_raw.to_le_bytes());

        // data (XOR'd with the current LCG state)
        let data_start_magic = magic;
        let enc = encrypt_data(data, data_start_magic);
        buf.extend_from_slice(&enc);
    }

    buf
}

#[cfg(test)]
#[allow(unused_variables)]
pub fn build_rgss3(files: &[(&str, &[u8])]) -> Vec<u8> {
    let base_key_raw: u32 = 0x1234_5678;
    let base_k = base_key_raw.wrapping_mul(9).wrapping_add(3);
    let base_k_bytes = base_k.to_le_bytes();

    let mut buf = Vec::new();
    buf.extend_from_slice(RGSS3_HEADER);
    buf.extend_from_slice(&base_key_raw.to_le_bytes());

    // Pass 1: calculate the byte offset where data area begins
    // (header + base_key + index entries + sentinel).
    let data_start: u32 = 8  // header
        + 4  // base_key_raw
        + (files.iter().map(|(n, _)| 4 + 4 + 4 + 4 + n.len()).sum::<usize>() as u32)  // entries
        + 4; // sentinel u32

    let mut offsets: Vec<u32> = Vec::new();
    let mut next_offset = data_start;
    for (name, data) in files {
        offsets.push(next_offset);
        next_offset += data.len() as u32;
    }

    // Pass 2: write index entries.
    for (i, (name, data)) in files.iter().enumerate() {
        let name_bytes = name.as_bytes();
        let file_key: u32 = 0xDEAD_0000 + i as u32;

        buf.extend_from_slice(&(offsets[i] ^ base_k).to_le_bytes());
        buf.extend_from_slice(&((data.len() as u32) ^ base_k).to_le_bytes());
        buf.extend_from_slice(&(file_key ^ base_k).to_le_bytes());
        buf.extend_from_slice(&((name_bytes.len() as u32) ^ base_k).to_le_bytes());

        for (j, &b) in name_bytes.iter().enumerate() {
            let c = b ^ base_k_bytes[j % 4];
            buf.push(c);
        }
    }

    // Sentinel: zero offset (raw = base_k so that raw ^ base_k = 0).
    buf.extend_from_slice(&base_k.to_le_bytes());

    // Pass 3: write encrypted data.
    for (i, (name, data)) in files.iter().enumerate() {
        let file_key: u32 = 0xDEAD_0000 + i as u32;
        buf.extend_from_slice(&encrypt_data(data, file_key));
    }

    buf
}

/// Inverse of `decrypt_data` — for building test archives.
#[cfg(test)]
fn encrypt_data(data: &[u8], start_magic: u32) -> Vec<u8> {
    // XOR is its own inverse, so encrypt == decrypt.
    decrypt_data(data, start_magic)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- LCG ------------------------------------------------------------

    #[test]
    fn lcg_first_values() {
        let mut m = LCG_SEED;
        // mkxp-z: seeds from MAGIC, advances for each dword.
        // First advance returns MAGIC, then MAGIC*7+3, etc.
        assert_eq!(advance_lcg(&mut m), 0xDEAD_CAFE);
        assert_eq!(
            advance_lcg(&mut m),
            0xDEAD_CAFE_u32.wrapping_mul(7).wrapping_add(3)
        );
    }

    #[test]
    fn xor_roundtrip() {
        let data: Vec<u8> = (0..256).map(|i| i as u8).collect();
        let magic = 0x1234ABCD;
        let enc = encrypt_data(&data, magic);
        let dec = decrypt_data(&enc, magic);
        assert_eq!(dec, data);
    }

    #[test]
    fn xor_unaligned_data() {
        // Data lengths that aren't multiples of 4 should still roundtrip.
        for len in [0, 1, 2, 3, 4, 5, 7, 13, 64, 127] {
            let data: Vec<u8> = (0..len).map(|i| (i * 7) as u8).collect();
            let magic = 0xABCD0000 + len as u32;
            let enc = encrypt_data(&data, magic);
            let dec = decrypt_data(&enc, magic);
            assert_eq!(dec, data, "len={len}");
        }
    }

    // ---- Format detection -----------------------------------------------

    #[test]
    fn detect_rgss1() {
        let raw = build_rgss1(&[("a.txt", b"hello")]);
        assert!(RgssArchive::parse(raw).is_ok());
    }

    #[test]
    fn detect_rgss3() {
        let raw = build_rgss3(&[("a.txt", b"hello")]);
        assert!(RgssArchive::parse(raw).is_ok());
    }

    #[test]
    fn reject_unknown_header() {
        let mut raw = vec![0u8; 64];
        raw[..8].copy_from_slice(b"XXXXXXXX");
        assert!(matches!(
            RgssArchive::parse(raw).unwrap_err(),
            FsError::UnsupportedArchive(_)
        ));
    }

    #[test]
    fn reject_too_small() {
        assert!(RgssArchive::parse(vec![0x52, 0x47]).is_err());
    }

    // ---- Roundtrip ------------------------------------------------------

    #[test]
    fn roundtrip_rgss1_single() {
        let raw = build_rgss1(&[("hello.txt", b"hello world")]);
        let a = RgssArchive::parse(raw).unwrap();
        assert_eq!(a.file_count(), 1);
        assert_eq!(a.read_file("hello.txt").unwrap(), b"hello world");
    }

    #[test]
    fn roundtrip_rgss1_multiple() {
        let files: &[(&str, &[u8])] =
            &[("a.txt", b"aaa"), ("b.txt", b"bbb"), ("sub/c.txt", b"ccc")];
        let raw = build_rgss1(files);
        let a = RgssArchive::parse(raw).unwrap();
        assert_eq!(a.file_count(), 3);
        assert_eq!(a.read_file("a.txt").unwrap(), b"aaa");
        assert_eq!(a.read_file("b.txt").unwrap(), b"bbb");
        assert_eq!(a.read_file("sub/c.txt").unwrap(), b"ccc");
    }

    #[test]
    fn roundtrip_rgss1_binary_data() {
        // Binary content with null bytes and high bytes.
        let data: Vec<u8> = (0..=255).collect();
        let raw = build_rgss1(&[("binary.bin", &data)]);
        let a = RgssArchive::parse(raw).unwrap();
        assert_eq!(a.read_file("binary.bin").unwrap(), data);
    }

    #[test]
    fn roundtrip_rgss3_single() {
        let raw = build_rgss3(&[("data.txt", b"rgss3 content")]);
        let a = RgssArchive::parse(raw).unwrap();
        assert_eq!(a.file_count(), 1);
        assert_eq!(a.read_file("data.txt").unwrap(), b"rgss3 content");
    }

    #[test]
    fn roundtrip_rgss3_multiple() {
        let files: &[(&str, &[u8])] = &[
            ("Graphics/Titles/title.png", b"pngdata"),
            ("Data/Scripts.rxdata", b"rubyscripts"),
        ];
        let raw = build_rgss3(files);
        let a = RgssArchive::parse(raw).unwrap();
        assert_eq!(a.file_count(), 2);
        assert_eq!(
            a.read_file("Graphics/Titles/title.png").unwrap(),
            b"pngdata"
        );
        assert_eq!(a.read_file("Data/Scripts.rxdata").unwrap(), b"rubyscripts");
    }

    #[test]
    fn read_nonexistent() {
        let raw = build_rgss1(&[("a.txt", b"")]);
        let a = RgssArchive::parse(raw).unwrap();
        assert!(matches!(
            a.read_file("nope.txt").unwrap_err(),
            FsError::NotFound { .. }
        ));
    }

    // ---- file_exists ----------------------------------------------------

    #[test]
    fn file_exists() {
        let raw = build_rgss1(&[("exists.txt", b"yes")]);
        let a = RgssArchive::parse(raw).unwrap();
        assert!(a.file_exists("exists.txt"));
        assert!(!a.file_exists("no.txt"));
    }

    // ---- enumerate_dir --------------------------------------------------

    #[test]
    fn enumerate_root() {
        let files: &[(&str, &[u8])] = &[
            ("Graphics/Titles/title.png", b""),
            ("Graphics/Autotiles/grass.png", b""),
            ("Data/Scripts.rxdata", b""),
        ];
        let raw = build_rgss1(files);
        let a = RgssArchive::parse(raw).unwrap();

        let entries = a.enumerate_dir("").unwrap();
        assert_eq!(entries, vec!["Data/", "Graphics/"]);
    }

    #[test]
    fn enumerate_subdirectory() {
        let files: &[(&str, &[u8])] = &[
            ("Graphics/Titles/title.png", b""),
            ("Graphics/Titles/gameover.png", b""),
            ("Graphics/Characters/hero.png", b""),
        ];
        let raw = build_rgss1(files);
        let a = RgssArchive::parse(raw).unwrap();

        let entries = a.enumerate_dir("Graphics/Titles").unwrap();
        assert_eq!(entries, vec!["gameover.png", "title.png"]);
    }

    #[test]
    fn enumerate_nonexistent_dir() {
        let raw = build_rgss1(&[("a.txt", b"")]);
        let a = RgssArchive::parse(raw).unwrap();
        assert!(matches!(
            a.enumerate_dir("Nope").unwrap_err(),
            FsError::NotADirectory { .. }
        ));
    }

    // ---- Shift_JIS filenames --------------------------------------------

    #[test]
    fn shift_jis_filename() {
        // "グラフィック" (Graphics) in Shift_JIS
        let sjis_name: &[u8] = &[
            0x83, 0x4F, 0x83, 0x89, 0x83, 0x74, 0x83, 0x42, 0x83, 0x62, 0x83, 0x4E,
        ];
        let decoded = decode_filename(sjis_name).unwrap();
        assert!(!decoded.is_empty());
        assert!(!decoded.contains('\u{FFFD}')); // no replacement characters
    }

    #[test]
    fn shift_jis_in_archive() {
        // File named with Shift_JIS bytes directly.
        let sjis: &[u8] = &[0x83, 0x65, 0x83, 0x58, 0x83, 0x67]; // "テスト" (test)
        let decoded = decode_filename(sjis).unwrap();
        // Build archive with the decoded UTF-8 name.
        let raw = build_rgss1(&[(&decoded, b"data")]);
        let a = RgssArchive::parse(raw).unwrap();
        assert_eq!(a.read_file(&decoded).unwrap(), b"data");
    }

    // ---- Empty archive --------------------------------------------------

    #[test]
    fn empty_rgss1_archive() {
        let mut buf = Vec::new();
        buf.extend_from_slice(RGSS1_HEADER);
        // No entries — EOF after header.
        let a = RgssArchive::parse(buf).unwrap();
        assert_eq!(a.file_count(), 0);
    }

    #[test]
    fn empty_rgss3_archive() {
        let mut buf = Vec::new();
        buf.extend_from_slice(RGSS3_HEADER);
        let base_key_raw = 0x1234_5678u32;
        let base_k = base_key_raw.wrapping_mul(9).wrapping_add(3);
        buf.extend_from_slice(&base_key_raw.to_le_bytes());
        // Sentinel: zero offset.
        buf.extend_from_slice(&base_k.to_le_bytes());

        let a = RgssArchive::parse(buf).unwrap();
        assert_eq!(a.file_count(), 0);
    }
}
