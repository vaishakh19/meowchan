//! Content hashing.
//!
//! Full hashes stream through a 1 MiB buffer; the duplicate pipeline first hashes only a bounded
//! prefix/suffix sample so that full reads are reserved for collisions. Files are opened without
//! following symlinks/reparse points and cancellation is checked between reads.

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::filesystem::long_path::to_unc_long_path;

pub const HASH_BUFFER_BYTES: usize = 1024 * 1024;
pub const PARTIAL_HASH_BYTES: usize = 64 * 1024;

/// Supported algorithms. Only SHA-256 is exposed by Diskora.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgorithm {
    Sha256,
}

impl HashAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            HashAlgorithm::Sha256 => "sha256",
        }
    }

    pub fn from_name(value: &str) -> Option<Self> {
        match value {
            "sha256" => Some(HashAlgorithm::Sha256),
            _ => None,
        }
    }
}

impl Default for HashAlgorithm {
    fn default() -> Self {
        HashAlgorithm::Sha256
    }
}

/// SHA-256 of a file's contents, lowercase hex.
///
/// A file that disappears mid-read is an error, never a silent zero-length hash.
pub fn hash_file(path: &Path, algorithm: HashAlgorithm) -> io::Result<String> {
    hash_file_cancellable(path, algorithm, None, None).map(|(digest, _)| digest)
}

/// Full streaming hash, optionally validating the scanned size and stopping on cancellation.
pub fn hash_file_cancellable(
    path: &Path,
    algorithm: HashAlgorithm,
    expected_size: Option<u64>,
    cancelled: Option<&AtomicBool>,
) -> io::Result<(String, u64)> {
    let file = open_read_without_following_links(path)?;
    let before = file.metadata()?;
    if let Some(expected) = expected_size {
        if before.len() != expected {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "File size changed since the scan."));
        }
    }
    match algorithm {
        HashAlgorithm::Sha256 => hash_sha256(file, before, expected_size, cancelled),
    }
}

fn hash_sha256(
    file: File,
    before: std::fs::Metadata,
    expected_size: Option<u64>,
    cancelled: Option<&AtomicBool>,
) -> io::Result<(String, u64)> {
    let mut reader = BufReader::with_capacity(HASH_BUFFER_BYTES, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    let mut bytes_read = 0u64;
    loop {
        check_cancelled(cancelled)?;
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes_read = bytes_read.saturating_add(read as u64);
    }
    check_file_unchanged(&before, &reader.get_ref().metadata()?, expected_size)?;
    Ok((to_hex(&hasher.finalize()), bytes_read))
}

/// Partial stage for duplicate detection: hash all content for small files, otherwise hash the
/// first and last 64 KiB. Full SHA-256 is still required before a duplicate group is reported.
pub fn partial_hash_file(
    path: &Path,
    expected_size: u64,
    cancelled: Option<&AtomicBool>,
) -> io::Result<(String, u64)> {
    let mut file = open_read_without_following_links(path)?;
    let before = file.metadata()?;
    if before.len() != expected_size {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "File size changed since the scan."));
    }

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; PARTIAL_HASH_BYTES];
    let bytes_read = if expected_size <= (PARTIAL_HASH_BYTES * 2) as u64 {
        read_and_hash(&mut file, &mut hasher, expected_size, &mut buffer, cancelled)?
    } else {
        let first = read_and_hash(&mut file, &mut hasher, PARTIAL_HASH_BYTES as u64, &mut buffer, cancelled)?;
        file.seek(SeekFrom::Start(expected_size - PARTIAL_HASH_BYTES as u64))?;
        let last = read_and_hash(&mut file, &mut hasher, PARTIAL_HASH_BYTES as u64, &mut buffer, cancelled)?;
        first.saturating_add(last)
    };
    check_file_unchanged(&before, &file.metadata()?, Some(expected_size))?;
    Ok((to_hex(&hasher.finalize()), bytes_read))
}

fn read_and_hash(
    reader: &mut impl Read,
    hasher: &mut Sha256,
    mut remaining: u64,
    buffer: &mut [u8],
    cancelled: Option<&AtomicBool>,
) -> io::Result<u64> {
    let mut bytes_read = 0u64;
    while remaining > 0 {
        check_cancelled(cancelled)?;
        let requested = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = reader.read(&mut buffer[..requested])?;
        if read == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "File ended while it was being hashed."));
        }
        hasher.update(&buffer[..read]);
        let count = read as u64;
        bytes_read = bytes_read.saturating_add(count);
        remaining = remaining.saturating_sub(count);
    }
    Ok(bytes_read)
}

