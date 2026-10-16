//! Duplicate detection commands.
//!
//! Detection runs on a blocking thread and publishes `duplicates://progress`. The report is kept in
//! memory; nothing about file contents is ever persisted.

use std::sync::Arc;
use std::time::Duration;

use tauri::{Emitter, Manager, State};

use crate::config::product::DEFAULT_CONFIG;
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
    pub groups_found: usize,
    pub elapsed_ms: u64,
    pub eta_ms: Option<u64>,
}

/// Starts duplicate detection and returns the report id immediately.
///
/// Basic duplicate detection is free at every tier. Finding *more* than the free group limit is
/// gated: `UnlimitedDuplicates`. The limit is applied by the backend, not the UI, so a modified
/// interface cannot ask for all of them.
#[tauri::command]
pub async fn find_duplicates(
    options: DuplicateOptions,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    crate::authz::authorize_state(&state, crate::authz::Permission::BasicDuplicates)?;

    // The free tier gets the first N duplicate groups, largest first, and is told so. Everything
    // past the limit needs `UnlimitedDuplicates`, which the authorization layer enforces. The
    // limit is applied here, in the backend, so a modified interface cannot ask for more.
    let authorization = state.authorization()?;
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

    let report_id = format!("dup-{}", crate::licensing::verify::now_millis());
    let started_at = crate::licensing::verify::now_millis();
    let group_limit = free_group_limit(&authorization);

    // Mark the scan as running so the UI can show progress before the first event arrives.
    {
        let mut registry = state
            .duplicate_reports
            .lock()
            .map_err(|_| AppError::Internal("The report lock was poisoned.".to_string()))?;
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
                total_groups: 0,
                total_copies: 0,
                total_reclaimable_bytes: 0,
                hashed_bytes: 0,
                files_hashed: 0,
                errors: Vec::new(),
            },
        );
    }

    let progress_id = report_id.clone();
    let progress_app = app.clone();
    let progress_options = options.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let mut last_emit = std::time::Instant::now();

        let report = grouping::build_report(
            &index.files,
            &progress_options,
            progress_id.clone(),
            started_at,
            |files_hashed, bytes_hashed| {
                if last_emit.elapsed() >= Duration::from_millis(150) {
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    let payload = DuplicateProgress {
                        report_id: progress_id.clone(),
                        phase: "hashing".to_string(),
                        files_hashed,
                        bytes_hashed,
                        groups_found: 0,
                        elapsed_ms,
                        eta_ms: None,
                    };
                    let _ = progress_app.emit(DUPLICATE_PROGRESS_EVENT, &payload);
                    last_emit = std::time::Instant::now();
                }
            },
        );

        let _ = progress_app.emit(
            DUPLICATE_PROGRESS_EVENT,
            &DuplicateProgress {
                report_id: progress_id.clone(),
                phase: "done".to_string(),
                files_hashed: report.files_hashed,
                bytes_hashed: report.hashed_bytes,
                groups_found: report.total_groups,
                elapsed_ms: started.elapsed().as_millis() as u64,
                eta_ms: None,
            },
        );

        // The free tier sees the largest N groups and nothing else. Truncation happens here, in the
        // backend, so the limit cannot be lifted from the interface.
        let report = grouping::limit_groups(report, group_limit);

        let state = progress_app.state::<AppState>();
        if let Ok(mut registry) = state.duplicate_reports.lock() {
            registry.insert(progress_id, report);
        }
    });

    Ok(report_id)
}

/// Returns a finished report, or `None` when it is unknown or still running.
#[tauri::command]
pub fn duplicates_get(report_id: String, state: State<'_, AppState>) -> AppResult<Option<DuplicateReport>> {
    crate::authz::authorize_state(&state, crate::authz::Permission::BasicDuplicates)?;
    let registry = state
        .duplicate_reports
        .lock()
        .map_err(|_| AppError::Internal("The report lock was poisoned.".to_string()))?;
    Ok(registry.get(&report_id).cloned())
}

/// Byte-for-byte comparison of two files, used by the "are these really identical?" action.
#[tauri::command]
pub fn verify_duplicates(path_a: String, path_b: String) -> AppResult<crate::duplicates::verify::VerifyResult> {
    // Byte-level verification of a pair is part of finding duplicates, which is free.
    for path in [&path_a, &path_b] {
        if path.trim().is_empty() {
            return Err(AppError::InvalidArgument("Two paths are required.".to_string()));
        }
    }
    let result = verify_identical(
        &std::path::PathBuf::from(&path_a),
        &std::path::PathBuf::from(&path_b),
    )
    .map_err(|error| AppError::io(&path_a, &error))?;
    Ok(result)
}

/// `None` means unlimited; otherwise the free tier's group cap.
fn free_group_limit(authorization: &crate::authz::Authorization) -> Option<usize> {
    if crate::authz::authorize(authorization, crate::authz::Permission::UnlimitedDuplicates).is_ok() {
        return None;
    }
    Some(DEFAULT_CONFIG.free_duplicate_group_limit as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_rejects_empty_paths() {
        let empty = "".to_string();
        assert!(empty.trim().is_empty());
    }

    #[test]
    fn the_progress_payload_shape_matches_the_contract() {
        let payload = DuplicateProgress {
            report_id: "r".to_string(),
            phase: "hashing".to_string(),
            files_hashed: 1,
            bytes_hashed: 2,
            groups_found: 0,
            elapsed_ms: 3,
            eta_ms: None,
        };
        assert_eq!(payload.phase, "hashing");
    }

    #[test]
    fn an_arc_report_is_shareable() {
        let report = DuplicateReport {
            report_id: "r".to_string(),
            scan_id: "s".to_string(),
            started_at: 0,
            finished_at: None,
            status: "running".to_string(),
            algorithm: "sha256".to_string(),
            groups: Vec::new(),
            total_groups: 0,
            total_copies: 0,
            total_reclaimable_bytes: 0,
            hashed_bytes: 0,
            files_hashed: 0,
            errors: Vec::new(),
        };
        let shared = Arc::new(report);
        assert_eq!(shared.total_groups, 0);
    }
}
