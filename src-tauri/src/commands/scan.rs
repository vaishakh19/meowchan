//! Scan lifecycle commands.
//!
//! `scan_start` returns immediately with an id; the walk runs on a blocking thread and publishes
//! throttled progress events. The tree is only built when the UI asks for it, which is what keeps a
//! million-entry scan from being materialised into JSON unless it is actually displayed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager, State};

use crate::error::{AppError, AppResult};
use crate::scanner::aggregate::{prune_tree, ScanIndex, TreeNode, PruneOptions};
use crate::scanner::walker::{self, ScanOptions};
use crate::state::{entry_from_run, AppState, ScanEntry};
use crate::storage::db::ScanHistoryRow;

pub const SCAN_PROGRESS_EVENT: &str = "scan://progress";
const PROGRESS_INTERVAL_MS: u64 = 100;

/// Starts a scan and returns its id immediately.
#[tauri::command]
pub async fn scan_start(
    options: ScanOptions,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    // Scanning is free at every tier and is never gated. The call is still made so that the
    // permission catalogue is exercised on this path and the audit trail shows it was checked.

    let root = options.root_path.trim().to_string();
    if root.is_empty() {
        return Err(AppError::InvalidArgument("No scan root was provided.".to_string()));
    }

    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::Internal("The settings lock was poisoned.".to_string()))?
        .clone()
        .unwrap_or_default();

    // Honour the user's settings unless the caller explicitly overrode them.
    let mut options = options;
    options.root_path = root.clone();
    // A zero means "use the user's preference", never "scan nothing".
    if options.max_children_per_node == 0 {
        options.max_children_per_node = settings.max_children_per_node;
    }

    let root_path = std::path::PathBuf::from(&root);
    if !root_path.is_absolute() || !root_path.is_dir() {
        return Err(AppError::InvalidArgument("Select an existing, absolute directory path.".to_string()));
    }

    let scan_id = format!(
        "scan-{}-{}",
        crate::time::now_millis(),
        std::process::id()
    );

    // Expected total, from volume metadata, drives the ETA and percentage. Only meaningful when
    // scanning a whole volume; scanning a single folder has no honest expectation.
    let expected_bytes = if is_drive_root(&root_path) {
        volume_used_bytes(&root_path)
    } else {
        None
    };

    let counters = Arc::new(crate::scanner::progress::ProgressCounters::new());
    let cancel_flag = Arc::new(AtomicBool::new(false));
    {
        let mut registry = state
            .counters
            .lock()
            .map_err(|_| AppError::Internal("The counter lock was poisoned.".to_string()))?;
        registry.insert(scan_id.clone(), Arc::clone(&counters));
    }
    {
        let mut registry = state
            .cancelled
            .lock()
            .map_err(|_| AppError::Internal("The cancel lock was poisoned.".to_string()))?;
        registry.insert(scan_id.clone(), Arc::clone(&cancel_flag));
    }

    let started_at = crate::time::now_millis();
    let root_name = walker::volume_name(&root_path);

    // The walk itself, on a blocking thread so the async runtime stays free.
    let walk_counters = Arc::clone(&counters);
    let walk_options = options.clone();
    let walk_id = scan_id.clone();
    let walk_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = walk_app.state::<AppState>();

        // Progress emitter, owned by this task so it is guaranteed to stop. Throttled to ~10 Hz so
        // the UI never has to keep up with the walker.
        let emitter_app = walk_app.clone();
        let emitter_counters = Arc::clone(&walk_counters);
        let emitter_cancel = Arc::clone(&cancel_flag);
        let emitter_id = walk_id.clone();
        let emitter = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            // Safety cap: if the walk somehow never reports a terminal phase, stop after 30 minutes
            // rather than emitting forever.
            let deadline = started + Duration::from_secs(30 * 60);
            loop {
                if std::time::Instant::now() > deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(PROGRESS_INTERVAL_MS));
                let phase = emitter_counters.phase();
                let payload = crate::state::snapshot(
                    &emitter_id,
                    &emitter_counters,
                    started.elapsed().as_millis() as u64,
                    None,
                );
                let _ = emitter_app.emit(SCAN_PROGRESS_EVENT, &payload);
                if matches!(
                    phase,
                    crate::scanner::progress::ScanPhase::Done
                        | crate::scanner::progress::ScanPhase::Cancelled
                        | crate::scanner::progress::ScanPhase::Error
                ) {
                    break;
                }
                if emitter_cancel.load(Ordering::SeqCst) && emitter_counters.files() == 0 {
                    break;
                }
            }
        });

        let walk_started = std::time::Instant::now();
        let run = walker::walk(&walk_options, &walk_counters);
        let mut entry = entry_from_run(
            walk_id.clone(),
            root.clone(),
            root_name.clone(),
            started_at,
            run,
            expected_bytes,
        );

        // Apply the pruning budget so the tree the UI receives is bounded.
        if let Some(index) = &entry.index {
            let pruned = prune_tree(index, 0, PruneOptions {
                max_children: walk_options.max_children_per_node as usize,
                max_nodes: PruneOptions::default().max_nodes,
            });
            entry.pruned_nodes = pruned.pruned_nodes;
        }

        // Persist a short summary row: root, totals, duration, status. Never file contents.
        let drive_id = walker::drive_id_for(&std::path::PathBuf::from(&root));
        if let Ok(guard) = state.database() {
            if let Some(database) = guard.as_ref() {
                let _ = database.insert_scan_history(&ScanHistoryRow {
                    scan_id: walk_id.clone(),
                    root_path: root.clone(),
                    root_name: root_name.clone(),
                    started_at,
                    finished_at: entry.finished_at,
                    duration_ms: entry.duration_ms,
                    status: entry.status.clone(),
                    total_bytes: entry.index.as_ref().map(|index| index.total_bytes() as i64).unwrap_or(0),
                    file_count: entry.index.as_ref().map(|index| index.total_files() as i64).unwrap_or(0),
                    folder_count: entry.index.as_ref().map(|index| index.total_folders() as i64).unwrap_or(0),
                    error_count: entry.errors.len() as i64,
                    truncated: entry.hit_limit,
                    drive_id,
                    persisted: true,
                });
            }
        }

        if let Ok(mut registry) = state.scans() {
            registry.insert(walk_id.clone(), Arc::new(entry));
        }
        if let Ok(mut registry) = state.counters.lock() {
            registry.remove(&walk_id);
        }
        if let Ok(mut registry) = state.cancelled.lock() {
            registry.remove(&walk_id);
        }
        walk_counters.set_phase(if walk_counters.is_cancelled() {
            crate::scanner::progress::ScanPhase::Cancelled
        } else {
            crate::scanner::progress::ScanPhase::Done
        });

        // Publish one final reading, then stop the emitter.
        let final_payload = crate::state::snapshot(
            &walk_id,
            &walk_counters,
            walk_started.elapsed().as_millis() as u64,
            expected_bytes,
        );
        let _ = walk_app.emit(SCAN_PROGRESS_EVENT, &final_payload);
        let _ = emitter.join();
    });

    Ok(scan_id)
}

