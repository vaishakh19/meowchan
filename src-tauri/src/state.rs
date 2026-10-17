use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::duplicates::grouping::{DuplicateOptions, DuplicateReport};
use crate::scanner::aggregate::{PruneOptions, ScanIndex};
use crate::scanner::progress::{ProgressCounters, ProgressSnapshot};
use crate::scanner::walker::ScanRun;
use crate::settings::AppSettings;
use crate::storage::db::Database;

/// One completed (or in-flight) scan.
#[derive(Clone)]
pub struct ScanEntry {
    pub scan_id: String,
    pub root_path: String,
    pub root_name: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    /// The scan tables are immutable; post-scan deletions mutate the index
    /// through this lock so live readers (tree pages, details, large files)
    /// stay consistent without cloning a potentially huge snapshot.
    pub index: Option<Arc<std::sync::RwLock<ScanIndex>>>,
    pub errors: Vec<crate::scanner::walker::WalkError>,
    pub pruned_nodes: usize,
    pub expected_bytes: Option<u64>,
    pub hit_limit: bool,
    pub duration_ms: Option<i64>,
}

/// Shared mutable state behind `tauri::State`.
pub struct AppState {
    pub database: Mutex<Option<Database>>,
    pub scans: Mutex<HashMap<String, Arc<ScanEntry>>>,
    /// Live counters for scans that are still running.
    pub counters: Mutex<HashMap<String, Arc<ProgressCounters>>>,
    /// Cancel flags, kept so a cancelled scan stays cancelled.
    pub cancelled: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub duplicate_reports: Mutex<HashMap<String, DuplicateReport>>,
    pub duplicate_options: Mutex<HashMap<String, DuplicateOptions>>,
    /// Cancellation flags for in-flight duplicate reports, keyed by report ID.
    pub duplicate_cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub settings: Mutex<Option<AppSettings>>,
    /// Default pruning options for the tree sent to the UI.
    pub prune: PruneOptions,

}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            database: Mutex::new(None),
            scans: Mutex::new(HashMap::new()),
            counters: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(HashMap::new()),
            duplicate_reports: Mutex::new(HashMap::new()),
            duplicate_options: Mutex::new(HashMap::new()),
            duplicate_cancellations: Mutex::new(HashMap::new()),
            settings: Mutex::new(None),
            prune: PruneOptions::default(),
        }
    }

    pub fn database(&self) -> Result<std::sync::MutexGuard<'_, Option<Database>>, String> {
        self.database
            .lock()
            .map_err(|_| "The database lock was poisoned.".to_string())
    }

    pub fn scans(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Arc<ScanEntry>>>, String> {
        self.scans
            .lock()
            .map_err(|_| "The scan registry lock was poisoned.".to_string())
    }


}

/// Snapshot of a scan's counters, ready to become a progress event.
pub fn snapshot(
    scan_id: &str,
    counters: &ProgressCounters,
    elapsed_ms: u64,
    expected: Option<u64>,
) -> ProgressSnapshot {
    let (percent, eta_ms) = crate::scanner::progress::estimate(counters.bytes(), expected, elapsed_ms);
    ProgressSnapshot {
        scan_id: scan_id.to_string(),
        status: match counters.phase() {
            crate::scanner::progress::ScanPhase::Done => {
                if counters.error_count() > 0 || counters.is_limit_reached() {
                    crate::scanner::progress::ScanStatus::Partial
                } else {
                    crate::scanner::progress::ScanStatus::Complete
                }
            }
            crate::scanner::progress::ScanPhase::Cancelled => crate::scanner::progress::ScanStatus::Cancelled,
            crate::scanner::progress::ScanPhase::Error => crate::scanner::progress::ScanStatus::Error,
            _ => crate::scanner::progress::ScanStatus::Running,
        },
        truncated: counters.is_limit_reached(),
        phase: counters.phase(),
        entries_scanned: counters.entries(),
        files_scanned: counters.files(),
        folders_scanned: counters.folders(),
        bytes_analyzed: counters.bytes(),
        errors: counters.error_count(),
        current_path: counters.current_path(),
        elapsed_ms,
        eta_ms,
        percent,
    }
}

/// Helper used by the scan command: builds an entry from a finished walk.
pub fn entry_from_run(
    scan_id: String,
    root_path: String,
    root_name: String,
    started_at: i64,
    run: ScanRun,
    expected_bytes: Option<u64>,
) -> ScanEntry {
    let finished_at = crate::time::now_millis();
    let duration_ms = finished_at.saturating_sub(started_at);
    let status = if run.cancelled {
        "cancelled".to_string()
    } else if run.directories.is_empty() {
        "error".to_string()
    } else if run.errors.is_empty() {
        "complete".to_string()
    } else {
        "partial".to_string()
    };
    let crate::scanner::walker::ScanRun {
        files,
        directories,
        errors,
        hit_limit,
        ..
    } = run;
    let index = if directories.is_empty() {
        None
    } else {
        Some(Arc::new(std::sync::RwLock::new(ScanIndex::from_parts(files, directories))))
    };
    ScanEntry {
        scan_id,
        root_path,
        root_name,
        started_at,
        finished_at: Some(finished_at),
        status,
        index,
        errors,
        pruned_nodes: 0,
        expected_bytes,
        hit_limit,
        duration_ms: Some(duration_ms),
    }
}
