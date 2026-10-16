//! Size grouping, hashing and report assembly.
//!
//! The pipeline is deliberately staged so the expensive step (reading file contents) only runs on
//! files that already share an exact size with at least one other file. On a real volume that
//! removes the overwhelming majority of candidates for free.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::duplicates::hashing::{hash_file, HashAlgorithm};
use crate::filesystem::FileRecord;
use crate::scanner::walker::WalkError;

/// Mirrors `DuplicateOptions` in `src/types/contracts.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DuplicateOptions {
    pub scan_id: String,
    pub min_bytes: u64,
    pub max_bytes: u64,
    pub exclude_paths: Vec<String>,
    pub algorithm: HashAlgorithm,
    pub extensions: Vec<String>,
}

impl Default for DuplicateOptions {
    fn default() -> Self {
        DuplicateOptions {
            scan_id: String::new(),
            min_bytes: 1024,
            max_bytes: 5 * 1024 * 1024 * 1024,
            exclude_paths: Vec::new(),
            algorithm: HashAlgorithm::Sha256,
            extensions: Vec::new(),
        }
    }
}

/// Mirrors `DuplicateFile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateFile {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub modified_at: Option<i64>,
    pub byte_verified: bool,
    pub is_suggested_original: bool,
}

/// Mirrors `DuplicateGroup`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub group_id: String,
    pub size_bytes: u64,
    pub reclaimable_bytes: u64,
    pub copies: Vec<DuplicateFile>,
    pub hash: String,
    /// `size` = grouped by size only, `hash` = confirmed by content hash, `verified` = byte compared.
    pub confidence: String,
}

/// Mirrors `DuplicateReport`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateReport {
    pub report_id: String,
    pub scan_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub algorithm: String,
    pub groups: Vec<DuplicateGroup>,
    pub total_groups: usize,
    pub total_copies: usize,
    pub total_reclaimable_bytes: u64,
    pub hashed_bytes: u64,
    pub files_hashed: u64,
    pub errors: Vec<WalkError>,
}

/// Truncates a report to the largest `limit` groups, recomputing every total exactly.
///
/// This is the enforcement point for the free tier's group limit. It runs in the backend, after
/// the report is built, so no amount of interface tampering can lift it. `limit` of `None` leaves
/// the report untouched.
///
/// The recomputation is exact rather than a running delta, because the UI must never show a size
/// the backend cannot account for.
pub fn limit_groups(report: DuplicateReport, limit: Option<usize>) -> DuplicateReport {
    let Some(limit) = limit else {
        return report;
    };
    if report.groups.len() <= limit {
        return report;
    }

    // Groups are already ordered largest reclaimable first; re-sorting after truncation is cheap
    // and guarantees the ordering the UI relies on.
    let mut kept: Vec<DuplicateGroup> = report.groups.into_iter().take(limit).collect();
    kept.sort_by(|a, b| b.reclaimable_bytes.cmp(&a.reclaimable_bytes));

    let total_copies: usize = kept.iter().map(|group| group.copies.len()).sum();
    let total_reclaimable_bytes: u64 = kept.iter().map(|group| group.reclaimable_bytes).sum();

    DuplicateReport {
        groups: kept,
        total_groups: limit,
        total_copies,
        total_reclaimable_bytes,
        ..report
    }
}

/// Stage 1: bucket files by exact size. Cheap, and removes almost everything.
pub fn group_by_size<'a>(
    files: &'a [FileRecord],
    options: &DuplicateOptions,
) -> Vec<(u64, Vec<&'a FileRecord>)> {
    let mut buckets: HashMap<u64, Vec<&FileRecord>> = HashMap::new();
    for file in files {
        if file.size < options.min_bytes || file.size > options.max_bytes {
            continue;
        }
        if !options.extensions.is_empty() {
            match &file.extension {
                Some(extension) if options.extensions.iter().any(|e| e == extension) => {}
                _ => continue,
            }
        }
        if is_excluded(&file.path, &options.exclude_paths) {
            continue;
        }
        buckets.entry(file.size).or_default().push(file);
    }
    let mut out: Vec<(u64, Vec<&FileRecord>)> = buckets
        .into_iter()
        .filter(|(_, group)| group.len() > 1)
        .collect();
    // Biggest reclaimable potential first.
    out.sort_by(|a, b| {
        let savings_a = a.0 * (a.1.len() as u64 - 1);
        let savings_b = b.0 * (b.1.len() as u64 - 1);
        savings_b.cmp(&savings_a).then_with(|| a.0.cmp(&b.0))
    });
    out
}