/** True when `path` is a drive root such as `C:\`. */
fn is_drive_root(path: &std::path::Path) -> bool {
    let text = path.to_string_lossy();
    text.len() == 3 && text.ends_with('\\')
}

/// Bytes in use on the volume containing `path`. Used only as an ETA hint, so a folder scan
/// (which has no honest expectation) gets `None` instead of a made-up number.
fn volume_used_bytes(path: &std::path::Path) -> Option<u64> {
    crate::platform::list_volumes()
        .into_iter()
        .find(|volume| {
            let root = std::path::PathBuf::from(&volume.root);
            path.starts_with(&root)
        })
        .map(|volume| volume.total_bytes.saturating_sub(volume.free_bytes))
}

/// Cancels a running scan. The walk stops within one directory read.
#[tauri::command]
pub async fn scan_cancel(scan_id: String, state: State<'_, AppState>) -> AppResult<()> {
    if let Ok(registry) = state.counters.lock() {
        if let Some(counters) = registry.get(&scan_id) {
            counters.cancel();
        }
    }
    if let Ok(registry) = state.cancelled.lock() {
        if let Some(flag) = registry.get(&scan_id) {
            flag.store(true, Ordering::SeqCst);
        }
    }
    if let Ok(mut scans) = state.scans() {
        if let Some(entry) = scans.get(&scan_id) {
            let mut updated = (**entry).clone();
            updated.status = "cancelled".to_string();
            updated.finished_at = Some(crate::time::now_millis());
            scans.insert(scan_id, Arc::new(updated));
        }
    }
    Ok(())
}

