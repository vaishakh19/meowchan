//! Size grouping, hashing and report assembly.
//!
//! The pipeline is deliberately staged so the expensive step (reading file contents) only runs on
//! files that already share an exact size with at least one other file. On a real volume that
//! removes the overwhelming majority of candidates for free.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use serde::{Deserialize, Serialize};

use crate::duplicates::hashing::{hash_file_cancellable, partial_hash_file, HashAlgorithm, PARTIAL_HASH_BYTES};
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
    pub candidate_groups: usize,
    pub total_groups: usize,
    pub total_copies: usize,
    pub total_reclaimable_bytes: u64,
    pub hashed_bytes: u64,
    pub files_hashed: u64,
    pub skipped_large_files: u64,
    pub skipped_large_bytes: u64,
    pub errors: Vec<WalkError>,
}

/// Hard cap: duplicate detection never hashes a file larger than 5 GiB.
pub const MAX_DUPLICATE_FILE_BYTES: u64 = 5 * 1024 * 1024 * 1024;

fn effective_max_bytes(options: &DuplicateOptions) -> u64 {
    if options.max_bytes == 0 {
        MAX_DUPLICATE_FILE_BYTES
    } else {
        options.max_bytes.min(MAX_DUPLICATE_FILE_BYTES)
    }
}

fn matches_non_size_filters(file: &FileRecord, options: &DuplicateOptions) -> bool {
    if file.is_symlink || file.is_junction || file.is_cloud_placeholder {
        return false;
    }
    if !options.extensions.is_empty() {
        match &file.extension {
            Some(extension) if options.extensions.iter().any(|value| {
                value.trim_start_matches('.').eq_ignore_ascii_case(extension)
            }) => {}
            _ => return false,
        }
    }
    !is_excluded(&file.path, &options.exclude_paths)
}

/// Stage 1: bucket ordinary files by exact size. Link entries, cloud placeholders, and paths the
/// user excluded never reach a content-reading stage.
pub fn group_by_size<'a>(
    files: &'a [FileRecord],
    options: &DuplicateOptions,
) -> Vec<(u64, Vec<&'a FileRecord>)> {
    let references: Vec<&FileRecord> = files.iter().collect();
    group_refs_by_size(&references, options)
}

fn group_refs_by_size<'a>(
    files: &[&'a FileRecord],
    options: &DuplicateOptions,
) -> Vec<(u64, Vec<&'a FileRecord>)> {
    group_refs_by_size_cancellable(files, options, None).unwrap_or_default()
}

fn group_refs_by_size_cancellable<'a>(
    files: &[&'a FileRecord],
    options: &DuplicateOptions,
    cancelled: Option<&AtomicBool>,
) -> Option<Vec<(u64, Vec<&'a FileRecord>)>> {
    let mut buckets: HashMap<u64, Vec<&FileRecord>> = HashMap::new();
    let max_bytes = effective_max_bytes(options);
    for (index, file) in files.iter().copied().enumerate() {
        if index % 1024 == 0 && is_cancelled(cancelled) {
            return None;
        }
        if file.size < options.min_bytes || file.size > max_bytes || !matches_non_size_filters(file, options) {
            continue;
        }
        buckets.entry(file.size).or_default().push(file);
    }
    if is_cancelled(cancelled) {
        return None;
    }
    let mut out: Vec<(u64, Vec<&FileRecord>)> = buckets
        .into_iter()
        .filter(|(_, group)| group.len() > 1)
        .collect();
    out.sort_by(|a, b| {
        let savings_a = a.0.saturating_mul(a.1.len().saturating_sub(1) as u64);
        let savings_b = b.0.saturating_mul(b.1.len().saturating_sub(1) as u64);
        savings_b.cmp(&savings_a).then_with(|| a.0.cmp(&b.0))
    });
    Some(out)
}

fn skipped_large_files(
    files: &[&FileRecord],
    options: &DuplicateOptions,
    cancelled: Option<&AtomicBool>,
) -> Option<(u64, u64)> {
    let mut count = 0u64;
    let mut bytes = 0u64;
    for (index, file) in files.iter().enumerate() {
        if index % 1024 == 0 && is_cancelled(cancelled) {
            return None;
        }
        if file.size > MAX_DUPLICATE_FILE_BYTES
            && file.size >= options.min_bytes
            && matches_non_size_filters(file, options)
        {
            count = count.saturating_add(1);
            bytes = bytes.saturating_add(file.size);
        }
    }
    Some((count, bytes))
}

fn is_cancelled(cancelled: Option<&AtomicBool>) -> bool {
    cancelled.map(|flag| flag.load(Ordering::SeqCst)).unwrap_or(false)
}