fn is_excluded(path: &Path, exclusions: &[String]) -> bool {
    if exclusions.is_empty() {
        return false;
    }
    let text = path.to_string_lossy().to_lowercase();
    exclusions.iter().any(|exclusion| {
        let needle = exclusion.to_lowercase().replace('/', "\\");
        text.starts_with(&needle) || text.contains(&needle)
    })
}

/// Stage 2 + 3: hash each size group, then optionally verify byte-for-byte.
///
/// `on_progress` is called with (files hashed, bytes hashed) after each group so the emitter can
/// publish throttled progress without the scanner holding a lock.
pub fn build_report<F>(
    files: &[FileRecord],
    options: &DuplicateOptions,
    report_id: String,
    started_at: i64,
    mut on_progress: F,
) -> DuplicateReport
where
    F: FnMut(u64, u64),
{
    let size_groups = group_by_size(files, options);

    let mut groups: Vec<DuplicateGroup> = Vec::new();
    let mut errors: Vec<WalkError> = Vec::new();
    let mut hashed_bytes = 0u64;
    let mut files_hashed = 0u64;

    for (size, candidates) in &size_groups {
        // Hash in parallel: each file is independent and I/O bound.
        let hashed: Vec<(&FileRecord, Option<String>)> = candidates
            .par_iter()
            .map(|file| (*file, hash_file(&file.path, options.algorithm).ok()))
            .collect();

        let mut by_hash: HashMap<String, Vec<&FileRecord>> = HashMap::new();
        for (file, digest) in hashed {
            match digest {
                Some(digest) => {
                    hashed_bytes += file.size;
                    files_hashed += 1;
                    by_hash.entry(digest).or_default().push(file);
                }
                None => errors.push(WalkError {
                    path: file.path.to_string_lossy().to_string(),
                    kind: "access-denied".to_string(),
                    message: "The file could not be read for hashing.".to_string(),
                }),
            }
        }
        on_progress(files_hashed, hashed_bytes);

        for (digest, copies) in by_hash {
            if copies.len() < 2 {
                continue;
            }
            let mut entries: Vec<DuplicateFile> = copies
                .iter()
                .map(|file| DuplicateFile {
                    path: file.path.to_string_lossy().to_string(),
                    name: file.name.clone(),
                    size_bytes: file.size,
                    modified_at: file.modified_at,
                    byte_verified: false,
                    is_suggested_original: false,
                })
                .collect();

            // Suggested original: oldest, then shallowest path, then lexicographic.
            entries.sort_by(|a, b| {
                a.modified_at
                    .unwrap_or(i64::MAX)
                    .cmp(&b.modified_at.unwrap_or(i64::MAX))
                    .then_with(|| a.path.matches('\\').count().cmp(&b.path.matches('\\').count()))
                    .then_with(|| a.path.cmp(&b.path))
            });
            if let Some(first) = entries.first_mut() {
                first.is_suggested_original = true;
            }

            let copies_count = entries.len() as u64;
            groups.push(DuplicateGroup {
                group_id: format!("grp-{size}-{}", &digest[..8.min(digest.len())]),
                size_bytes: *size,
                reclaimable_bytes: size * (copies_count - 1),
                copies: entries,
                hash: digest,
                confidence: "hash".to_string(),
            });
        }
    }

    // Biggest savings first.
    groups.sort_by(|a, b| b.reclaimable_bytes.cmp(&a.reclaimable_bytes));

    let total_groups = groups.len();
    let total_copies: usize = groups.iter().map(|group| group.copies.len()).sum();
    let total_reclaimable_bytes: u64 = groups.iter().map(|group| group.reclaimable_bytes).sum();

    DuplicateReport {
        report_id,
        scan_id: options.scan_id.clone(),
        started_at,
        finished_at: None,
        status: "complete".to_string(),
        algorithm: options.algorithm.as_str().to_string(),
        groups,
        total_groups,
        total_copies,
        total_reclaimable_bytes,
        hashed_bytes,
        files_hashed,
        errors,
    }
}