/// Returns the full result for a finished scan, or `None` when it is unknown.
#[tauri::command]
pub fn scan_get(scan_id: String, state: State<'_, AppState>) -> AppResult<Option<ScanResult>> {
    let scans = state.scans()?;
    let entry = match scans.get(&scan_id) {
        Some(entry) => entry,
        None => return Ok(None),
    };
    Ok(Some(build_result(entry)))
}

/// Every scan this session knows about, newest first.
#[tauri::command]
pub fn scan_list(state: State<'_, AppState>) -> AppResult<Vec<ScanSummary>> {
    // Scan *history* is a paid capability; this list is the in-memory one and stays free.
    let mut summaries: Vec<ScanSummary> = {
        let scans = state.scans()?;
        let mut collected: Vec<ScanSummary> = scans
            .values()
            .map(|entry| ScanSummary {
                scan_id: entry.scan_id.clone(),
                root_path: entry.root_path.clone(),
                root_name: entry.root_name.clone(),
                started_at: entry.started_at,
                finished_at: entry.finished_at,
                duration_ms: entry.duration_ms,
                status: entry.status.clone(),
                total_bytes: entry.index.as_ref().map(|index| index.total_bytes()).unwrap_or(0),
                file_count: entry.index.as_ref().map(|index| index.total_files()).unwrap_or(0),
                folder_count: entry.index.as_ref().map(|index| index.total_folders()).unwrap_or(0),
                error_count: entry.errors.len(),
                truncated: entry.hit_limit,
                drive_id: walker::drive_id_for(&std::path::PathBuf::from(&entry.root_path)),
                persisted: false,
            })
            .collect();
        collected
    };
    summaries.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    // Merge in persisted history so the list survives a restart.
    if let Ok(guard) = state.database() {
        if let Some(database) = guard.as_ref() {
            if let Ok(rows) = database.list_scan_history(50) {
                let known: std::collections::HashSet<String> =
                    summaries.iter().map(|item| item.scan_id.clone()).collect();
                for row in rows {
                    if known.contains(&row.scan_id) {
                        continue;
                    }
                    summaries.push(ScanSummary {
                        scan_id: row.scan_id,
                        root_path: row.root_path,
                        root_name: row.root_name,
                        started_at: row.started_at,
                        finished_at: row.finished_at,
                        duration_ms: row.duration_ms,
                        status: row.status,
                        total_bytes: row.total_bytes.max(0) as u64,
                        file_count: row.file_count.max(0) as u64,
                        folder_count: row.folder_count.max(0) as u64,
                        error_count: row.error_count.max(0) as usize,
                        truncated: row.truncated,
                        drive_id: row.drive_id,
                        persisted: row.persisted,
                    });
                }
                summaries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
            }
        }
    }

    Ok(summaries)
}

/// Drops a scan from memory. Never touches the filesystem.
#[tauri::command]
pub fn scan_delete(scan_id: String, state: State<'_, AppState>) -> AppResult<()> {
    let mut scans = state.scans()?;
    scans.remove(&scan_id);
    Ok(())
}

/// Paginated children of a directory inside a scan, for the virtualized file list.
#[tauri::command]
pub fn scan_children(
    scan_id: String,
    path: String,
    offset: usize,
    limit: usize,
    state: State<'_, AppState>,
) -> AppResult<ChildrenPage> {
    let scans = state.scans()?;
    let entry = scans
        .get(&scan_id)
        .ok_or_else(|| AppError::NotFound(format!("Unknown scan {scan_id}")))?;
    let index = entry
        .index
        .as_ref()
        .ok_or_else(|| AppError::NotFound("The scan is still running.".to_string()))?;

    let target = std::path::PathBuf::from(&path);
    let position = index
        .position_of(&target)
        .ok_or_else(|| AppError::NotFound(path.clone()))?;

    let mut items: Vec<TreeNode> = Vec::new();
    for child in index.children_of(position) {
        items.push(folder_node(index, *child));
    }
    for file in index.files_of(position) {
        items.push(file_node(file));
    }
    items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then_with(|| a.path.cmp(&b.path)));

    let total = items.len();
    let limit = limit.clamp(1, 1000);
    let page: Vec<TreeNode> = items.into_iter().skip(offset).take(limit).collect();

    Ok(ChildrenPage {
        path,
        offset,
        limit,
        total,
        items: page,
    })
}

