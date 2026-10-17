//! Duplicate detection commands.
//!
//! Detection runs on a blocking thread and publishes `duplicates://progress`. The report is kept in
//! memory; nothing about file contents is ever persisted.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager, State};

use crate::duplicates::grouping::{self, DuplicateOptions, DuplicateReport};
use crate::duplicates::verify::verify_identical;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub const DUPLICATE_PROGRESS_EVENT: &str = "duplicates://progress";

/// Mirrors `DuplicateProgress`.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateProgress {
    pub report_id: String,
    pub phase: String,
    pub files_hashed: u64,
    pub bytes_hashed: u64,
    pub candidate_groups: usize,
    pub groups_found: usize,
    pub elapsed_ms: u64,
    pub eta_ms: Option<u64>,
}

/// Starts duplicate detection and returns the report id immediately.
///
/// Duplicate detection is a free local feature. The only input data is the completed scan index; file
/// contents are streamed locally and no report is persisted.
#[tauri::command]
pub async fn find_duplicates(
    options: DuplicateOptions,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    // Excluded folders may arrive as identity tokens from the file list; resolve them first.
    let mut options = options;
    options.exclude_paths = options
        .exclude_paths
        .iter()
        .map(|path| crate::disclosure::resolve_path(&state, Some(&options.scan_id), path))
        .collect::<AppResult<Vec<String>>>()?;

    let scans = state.scans()?;
    let entry = scans
        .get(&options.scan_id)
        .ok_or_else(|| AppError::NotFound(format!("Unknown scan {}", options.scan_id)))?
        .clone();
    drop(scans);

    let index = entry
        .index
        .clone()
        .ok_or_else(|| AppError::NotFound("The scan is still running.".to_string()))?;

    let report_id = format!("dup-{}", crate::time::now_millis());
    let started_at = crate::time::now_millis();
    let cancelled = Arc::new(AtomicBool::new(false));

    // Mark the scan as running before returning its ID so callers can immediately poll/cancel it.
    {
        let mut cancellations = state
            .duplicate_cancellations
            .lock()
            .map_err(|_| AppError::Internal("The duplicate cancellation lock was poisoned.".to_string()))?;
        let mut registry = state
            .duplicate_reports
            .lock()
            .map_err(|_| AppError::Internal("The report lock was poisoned.".to_string()))?;
        if registry.values().any(|report| report.status == "running") {
            return Err(AppError::InvalidArgument("Duplicate detection is already running.".to_string()));
        }
        registry.clear();
        registry.insert(
            report_id.clone(),
            DuplicateReport {
                report_id: report_id.clone(),
                scan_id: options.scan_id.clone(),
                started_at,
                finished_at: None,
                status: "running".to_string(),
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
            },
        );
        cancellations.insert(report_id.clone(), Arc::clone(&cancelled));
    }

    let progress_id = report_id.clone();
    let progress_app = app.clone();
    let progress_options = options.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let mut last_emit: Option<Instant> = None;

        // Snapshot active metadata under a short read lock, then release it before hashing so a
        // long duplicate run does not block safe deletion or other index readers. Live paths are
        // rechecked after hashing and become typed errors if they vanished or changed meanwhile.
        let active_files: Vec<crate::filesystem::FileRecord> = {
            let index_guard = index.read().unwrap_or_else(|poisoned| poisoned.into_inner());
            index_guard.active_files().cloned().collect()
        };
        let active_file_refs: Vec<&crate::filesystem::FileRecord> = active_files.iter().collect();
        let report = grouping::build_report_with_progress(
            &active_file_refs,
            &progress_options,
            progress_id.clone(),
            started_at,
            &cancelled,
            |phase, files_hashed, bytes_hashed, candidate_groups, groups_found| {
                let should_emit = phase == "grouping"
                    || last_emit
                        .map(|last| last.elapsed() >= Duration::from_millis(150))
                        .unwrap_or(true);
                if should_emit {
                    let payload = DuplicateProgress {
                        report_id: progress_id.clone(),
                        phase: phase.to_string(),
                        files_hashed,
                        bytes_hashed,
                        candidate_groups,
                        groups_found,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                        eta_ms: None,
                    };
                    let _ = progress_app.emit(DUPLICATE_PROGRESS_EVENT, &payload);
                    last_emit = Some(Instant::now());
                }
            },
        );

        let payload = DuplicateProgress {
            report_id: progress_id.clone(),
            phase: if report.status == "cancelled" { "cancelled" } else { "done" }.to_string(),
            files_hashed: report.files_hashed,
            bytes_hashed: report.hashed_bytes,
            candidate_groups: report.candidate_groups,
            groups_found: report.total_groups,
            elapsed_ms: started.elapsed().as_millis() as u64,
            eta_ms: None,
        };
        let state = progress_app.state::<AppState>();
        if let Ok(mut registry) = state.duplicate_reports.lock() {
            registry.insert(progress_id.clone(), report);
        }
        if let Ok(mut cancellations) = state.duplicate_cancellations.lock() {
            cancellations.remove(&progress_id);
        }
        let _ = progress_app.emit(DUPLICATE_PROGRESS_EVENT, &payload);
    });

    Ok(report_id)
}

/// Requests cancellation of an in-flight duplicate scan.
#[tauri::command]
pub fn duplicates_cancel(report_id: String, state: State<'_, AppState>) -> AppResult<()> {
    let cancellations = state
        .duplicate_cancellations
        .lock()
        .map_err(|_| AppError::Internal("The duplicate cancellation lock was poisoned.".to_string()))?;
    if let Some(flag) = cancellations.get(&report_id) {
        flag.store(true, Ordering::SeqCst);
        return Ok(());
    }
    drop(cancellations);

    let reports = state
        .duplicate_reports
        .lock()
        .map_err(|_| AppError::Internal("The report lock was poisoned.".to_string()))?;
    if reports.contains_key(&report_id) {
        Ok(()) // Already completed (or already cancelled).
    } else {
        Err(AppError::NotFound(format!("Unknown duplicate report {report_id}")))
    }
}