fn is_excluded(path: &Path, exclusions: &[String]) -> bool {
    if exclusions.is_empty() {
        return false;
    }
    let text = path.to_string_lossy().to_lowercase().replace('/', "\\");
    exclusions.iter().any(|exclusion| {
        let needle = exclusion.to_lowercase().replace('/', "\\");
        let needle = needle.trim_end_matches('\\');
        !needle.is_empty() && (text == needle || text.starts_with(&format!("{needle}\\")))
    })
}

/// Compatibility wrapper used by unit tests. The command path uses the cancellable progress API.
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
    let not_cancelled = AtomicBool::new(false);
    let references: Vec<&FileRecord> = files.iter().collect();
    build_report_with_progress(&references, options, report_id, started_at, &not_cancelled, |_, files, bytes, _, _| {
        on_progress(files, bytes);
    })
}

/// Size -> partial SHA-256 sample -> full streaming SHA-256. Cancellation is checked between file
/// reads, and progress is emitted after each candidate rather than only after an entire size group.
pub fn build_report_with_progress<F>(
    files: &[&FileRecord],
    options: &DuplicateOptions,
    report_id: String,
    started_at: i64,
    cancelled: &AtomicBool,
    mut on_progress: F,
) -> DuplicateReport
where
    F: FnMut(&'static str, u64, u64, usize, usize),
{
    on_progress("grouping", 0, 0, 0, 0);
    let grouping = group_refs_by_size_cancellable(files, options, Some(cancelled)).and_then(|size_groups| {
        skipped_large_files(files, options, Some(cancelled)).map(|skipped| (size_groups, skipped))
    });
    let (size_groups, (skipped_large_files, skipped_large_bytes)) = match grouping {
        Some(result) => result,
        None => {
            on_progress("finalizing", 0, 0, 0, 0);
            return DuplicateReport {
                report_id,
                scan_id: options.scan_id.clone(),
                started_at,
                finished_at: Some(now_millis()),
                status: "cancelled".to_string(),
                algorithm: options.algorithm.as_str().to_string(),
                groups: Vec::new(),
                candidate_groups: 0,
                total_groups: 0,
                total_copies: 0,
                total_reclaimable_bytes: 0,
                hashed_bytes: 0,
                files_hashed: 0,
                skipped_large_files: 0,
                skipped_large_bytes: 0,
                errors: Vec::new(),
            };
        }
    };
    let candidate_groups = size_groups.len();
    let mut groups: Vec<DuplicateGroup> = Vec::new();
    let mut errors: Vec<WalkError> = Vec::new();
    let mut hashed_bytes = 0u64;
    let mut files_hashed = 0u64;
    on_progress("grouping", 0, 0, candidate_groups, 0);

    'size_groups: for (size, candidates) in &size_groups {
        let mut partial_buckets: HashMap<String, Vec<&FileRecord>> = HashMap::new();
        for file in candidates {
            if cancelled.load(Ordering::SeqCst) {
                break 'size_groups;
            }
            match partial_hash_file(&file.path, file.size, Some(cancelled)) {
                Ok((digest, bytes_read)) => {
                    files_hashed = files_hashed.saturating_add(1);
                    hashed_bytes = hashed_bytes.saturating_add(bytes_read);
                    partial_buckets.entry(digest).or_default().push(file);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                    break 'size_groups;
                }
                Err(error) => errors.push(WalkError {
                    path: file.path.to_string_lossy().to_string(),
                    kind: hashing_error_kind(&error),
                    message: error.to_string(),
                }),
            }
            on_progress("partial-hashing", files_hashed, hashed_bytes, candidate_groups, groups.len());
        }

        for partial_copies in partial_buckets.values() {
            if partial_copies.len() < 2 {
                continue;
            }
            let mut full_hashes: HashMap<String, Vec<&FileRecord>> = HashMap::new();
            let mut interrupted = false;
            for file in partial_copies {
                if cancelled.load(Ordering::SeqCst) {
                    interrupted = true;
                    break;
                }
                match hash_file_cancellable(&file.path, options.algorithm, Some(file.size), Some(cancelled)) {
                    Ok((digest, bytes_read)) => {
                        hashed_bytes = hashed_bytes.saturating_add(bytes_read);
                        full_hashes.entry(digest).or_default().push(file);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                        interrupted = true;
                        break;
                    }
                    Err(error) => errors.push(WalkError {
                        path: file.path.to_string_lossy().to_string(),
                        kind: hashing_error_kind(&error),
                        message: error.to_string(),
                    }),
                }
                on_progress("hashing", files_hashed, hashed_bytes, candidate_groups, groups.len());
            }
            if interrupted {
                break 'size_groups;
            }

            for (digest, copies) in full_hashes {
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
                        is_suggested_original: false,
                    })
                    .collect();
                entries.sort_by(|a, b| {
                    a.modified_at
                        .unwrap_or(i64::MAX)
                        .cmp(&b.modified_at.unwrap_or(i64::MAX))
                        .then_with(|| Path::new(&a.path).components().count().cmp(&Path::new(&b.path).components().count()))
                        .then_with(|| a.path.cmp(&b.path))
                });
                if let Some(first) = entries.first_mut() {
                    first.is_suggested_original = true;
                }
                let copies_count = entries.len() as u64;
                groups.push(DuplicateGroup {
                    group_id: format!("grp-{size}-{digest}"),
                    size_bytes: *size,
                    reclaimable_bytes: size.saturating_mul(copies_count.saturating_sub(1)),
                    copies: entries,
                    hash: digest,
                });
            }
            on_progress("hashing", files_hashed, hashed_bytes, candidate_groups, groups.len());
        }
    }

    retain_live_copies(&mut groups, &mut errors);
    groups.sort_by(|a, b| b.reclaimable_bytes.cmp(&a.reclaimable_bytes));
    let total_groups = groups.len();
    let total_copies: usize = groups.iter().map(|group| group.copies.len()).sum();
    let total_reclaimable_bytes = groups
        .iter()
        .fold(0u64, |total, group| total.saturating_add(group.reclaimable_bytes));
    let status = if cancelled.load(Ordering::SeqCst) {
        "cancelled"
    } else if errors.is_empty() && skipped_large_files == 0 {
        "complete"
    } else {
        "partial"
    };
    on_progress("finalizing", files_hashed, hashed_bytes, candidate_groups, total_groups);

    DuplicateReport {
        report_id,
        scan_id: options.scan_id.clone(),
        started_at,
        finished_at: Some(now_millis()),
        status: status.to_string(),
        algorithm: options.algorithm.as_str().to_string(),
        groups,
        candidate_groups,
        total_groups,
        total_copies,
        total_reclaimable_bytes,
        hashed_bytes,
        files_hashed,
        skipped_large_files,
        skipped_large_bytes,
        errors,
    }
}