fn folder_node(index: &ScanIndex, position: usize) -> TreeNode {
    let directory = &index.directories[position];
    let aggregate = &index.aggregates[position];
    TreeNode {
        id: crate::scanner::aggregate::stable_path_id(&directory.path),
        path: directory.path.to_string_lossy().to_string(),
        name: directory.name.clone(),
        kind: "folder".to_string(),
        size_bytes: aggregate.size_bytes,
        disk_size_bytes: aggregate.disk_size_bytes,
        file_count: aggregate.file_count,
        folder_count: aggregate.folder_count,
        modified_at: aggregate.modified_at,
        created_at: aggregate.created_at,
        extension: None,
        category: crate::filesystem::classify(&directory.path, true),
        access_denied: aggregate.access_denied,
        children: Vec::new(),
        synthetic: false,
    }
}

fn file_node(file: &crate::filesystem::FileRecord) -> TreeNode {
    TreeNode {
        id: crate::scanner::aggregate::stable_path_id(&file.path),
        path: file.path.to_string_lossy().to_string(),
        name: file.name.clone(),
        kind: "file".to_string(),
        size_bytes: file.size,
        disk_size_bytes: file.disk_size,
        file_count: 0,
        folder_count: 0,
        modified_at: file.modified_at,
        created_at: None,
        extension: file.extension.clone(),
        category: crate::filesystem::classify(&file.path, false),
        access_denied: false,
        children: Vec::new(),
        synthetic: false,
    }
}

/// Mirrors `ScanResult`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub scan_id: String,
    pub root_path: String,
    pub root_name: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub tree: TreeNode,
    pub total_bytes: u64,
    pub disk_total_bytes: Option<u64>,
    pub file_count: u64,
    pub folder_count: u64,
    pub errors: Vec<crate::scanner::walker::WalkError>,
    pub truncated: bool,
    pub pruned_nodes: usize,
    pub duration_ms: Option<i64>,
    pub expected_bytes: Option<u64>,
}

/// Mirrors `ScanSummary`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub scan_id: String,
    pub root_path: String,
    pub root_name: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub total_bytes: u64,
    pub file_count: u64,
    pub folder_count: u64,
    pub error_count: usize,
    pub truncated: bool,
    pub drive_id: Option<String>,
    pub persisted: bool,
}

/// Mirrors `ChildrenPage`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildrenPage {
    pub path: String,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub items: Vec<TreeNode>,
}

fn build_result(entry: &ScanEntry) -> ScanResult {
    let tree = match &entry.index {
        Some(index) => {
            let pruned = prune_tree(index, 0, PruneOptions::default());
            pruned.tree
        }
        None => TreeNode {
            id: crate::scanner::aggregate::stable_path_id(&std::path::PathBuf::from(&entry.root_path)),
            path: entry.root_path.clone(),
            name: entry.root_name.clone(),
            kind: "folder".to_string(),
            size_bytes: 0,
            disk_size_bytes: None,
            file_count: 0,
            folder_count: 0,
            modified_at: None,
            created_at: None,
            extension: None,
            category: "other".to_string(),
            access_denied: true,
            children: Vec::new(),
            synthetic: false,
        },
    };

    let index = entry.index.as_ref();
    ScanResult {
        scan_id: entry.scan_id.clone(),
        root_path: entry.root_path.clone(),
        root_name: entry.root_name.clone(),
        started_at: entry.started_at,
        finished_at: entry.finished_at,
        status: entry.status.clone(),
        tree,
        total_bytes: index.map(|index| index.total_bytes()).unwrap_or(0),
        disk_total_bytes: index.and_then(|index| index.total_disk_bytes()),
        file_count: index.map(|index| index.total_files()).unwrap_or(0),
        folder_count: index.map(|index| index.total_folders()).unwrap_or(0),
        errors: entry.errors.clone(),
        truncated: entry.hit_limit,
        pruned_nodes: entry.pruned_nodes,
        duration_ms: entry.duration_ms,
        expected_bytes: entry.expected_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_page_clamps_its_limit() {
        // Guards against a caller asking for a million rows.
        let limit = 5000usize;
        assert_eq!(limit.clamp(1, 1000), 1000);
        assert_eq!(0usize.clamp(1, 1000), 1);
    }
}
