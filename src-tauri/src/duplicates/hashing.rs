//! Content hashing.
//!
//! Streaming, bounded memory: a 1 MiB buffer is reused for every file so hashing a 40 GB video costs
//! 1 MiB of RAM, not 40 GB.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::filesystem::long_path::to_unc_long_path;

pub const HASH_BUFFER_BYTES: usize = 1024 * 1024;

/// Supported algorithms. The UI only offers what is compiled in.
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

    pub fn from_name(value: &str) -> Self {
        match value {
            "blake3" | "sha256" => HashAlgorithm::Sha256,
            _ => HashAlgorithm::Sha256,
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
/// Reads through the `\\?\` path so long paths work. A file that disappears mid-read is an error,
/// never a silent zero-length hash.
pub fn hash_file(path: &Path, algorithm: HashAlgorithm) -> std::io::Result<String> {
    match algorithm {
        HashAlgorithm::Sha256 => hash_sha256(path),
    }
}

fn hash_sha256(path: &Path) -> std::io::Result<String> {
    let file = File::open(to_unc_long_path(path))?;
    let mut reader = BufReader::with_capacity(HASH_BUFFER_BYTES, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(to_hex(&hasher.finalize()))
}

/// SHA-256 of an in-memory buffer. Used by the tests and by the synthetic browser transport.
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
        // SHA-256 of the empty string.
        assert_eq!(
            hash_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // SHA-256 of "abc".
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
        assert_eq!(
            hash_file(&path, HashAlgorithm::Sha256).unwrap(),
            hash_bytes(b"abc")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_an_error_not_an_empty_hash() {
        let missing = std::env::temp_dir().join("diskwise-does-not-exist.bin");
        assert!(hash_file(&missing, HashAlgorithm::Sha256).is_err());
    }

    #[test]
    fn algorithm_names_match_the_wire_format() {
        assert_eq!(HashAlgorithm::Sha256.as_str(), "sha256");
        assert_eq!(HashAlgorithm::default(), HashAlgorithm::Sha256);
    }
}