/// Returns a finished report, or `None` when it is unknown or still running.
#[tauri::command]
pub fn duplicates_get(report_id: String, state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    let result = duplicates_get_typed(report_id, &state)?;
    crate::disclosure::disclosed(&result)
}

fn duplicates_get_typed(report_id: String, state: &AppState) -> AppResult<Option<DuplicateReport>> {
    let registry = state
        .duplicate_reports
        .lock()
        .map_err(|_| AppError::Internal("The report lock was poisoned.".to_string()))?;
    Ok(registry.get(&report_id).cloned())
}

/// Byte-for-byte comparison of two files, used by the "are these really identical?" action.
#[tauri::command]
pub async fn verify_duplicates(
    path_a: String,
    path_b: String,
    state: State<'_, AppState>,
) -> AppResult<crate::duplicates::verify::VerifyResult> {
    let path_a = crate::disclosure::resolve_path(&state, None, &path_a)?;
    let path_b = crate::disclosure::resolve_path(&state, None, &path_b)?;
    if path_a == path_b {
        return Err(AppError::InvalidArgument("Choose two different files to compare.".to_string()));
    }
    crate::commands::files::validate_path(&path_a)?;
    crate::commands::files::validate_path(&path_b)?;
    let is_reported_pair = {
        let reports = state
            .duplicate_reports
            .lock()
            .map_err(|_| AppError::Internal("The report lock was poisoned.".to_string()))?;
        reports.values().any(|report| report_contains_pair(report, &path_a, &path_b))
    };
    if !is_reported_pair {
        return Err(AppError::InvalidArgument(
            "Only two files in the same completed duplicate group can be byte-verified.".to_string(),
        ));
    }

    let error_path = path_a.clone();
    tauri::async_runtime::spawn_blocking(move || {
        verify_identical(&std::path::PathBuf::from(path_a), &std::path::PathBuf::from(path_b))
    })
    .await
    .map_err(|error| AppError::Internal(format!("The byte-verification worker failed: {error}")))?
    .map_err(|error| AppError::io(error_path, &error))
}

fn report_contains_pair(report: &DuplicateReport, path_a: &str, path_b: &str) -> bool {
    report.status != "running"
        && report.groups.iter().any(|group| {
            group.size_bytes <= grouping::MAX_DUPLICATE_FILE_BYTES
                && group.copies.iter().any(|copy| copy.path == path_a)
                && group.copies.iter().any(|copy| copy.path == path_b)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_verification_requires_two_members_of_the_same_finished_group() {
        let report = DuplicateReport {
            report_id: "r".to_string(),
            scan_id: "s".to_string(),
            started_at: 0,
            finished_at: Some(1),
            status: "complete".to_string(),
            algorithm: "sha256".to_string(),
            groups: vec![
                crate::duplicates::grouping::DuplicateGroup {
                    group_id: "g1".to_string(),
                    size_bytes: 10,
                    reclaimable_bytes: 10,
                    copies: vec![
                        duplicate_copy("C:\\a.bin", 10),
                        duplicate_copy("C:\\b.bin", 10),
                    ],
                    hash: "hash-1".to_string(),
                },
                crate::duplicates::grouping::DuplicateGroup {
                    group_id: "g2".to_string(),
                    size_bytes: 20,
                    reclaimable_bytes: 20,
                    copies: vec![duplicate_copy("C:\\c.bin", 20), duplicate_copy("C:\\d.bin", 20)],
                    hash: "hash-2".to_string(),
                },
            ],
            candidate_groups: 2,
            total_groups: 2,
            total_copies: 4,
            total_reclaimable_bytes: 30,
            hashed_bytes: 60,
            files_hashed: 4,
            skipped_large_files: 0,
            skipped_large_bytes: 0,
            errors: Vec::new(),
        };
        assert!(report_contains_pair(&report, "C:\\a.bin", "C:\\b.bin"));
        assert!(!report_contains_pair(&report, "C:\\a.bin", "C:\\c.bin"));
        assert!(!report_contains_pair(&report, "C:\\a.bin", "C:\\missing.bin"));

        let mut running = report;
        running.status = "running".to_string();
        assert!(!report_contains_pair(&running, "C:\\a.bin", "C:\\b.bin"));
    }

    fn duplicate_copy(path: &str, size_bytes: u64) -> crate::duplicates::grouping::DuplicateFile {
        crate::duplicates::grouping::DuplicateFile {
            path: path.to_string(),
            name: path.rsplit('\\').next().unwrap_or_default().to_string(),
            size_bytes,
            modified_at: None,
            is_suggested_original: false,
        }
    }

    #[test]
    fn the_progress_payload_shape_matches_the_contract() {
        let payload = DuplicateProgress {
            report_id: "r".to_string(),
            phase: "partial-hashing".to_string(),
            files_hashed: 1,
            bytes_hashed: 2,
            candidate_groups: 3,
            groups_found: 0,
            elapsed_ms: 3,
            eta_ms: None,
        };
        let json = serde_json::to_value(payload).unwrap();
        assert_eq!(json["phase"], "partial-hashing");
        assert_eq!(json["candidateGroups"], 3);
        assert_eq!(json["bytesHashed"], 2);
    }

}