fn hashing_error_kind(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => "access-denied".to_string(),
        std::io::ErrorKind::NotFound => "not-found".to_string(),
        _ => "io".to_string(),
    }
}

/// Rechecks candidate copies before returning a report. A copy that vanished, became a link, or
/// changed since the scan is omitted, and the report becomes partial through its typed errors.
fn retain_live_copies(groups: &mut Vec<DuplicateGroup>, errors: &mut Vec<WalkError>) {
    for group in groups.iter_mut() {
        let before = group.copies.len();
        let mut retained = Vec::with_capacity(before);
        for copy in group.copies.drain(..) {
            let path = Path::new(&copy.path);
            let checked = crate::filesystem::entries::ensure_no_reparse_components(path)
                .and_then(|()| crate::filesystem::entries::read_metadata(path, 0));
            match checked {
                Ok(metadata)
                    if metadata.kind == crate::filesystem::Kind::File
                        && metadata.size == copy.size_bytes
                        && copy.modified_at.map(|expected| metadata.modified_at == Some(expected)).unwrap_or(true) =>
                {
                    retained.push(copy);
                }
                Ok(_) => errors.push(WalkError {
                    path: copy.path,
                    kind: "changed".to_string(),
                    message: "The file changed after hashing and was omitted from the report.".to_string(),
                }),
                Err(error) => errors.push(WalkError {
                    path: copy.path,
                    kind: hashing_error_kind(&error),
                    message: error.to_string(),
                }),
            }
        }
        let after = retained.len();
        group.copies = retained;
        if after < before {
            group.reclaimable_bytes = group
                .size_bytes
                .saturating_mul(after.saturating_sub(1) as u64);
        }
    }
    groups.retain(|group| group.copies.len() >= 2);
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

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
            depth: 1,
            ..Default::default()
        }
    }

    fn tiny_file_options() -> DuplicateOptions {
        DuplicateOptions { min_bytes: 0, ..Default::default() }
    }

    #[test]
    fn groups_only_files_that_share_an_exact_size() {
        let files = vec![
            record("C:\\a.bin", 100),
            record("C:\\b.bin", 100),
            record("C:\\c.bin", 200),
            record("C:\\tiny.bin", 1),
        ];
        let options = tiny_file_options();
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
            ..tiny_file_options()
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
            ..tiny_file_options()
        };
        assert!(group_by_size(&files, &options).is_empty());
    }

    #[test]
    fn exclusions_match_only_the_selected_path_and_descendants() {
        let exclusions = vec!["C:/Users/me/files/".to_string()];
        assert!(is_excluded(Path::new(r"C:\Users\me\files\a.bin"), &exclusions));
        assert!(!is_excluded(Path::new(r"C:\Users\me\files-other\a.bin"), &exclusions));
        let file = vec![r"C:\a.bin".to_string()];
        assert!(is_excluded(Path::new(r"C:\a.bin"), &file));
        assert!(!is_excluded(Path::new(r"C:\a.bin.backup"), &file));
        assert!(is_excluded(Path::new("/tmp/files/a.bin"), &["/tmp/files".to_string()]));
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
            ..tiny_file_options()
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
        let options = tiny_file_options();
        let report = build_report(&files, &options, "r1".to_string(), 0, |_, _| {});

        assert_eq!(report.total_groups, 1);
        assert_eq!(report.total_copies, 2);
        assert_eq!(report.groups[0].size_bytes, 12);
        assert_eq!(report.groups[0].reclaimable_bytes, 12);
        assert_eq!(report.algorithm, "sha256");
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
            &tiny_file_options(),
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
            &tiny_file_options(),
            "r3".to_string(),
            0,
            |_, _| {},
        );
        assert_eq!(report.total_groups, 0);
        assert_eq!(report.errors.len(), 2);
        assert_eq!(report.errors[0].kind, "not-found");
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
            &tiny_file_options(),
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
            &tiny_file_options(),
            "r5".to_string(),
            0,
            |_, _| {},
        );
        assert_eq!(report.total_groups, 2);
        assert!(report.groups[0].reclaimable_bytes > report.groups[1].reclaimable_bytes);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_hash_collisions_require_a_full_sha256_match() {
        let dir = std::env::temp_dir().join(format!("diskwise-partial-dup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let first = dir.join("first.bin");
        let second = dir.join("second.bin");
        let a = vec![0u8; 200 * 1024];
        let mut b = a.clone();
        b[90 * 1024] = 1; // middle differs; the sampled prefix and suffix still match
        fs::write(&first, &a).unwrap();
        fs::write(&second, &b).unwrap();
        let files = vec![record(&first.to_string_lossy(), a.len() as u64), record(&second.to_string_lossy(), b.len() as u64)];
        let report = build_report(&files, &tiny_file_options(), "partial".to_string(), 0, |_, _| {});
        assert_eq!(report.total_groups, 0);
        assert_eq!(report.files_hashed, 2);
        assert!(report.hashed_bytes > (PARTIAL_HASH_BYTES * 2 * 2) as u64);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn files_above_the_five_gibibyte_cap_are_reported_not_hashed() {
        let size = MAX_DUPLICATE_FILE_BYTES + 1;
        let files = vec![record("C:/huge-a.bin", size), record("C:/huge-b.bin", size)];
        let report = build_report(&files, &tiny_file_options(), "large".to_string(), 0, |_, _| {});
        assert_eq!(report.skipped_large_files, 2);
        assert_eq!(report.skipped_large_bytes, size.saturating_mul(2));
        assert_eq!(report.files_hashed, 0);
        assert_eq!(report.total_groups, 0);
        assert_eq!(report.status, "partial");
    }

    #[test]
    fn duplicate_detection_never_hashes_reparse_entries() {
        let mut first = record("C:/one.bin", 100);
        let mut second = record("C:/two.bin", 100);
        first.is_symlink = true;
        second.is_symlink = true;
        assert!(group_by_size(&[first, second], &tiny_file_options()).is_empty());
    }

    #[test]
    fn cancellation_returns_an_explicit_cancelled_report() {
        let cancelled = AtomicBool::new(true);
        let files = vec![record("C:/one.bin", 10), record("C:/two.bin", 10)];
        let references: Vec<&FileRecord> = files.iter().collect();
        let report = build_report_with_progress(
            &references,
            &tiny_file_options(),
            "cancelled".to_string(),
            0,
            &cancelled,
            |_, _, _, _, _| {},
        );
        assert_eq!(report.status, "cancelled");
        assert!(report.errors.is_empty());
    }

    #[test]
    fn a_copy_changed_after_hashing_is_omitted_from_the_report() {
        let directory = std::env::temp_dir().join(format!("diskora-duplicate-live-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let first = directory.join("first.bin");
        let second = directory.join("second.bin");
        fs::write(&first, b"same bytes").unwrap();
        fs::write(&second, b"same bytes").unwrap();
        let mut groups = vec![DuplicateGroup {
            group_id: "g".to_string(),
            size_bytes: 10,
            reclaimable_bytes: 10,
            copies: [first, second]
                .iter()
                .map(|path| DuplicateFile {
                    path: path.to_string_lossy().to_string(),
                    name: path.file_name().unwrap().to_string_lossy().to_string(),
                    size_bytes: 10,
                    modified_at: None,
                    is_suggested_original: false,
                })
                .collect(),
            hash: "abc".to_string(),
        }];
        let mut errors = Vec::new();
        retain_live_copies(&mut groups, &mut errors);
        assert_eq!(groups.len(), 1);
        assert!(errors.is_empty());

        fs::write(&second, b"changed length").unwrap();
        retain_live_copies(&mut groups, &mut errors);
        assert!(groups.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, "changed");
        let _ = fs::remove_dir_all(directory);
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