/// Marks a group as byte-verified after a successful comparison.
pub fn mark_verified(group: &mut DuplicateGroup) {
    group.confidence = "verified".to_string();
    for copy in group.copies.iter_mut() {
        copy.byte_verified = true;
    }
}

/// Removes paths that no longer exist, so the report never offers a vanished file.
pub fn retain_existing(groups: &mut Vec<DuplicateGroup>) -> usize {
    let mut removed = 0usize;
    for group in groups.iter_mut() {
        let before = group.copies.len();
        group.copies
            .retain(|copy| PathBuf::from(&copy.path).exists());
        let after = group.copies.len();
        if after < before {
            removed += before - after;
            group.reclaimable_bytes = group.size_bytes.saturating_mul(after.saturating_sub(1) as u64);
            if after < 2 {
                group.reclaimable_bytes = 0;
            }
        }
    }
    groups.retain(|group| group.copies.len() >= 2);
    removed
}

/// The safe default for a fresh group list: nothing preselected for removal.
pub fn nothing_preselected(groups: &[DuplicateGroup]) -> Vec<String> {
    let _ = groups;
    Vec::new()
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    fn group(id: &str, size: u64, copies: usize) -> DuplicateGroup {
        DuplicateGroup {
            group_id: id.to_string(),
            size_bytes: size,
            reclaimable_bytes: size * (copies.saturating_sub(1) as u64),
            copies: (0..copies)
                .map(|index| DuplicateFile {
                    path: format!("C:\\{id}-{index}.bin"),
                    name: format!("{id}-{index}.bin"),
                    size_bytes: size,
                    modified_at: None,
                    byte_verified: false,
                    is_suggested_original: index == 0,
                })
                .collect(),
            hash: "h".to_string(),
            confidence: "hash".to_string(),
        }
    }

    fn report_of(count: usize) -> DuplicateReport {
        let groups: Vec<DuplicateGroup> = (0..count)
            .map(|index| group(&format!("g{index}"), 10_000 * (index as u64 + 1), 2))
            .collect();
        let total_copies: usize = groups.iter().map(|group| group.copies.len()).sum();
        let total_reclaimable_bytes: u64 = groups.iter().map(|group| group.reclaimable_bytes).sum();
        DuplicateReport {
            report_id: "r".to_string(),
            scan_id: "s".to_string(),
            started_at: 0,
            finished_at: Some(1),
            status: "complete".to_string(),
            algorithm: "sha256".to_string(),
            total_groups: groups.len(),
            total_copies,
            total_reclaimable_bytes,
            hashed_bytes: 0,
            files_hashed: 0,
            groups,
            errors: Vec::new(),
        }
    }

    #[test]
    fn a_report_within_the_limit_is_untouched() {
        let report = report_of(3);
        let before = report.total_reclaimable_bytes;
        let limited = limit_groups(report, Some(5));
        assert_eq!(limited.groups.len(), 3);
        assert_eq!(limited.total_reclaimable_bytes, before);
    }

    #[test]
    fn a_report_over_the_limit_keeps_the_largest_groups() {
        let report = report_of(5);
        // Each group reclaims `size` (two copies, one kept), so the three largest are 50k + 40k + 30k.
        let limited = limit_groups(report, Some(3));
        assert_eq!(limited.groups.len(), 3);
        assert_eq!(limited.total_groups, 3);
        assert_eq!(limited.total_reclaimable_bytes, 120_000);
        assert_eq!(limited.total_copies, 6);
        assert_eq!(limited.groups[0].size_bytes, 50_000);
        assert_eq!(limited.groups[2].size_bytes, 30_000);
    }

    #[test]
    fn no_limit_means_everything_is_kept() {
        let report = report_of(5);
        let limited = limit_groups(report, None);
        assert_eq!(limited.groups.len(), 5);
        assert_eq!(limited.total_reclaimable_bytes, 150_000);
    }

    #[test]
    fn the_totals_stay_arithmetically_exact() {
        let report = report_of(10);
        let limited = limit_groups(report, Some(4));
        let sum: u64 = limited.groups.iter().map(|group| group.reclaimable_bytes).sum();
        assert_eq!(sum, limited.total_reclaimable_bytes);
        let copies: usize = limited.groups.iter().map(|group| group.copies.len()).sum();
        assert_eq!(copies, limited.total_copies);
    }

    #[test]
    fn truncation_never_leaves_zero_copies() {
        let report = report_of(6);
        let limited = limit_groups(report, Some(2));
        for kept in &limited.groups {
            assert_eq!(kept.copies.len(), 2, "one original plus one reclaimable copy");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::hashing::hash_bytes;
    use std::fs;

    fn record(path: &str, size: u64) -> FileRecord {
        FileRecord {
            path: PathBuf::from(path),
            name: PathBuf::from(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            size,
            disk_size: None,
            modified_at: None,
            extension: None,
            parent_path: PathBuf::from("C:\\"),
            depth: 1,
        }
    }

    #[test]
    fn groups_only_files_that_share_an_exact_size() {
        let files = vec![
            record("C:\\a.bin", 100),
            record("C:\\b.bin", 100),
            record("C:\\c.bin", 200),
            record("C:\\tiny.bin", 1),
        ];
        let options = DuplicateOptions::default();
        let groups = group_by_size(&files, &options);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, 100);
        assert_eq!(groups[0].1.len(), 2);
    }

    #[test]
    fn respects_min_and_max_size() {
        let files = vec![
            record("C:\\a.bin", 10),
            record("C:\\b.bin", 10),
            record("C:\\big1.bin", 10_000),
            record("C:\\big2.bin", 10_000),
        ];
        let options = DuplicateOptions {
            min_bytes: 1000,
            max_bytes: 20_000,
            ..Default::default()
        };
        let groups = group_by_size(&files, &options);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, 10_000);
    }

    #[test]
    fn respects_excluded_paths() {
        let files = vec![
            record("C:\\Users\\me\\a.bin", 100),
            record("C:\\Users\\me\\b.bin", 100),
        ];
        let options = DuplicateOptions {
            exclude_paths: vec!["C:\\Users\\me".to_string()],
            ..Default::default()
        };
        assert!(group_by_size(&files, &options).is_empty());
    }

    #[test]
    fn respects_extension_filters() {
        let mut a = record("C:\\a.mp4", 100);
        a.extension = Some("mp4".to_string());
        let mut b = record("C:\\b.mp4", 100);
        b.extension = Some("mp4".to_string());
        let mut c = record("C:\\c.txt", 100);
        c.extension = Some("txt".to_string());
        let files = vec![a, b, c];
        let options = DuplicateOptions {
            extensions: vec!["mp4".to_string()],
            ..Default::default()
        };
        let groups = group_by_size(&files, &options);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1.len(), 2);
    }

    #[test]
    fn detects_real_duplicates_by_content() {
        let dir = std::env::temp_dir().join(format!("diskwise-dup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let one = dir.join("one.bin");
        let two = dir.join("two.bin");
        let three = dir.join("three.bin");
        fs::write(&one, b"same content").unwrap();
        fs::write(&two, b"same content").unwrap();
        fs::write(&three, b"different!!").unwrap();

        let files = vec![
            record(&one.to_string_lossy(), 12),
            record(&two.to_string_lossy(), 12),
            record(&three.to_string_lossy(), 11),
        ];
        let options = DuplicateOptions::default();
        let report = build_report(&files, &options, "r1".to_string(), 0, |_, _| {});

        assert_eq!(report.total_groups, 1);
        assert_eq!(report.total_copies, 2);
        assert_eq!(report.groups[0].size_bytes, 12);
        assert_eq!(report.groups[0].reclaimable_bytes, 12);
        assert_eq!(report.algorithm, "sha256");
        assert_eq!(report.groups[0].confidence, "hash");
        assert_eq!(report.groups[0].copies.len(), 2);
        // Nothing is preselected: exactly one copy is flagged as the suggested original.
        assert_eq!(
            report.groups[0]
                .copies
                .iter()
                .filter(|copy| copy.is_suggested_original)
                .count(),
            1
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_size_different_content_is_not_a_duplicate() {
        let dir = std::env::temp_dir().join(format!("diskwise-nodup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let one = dir.join("one.bin");
        let two = dir.join("two.bin");
        fs::write(&one, b"aaaa").unwrap();
        fs::write(&two, b"bbbb").unwrap();
        let files = vec![
            record(&one.to_string_lossy(), 4),
            record(&two.to_string_lossy(), 4),
        ];
        let report = build_report(
            &files,
            &DuplicateOptions::default(),
            "r2".to_string(),
            0,
            |_, _| {},
        );
        assert_eq!(report.total_groups, 0, "equal size is not enough");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_files_are_reported_not_skipped_silently() {
        let files = vec![
            record("C:\\missing-1.bin", 100),
            record("C:\\missing-2.bin", 100),
        ];
        let report = build_report(
            &files,
            &DuplicateOptions::default(),
            "r3".to_string(),
            0,
            |_, _| {},
        );
        assert_eq!(report.total_groups, 0);
        assert_eq!(report.errors.len(), 2);
        assert_eq!(report.errors[0].kind, "access-denied");
    }

    #[test]
    fn progress_is_reported() {
        let dir = std::env::temp_dir().join(format!("diskwise-prog-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let one = dir.join("one.bin");
        let two = dir.join("two.bin");
        fs::write(&one, b"xyz").unwrap();
        fs::write(&two, b"xyz").unwrap();
        let files = vec![
            record(&one.to_string_lossy(), 3),
            record(&two.to_string_lossy(), 3),
        ];
        let mut seen = 0u64;
        build_report(
            &files,
            &DuplicateOptions::default(),
            "r4".to_string(),
            0,
            |files, _| {
                if files > seen {
                    seen = files;
                }
            },
        );
        assert_eq!(seen, 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn groups_are_ordered_by_reclaimable_space() {
        let dir = std::env::temp_dir().join(format!("diskwise-order-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut records = Vec::new();
        for (size, copies) in [(10u64, 2u64), (1000u64, 3u64)] {
            for index in 0..copies {
                let path = dir.join(format!("s{size}-{index}.bin"));
                fs::write(&path, vec![0u8; size as usize]).unwrap();
                records.push(record(&path.to_string_lossy(), size));
            }
        }
        let report = build_report(
            &records,
            &DuplicateOptions::default(),
            "r5".to_string(),
            0,
            |_, _| {},
        );
        assert_eq!(report.total_groups, 2);
        assert!(report.groups[0].reclaimable_bytes > report.groups[1].reclaimable_bytes);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verified_groups_are_labelled() {
        let mut group = DuplicateGroup {
            group_id: "g".to_string(),
            size_bytes: 10,
            reclaimable_bytes: 10,
            copies: vec![],
            hash: hash_bytes(b"x"),
            confidence: "hash".to_string(),
        };
        mark_verified(&mut group);
        assert_eq!(group.confidence, "verified");
    }

    #[test]
    fn vanished_copies_are_dropped_from_the_report() {
        let mut groups = vec![DuplicateGroup {
            group_id: "g".to_string(),
            size_bytes: 100,
            reclaimable_bytes: 200,
            copies: vec![
                DuplicateFile {
                    path: "C:\\nope-a.bin".to_string(),
                    name: "a".to_string(),
                    size_bytes: 100,
                    modified_at: None,
                    byte_verified: false,
                    is_suggested_original: false,
                },
                DuplicateFile {
                    path: "C:\\nope-b.bin".to_string(),
                    name: "b".to_string(),
                    size_bytes: 100,
                    modified_at: None,
                    byte_verified: false,
                    is_suggested_original: false,
                },
            ],
            hash: "abc".to_string(),
            confidence: "hash".to_string(),
        }];
        retain_existing(&mut groups);
        assert!(groups.is_empty(), "a group with fewer than two real copies is dropped");
    }

    #[test]
    fn nothing_is_preselected_by_default() {
        assert!(nothing_preselected(&[]).is_empty());
    }

    #[test]
    fn options_defaults_are_conservative() {
        let options = DuplicateOptions::default();
        assert_eq!(options.min_bytes, 1024);
        assert_eq!(options.max_bytes, 5 * 1024 * 1024 * 1024);
        assert!(options.exclude_paths.is_empty());
        assert!(options.extensions.is_empty());
        assert_eq!(options.algorithm, HashAlgorithm::Sha256);
    }
}