fn check_cancelled(cancelled: Option<&AtomicBool>) -> io::Result<()> {
    if cancelled.map(|flag| flag.load(Ordering::SeqCst)).unwrap_or(false) {
        Err(io::Error::new(io::ErrorKind::Interrupted, "Duplicate hashing was cancelled."))
    } else {
        Ok(())
    }
}

fn check_file_unchanged(
    before: &std::fs::Metadata,
    after: &std::fs::Metadata,
    expected_size: Option<u64>,
) -> io::Result<()> {
    if before.len() != after.len()
        || expected_size.map(|expected| after.len() != expected).unwrap_or(false)
        || before.modified().ok() != after.modified().ok()
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "File changed while it was being hashed."));
    }
    Ok(())
}

pub(crate) fn open_read_without_following_links(path: &Path) -> io::Result<File> {
    crate::filesystem::entries::ensure_no_reparse_components(path)?;
    let long_path = to_unc_long_path(path);
    let mut options = OpenOptions::new();
    options.read(true);

    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
        const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
        const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
        let file = options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(long_path)?;
        let attributes = file.metadata()?.file_attributes();
        if attributes & (FILE_ATTRIBUTE_REPARSE_POINT
            | FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
            != 0
        {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Refusing to hash a reparse point or cloud placeholder."));
        }
        return Ok(file);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = options.custom_flags(libc::O_NOFOLLOW).open(long_path)?;
        if file.metadata()?.file_type().is_symlink() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Refusing to hash a symbolic link."));
        }
        return Ok(file);
    }

    #[cfg(not(any(unix, windows)))]
    {
        options.open(long_path)
    }
}

/// SHA-256 of an in-memory buffer. Used only by tests; real file evidence is always read from disk.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    to_hex(&hasher.finalize())
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn hashes_known_content() {
        assert_eq!(
            hash_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn different_content_hashes_differently() {
        assert_ne!(hash_bytes(b"abc"), hash_bytes(b"abd"));
    }

    #[test]
    fn hashes_a_file_on_disk() {
        let dir = std::env::temp_dir().join(format!("diskwise-hash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blob.bin");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(hash_file(&path, HashAlgorithm::Sha256).unwrap(), hash_bytes(b"abc"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_hash_matches_a_full_hash_for_small_files() {
        let dir = std::env::temp_dir().join(format!("diskwise-partial-small-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("small.bin");
        let bytes = vec![17u8; 70 * 1024];
        fs::write(&path, &bytes).unwrap();
        let (partial, read) = partial_hash_file(&path, bytes.len() as u64, None).unwrap();
        assert_eq!(partial, hash_bytes(&bytes));
        assert_eq!(read, bytes.len() as u64);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn partial_hash_reads_only_a_bounded_prefix_and_suffix_for_large_files() {
        let dir = std::env::temp_dir().join(format!("diskwise-partial-large-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("large.bin");
        let bytes = vec![23u8; PARTIAL_HASH_BYTES * 5];
        fs::write(&path, &bytes).unwrap();
        let (_, read) = partial_hash_file(&path, bytes.len() as u64, None).unwrap();
        assert_eq!(read, (PARTIAL_HASH_BYTES * 2) as u64);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_file_is_an_error_not_an_empty_hash() {
        let missing = std::env::temp_dir().join("diskwise-does-not-exist.bin");
        assert!(hash_file(&missing, HashAlgorithm::Sha256).is_err());
    }

    #[test]
    fn cancellation_stops_streaming_hashes() {
        let dir = std::env::temp_dir().join(format!("diskwise-hash-cancel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blob.bin");
        fs::write(&path, vec![0u8; HASH_BUFFER_BYTES * 2]).unwrap();
        let cancelled = AtomicBool::new(true);
        let result = hash_file_cancellable(&path, HashAlgorithm::Sha256, None, Some(&cancelled));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn algorithm_names_are_not_silently_aliased() {
        assert_eq!(HashAlgorithm::from_name("sha256"), Some(HashAlgorithm::Sha256));
        assert_eq!(HashAlgorithm::from_name("blake3"), None);
        assert_eq!(HashAlgorithm::default(), HashAlgorithm::Sha256);
    }
}
