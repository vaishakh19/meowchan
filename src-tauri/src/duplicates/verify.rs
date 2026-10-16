//! Byte-for-byte verification.
//!
//! Used when the user asks for certainty beyond a hash match, and to double-check the two files the
//! UI is about to compare. Streams both files in lockstep and reports the first differing offset.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use serde::Serialize;

use crate::filesystem::long_path::to_unc_long_path;
use crate::duplicates::hashing::HASH_BUFFER_BYTES;

/// Mirrors `DuplicateVerifyResult` in `src/types/contracts.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResult {
    pub equal: bool,
    pub bytes_compared: u64,
    pub first_difference_offset: Option<u64>,
}

/// Compares two files byte-for-byte with bounded memory.
pub fn verify_identical(path_a: &Path, path_b: &Path) -> std::io::Result<VerifyResult> {
    let file_a = File::open(to_unc_long_path(path_a))?;
    let file_b = File::open(to_unc_long_path(path_b))?;
    let mut reader_a = BufReader::with_capacity(HASH_BUFFER_BYTES, file_a);
    let mut reader_b = BufReader::with_capacity(HASH_BUFFER_BYTES, file_b);

    let mut buffer_a = vec![0u8; HASH_BUFFER_BYTES];
    let mut buffer_b = vec![0u8; HASH_BUFFER_BYTES];
    let mut offset = 0u64;
    let mut compared = 0u64;

    loop {
        let read_a = reader_a.read(&mut buffer_a)?;
        let read_b = reader_b.read(&mut buffer_b)?;
        if read_a != read_b {
            return Ok(VerifyResult {
                equal: false,
                bytes_compared: compared + read_a.min(read_b) as u64,
                first_difference_offset: Some(offset + read_a.min(read_b) as u64),
            });
        }
        if read_a == 0 {
            return Ok(VerifyResult {
                equal: true,
                bytes_compared: compared,
                first_difference_offset: None,
            });
        }
        for index in 0..read_a {
            if buffer_a[index] != buffer_b[index] {
                return Ok(VerifyResult {
                    equal: false,
                    bytes_compared: compared + index as u64,
                    first_difference_offset: Some(offset + index as u64),
                });
            }
        }
        compared += read_a as u64;
        offset += read_a as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("diskwise-verify-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn identical_files_are_reported_equal() {
        let dir = temp_dir("equal");
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        fs::write(&a, vec![7u8; 4096]).unwrap();
        fs::write(&b, vec![7u8; 4096]).unwrap();
        let result = verify_identical(&a, &b).unwrap();
        assert!(result.equal);
        assert_eq!(result.bytes_compared, 4096);
        assert_eq!(result.first_difference_offset, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_single_differing_byte_is_found() {
        let dir = temp_dir("differ");
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        let mut left = vec![0u8; 4096];
        let mut right = vec![0u8; 4096];
        left[1234] = 1;
        right[1234] = 2;
        fs::write(&a, &left).unwrap();
        fs::write(&b, &right).unwrap();
        let result = verify_identical(&a, &b).unwrap();
        assert!(!result.equal);
        assert_eq!(result.first_difference_offset, Some(1234));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn different_lengths_are_reported_different() {
        let dir = temp_dir("length");
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        fs::write(&a, vec![0u8; 100]).unwrap();
        fs::write(&b, vec![0u8; 200]).unwrap();
        let result = verify_identical(&a, &b).unwrap();
        assert!(!result.equal);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn large_files_are_compared_with_bounded_memory() {
        let dir = temp_dir("large");
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        let payload = vec![3u8; 3 * 1024 * 1024];
        fs::write(&a, &payload).unwrap();
        fs::write(&b, &payload).unwrap();
        let result = verify_identical(&a, &b).unwrap();
        assert!(result.equal);
        assert_eq!(result.bytes_compared, 3 * 1024 * 1024);
        let _ = fs::remove_dir_all(&dir);
    }
}
