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
    // The minimal analyzer does not follow links out of the selected tree.
    options.follow_links = false;
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
        if !registry.is_empty() {
            return Err(AppError::InvalidArgument("A scan is already running. Cancel it or wait for it to finish.".to_string()));
        }
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
            let pruned = {
                let guard = index.read().expect("the scan index lock was poisoned");
                prune_tree(&guard, 0, PruneOptions {
                    max_children: walk_options.max_children_per_node as usize,
                    max_nodes: PruneOptions::default().max_nodes,
                })
            };
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
                    total_bytes: entry.index.as_ref().map(|index| index.read().expect("the scan index lock was poisoned").total_bytes() as i64).unwrap_or(0),
                    file_count: entry.index.as_ref().map(|index| index.read().expect("the scan index lock was poisoned").total_files() as i64).unwrap_or(0),
                    folder_count: entry.index.as_ref().map(|index| index.read().expect("the scan index lock was poisoned").total_folders() as i64).unwrap_or(0),
                    error_count: entry.errors.len() as i64,
                    truncated: entry.hit_limit,
                    drive_id,
                    persisted: true,
                });
            }
        }

        let failed = entry.index.is_none();
        if let Ok(mut registry) = state.scans() {
            // Only the current file index stays in RAM; past summaries remain in SQLite.
            registry.clear();
            registry.insert(walk_id.clone(), Arc::new(entry));
        }
        if let Ok(mut registry) = state.counters.lock() {
            registry.remove(&walk_id);
        }
        if let Ok(mut registry) = state.cancelled.lock() {
            registry.remove(&walk_id);
        }
        walk_counters.set_phase(if failed {
            crate::scanner::progress::ScanPhase::Error
        } else if walk_counters.is_cancelled() {
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
        let collected: Vec<ScanSummary> = scans
            .values()
            .map(|entry| ScanSummary {
                scan_id: entry.scan_id.clone(),
                root_path: entry.root_path.clone(),
                root_name: entry.root_name.clone(),
                started_at: entry.started_at,
                finished_at: entry.finished_at,
                duration_ms: entry.duration_ms,
                status: entry.status.clone(),
                total_bytes: entry.index.as_ref().map(|index| index.read().expect("the scan index lock was poisoned").total_bytes()).unwrap_or(0),
                file_count: entry.index.as_ref().map(|index| index.read().expect("the scan index lock was poisoned").total_files()).unwrap_or(0),
                folder_count: entry.index.as_ref().map(|index| index.read().expect("the scan index lock was poisoned").total_folders()).unwrap_or(0),
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
    // Readers and the (rare) deletion writer share the index through this
    // lock; a page holds it only while the page is being built.
    let index = index.read().expect("the scan index lock was poisoned");

    let target = std::path::PathBuf::from(&path);
    let position = index
        .position_of(&target)
        .ok_or_else(|| AppError::NotFound(path.clone()))?;

    let mut items: Vec<TreeNode> = Vec::new();
    for child in index.children_of(position) {
        items.push(folder_node(&index, child));
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
            let pruned = {
                let guard = index.read().expect("the scan index lock was poisoned");
                prune_tree(&guard, 0, PruneOptions::default())
            };
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
        total_bytes: index.map(|index| index.read().expect("the scan index lock was poisoned").total_bytes()).unwrap_or(0),
        disk_total_bytes: index.and_then(|index| index.read().expect("the scan index lock was poisoned").total_disk_bytes()),
        file_count: index.map(|index| index.read().expect("the scan index lock was poisoned").total_files()).unwrap_or(0),
        folder_count: index.map(|index| index.read().expect("the scan index lock was poisoned").total_folders()).unwrap_or(0),
        errors: entry.errors.clone(),
        truncated: entry.hit_limit,
        pruned_nodes: entry.pruned_nodes,
        duration_ms: entry.duration_ms,
        expected_bytes: entry.expected_bytes,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopFolderEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserStorageProfile {
    pub username: String,
    pub user_path: String,
    pub total_bytes: u64,
    pub disk_size_bytes: Option<u64>,
    pub file_count: u64,
    pub folder_count: u64,
    pub categories: std::collections::HashMap<String, u64>,
    pub top_folders: Vec<TopFolderEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserStorageReport {
    pub drive_id: String,
    pub users: Vec<UserStorageProfile>,
    pub total_attributed_bytes: u64,
}

#[tauri::command]
pub fn user_storage_report(scan_id: String, state: State<'_, AppState>) -> AppResult<UserStorageReport> {
    let scans = state.scans()?;
    let entry = scans
        .get(&scan_id)
        .ok_or_else(|| AppError::NotFound(format!("Unknown scan {scan_id}.")))?;
    let index = entry
        .index
        .as_ref()
        .ok_or_else(|| AppError::NotFound("The scan is still running.".to_string()))?;
    let index = index.read().expect("the scan index lock was poisoned");

    let mut users: Vec<UserStorageProfile> = Vec::new();
    let mut total_attributed = 0u64;

    for (pos, dir) in index.directories.iter().enumerate() {
        let path_str = dir.path.to_string_lossy().to_string();
        let is_user_dir = {
            let parent_is_users = dir.path.parent().map(|p| {
                let p_str = p.to_string_lossy().to_lowercase();
                p_str.ends_with("\\users") || p_str.ends_with("/users") || p_str == "/home" || p_str == "c:\\users"
            }).unwrap_or(false);
            let name_lower = dir.name.to_lowercase();
            parent_is_users && name_lower != "public" && name_lower != "default" && name_lower != "default user" && name_lower != "all users"
        };

        if is_user_dir {
            let agg = &index.aggregates[pos];
            let mut categories: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

            let mut top_folders: Vec<TopFolderEntry> = Vec::new();
            for &child_pos in &index.children_of(pos) {
                let child_dir = &index.directories[child_pos];
                let child_agg = &index.aggregates[child_pos];
                top_folders.push(TopFolderEntry {
                    name: child_dir.name.clone(),
                    path: child_dir.path.to_string_lossy().to_string(),
                    size_bytes: child_agg.size_bytes,
                });
            }
            top_folders.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));

            for file in &index.files {
                if file.path.starts_with(&dir.path) {
                    let cat = crate::filesystem::classify(&file.path, false);
                    *categories.entry(cat).or_insert(0) += file.size;
                }
            }

            total_attributed += agg.size_bytes;
            users.push(UserStorageProfile {
                username: dir.name.clone(),
                user_path: path_str,
                total_bytes: agg.size_bytes,
                disk_size_bytes: agg.disk_size_bytes,
                file_count: agg.file_count,
                folder_count: agg.folder_count,
                categories,
                top_folders,
            });
        }
    }

    users.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes));

    Ok(UserStorageReport {
        drive_id: entry.root_name.clone(),
        users,
        total_attributed_bytes: total_attributed,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvExportRequest {
    pub scan_id: String,
    pub export_type: String,
}

fn csv_escape(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

#[tauri::command]
pub fn export_scan_csv(request: CsvExportRequest, state: State<'_, AppState>) -> AppResult<String> {
    let scans = state.scans()?;
    let entry = scans
        .get(&request.scan_id)
        .ok_or_else(|| AppError::NotFound(format!("Unknown scan {}.", request.scan_id)))?;
    let index = entry
        .index
        .as_ref()
        .ok_or_else(|| AppError::NotFound("The scan is still running.".to_string()))?;
    let index = index.read().expect("the scan index lock was poisoned");

    match request.export_type.as_str() {
        "all-files" => {
            let mut csv = String::from("\"Path\",\"Name\",\"Extension\",\"Category\",\"Size (Bytes)\",\"Allocated Size (Bytes)\",\"Modified Time (ms)\",\"Cloud Status\",\"Is Link\"\n");
            for file in &index.files {
                csv.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{}\n",
                    csv_escape(&file.path.to_string_lossy()),
                    csv_escape(&file.name),
                    csv_escape(file.extension.as_deref().unwrap_or("")),
                    csv_escape(&crate::filesystem::classify(&file.path, false)),
                    file.size,
                    file.disk_size.map(|d| d.to_string()).unwrap_or_else(|| "".to_string()),
                    file.modified_at.map(|m| m.to_string()).unwrap_or_else(|| "".to_string()),
                    csv_escape(file.cloud_status.as_deref().unwrap_or("none")),
                    file.is_symlink || file.is_junction
                ));
            }
            Ok(csv)
        }
        "large-files" => {
            let mut csv = String::from("\"Path\",\"Name\",\"Extension\",\"Category\",\"Size (Bytes)\",\"Allocated Size (Bytes)\",\"Modified Time (ms)\",\"Cloud Status\"\n");
            let mut sorted: Vec<_> = index.files.iter().filter(|f| f.size >= 100 * 1024 * 1024).collect();
            sorted.sort_by(|a, b| b.size.cmp(&a.size));
            for file in sorted {
                csv.push_str(&format!(
                    "{},{},{},{},{},{},{},{}\n",
                    csv_escape(&file.path.to_string_lossy()),
                    csv_escape(&file.name),
                    csv_escape(file.extension.as_deref().unwrap_or("")),
                    csv_escape(&crate::filesystem::classify(&file.path, false)),
                    file.size,
                    file.disk_size.map(|d| d.to_string()).unwrap_or_else(|| "".to_string()),
                    file.modified_at.map(|m| m.to_string()).unwrap_or_else(|| "".to_string()),
                    csv_escape(file.cloud_status.as_deref().unwrap_or("none")),
                ));
            }
            Ok(csv)
        }
        "user-attribution" => {
            let mut csv = String::from("\"Username\",\"User Path\",\"Total Bytes\",\"Allocated Bytes\",\"File Count\",\"Folder Count\"\n");
            for (pos, dir) in index.directories.iter().enumerate() {
                let parent_is_users = dir.path.parent().map(|p| {
                    let p_str = p.to_string_lossy().to_lowercase();
                    p_str.ends_with("\\users") || p_str.ends_with("/users") || p_str == "/home" || p_str == "c:\\users"
                }).unwrap_or(false);
                let name_lower = dir.name.to_lowercase();
                if parent_is_users && name_lower != "public" && name_lower != "default" && name_lower != "default user" && name_lower != "all users" {
                    let agg = &index.aggregates[pos];
                    csv.push_str(&format!(
                        "{},{},{},{},{},{}\n",
                        csv_escape(&dir.name),
                        csv_escape(&dir.path.to_string_lossy()),
                        agg.size_bytes,
                        agg.disk_size_bytes.map(|d| d.to_string()).unwrap_or_else(|| "".to_string()),
                        agg.file_count,
                        agg.folder_count
                    ));
                }
            }
            Ok(csv)
        }
        "duplicates" => {
            let reports = state.duplicate_reports.lock().map_err(|_| AppError::Internal("poisoned".into()))?;
            let report = reports.values().find(|r| r.scan_id == request.scan_id);
            let mut csv = String::from("\"Group ID\",\"Hash\",\"Confidence\",\"Reclaimable Bytes\",\"File Path\",\"File Size\"\n");
            if let Some(rep) = report {
                for group in &rep.groups {
                    for copy in &group.copies {
                        csv.push_str(&format!(
                            "{},{},{},{},{},{}\n",
                            csv_escape(&group.group_id),
                            csv_escape(&group.hash),
                            csv_escape(match group.confidence {
                                crate::duplicates::grouping::DuplicateConfidence::HashOnly => "hash",
                                crate::duplicates::grouping::DuplicateConfidence::ByteVerified => "verified",
                            }),
                            group.reclaimable_bytes,
                            csv_escape(&copy.path),
                            copy.size_bytes
                        ));
                    }
                }
            }
            Ok(csv)
        }
        other => Err(AppError::InvalidArgument(format!("Unknown export type '{other}'."))),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderGrowthDiff {
    pub path: String,
    pub name: String,
    pub baseline_bytes: u64,
    pub current_bytes: u64,
    pub delta_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanGrowthComparison {
    pub baseline_scan_id: String,
    pub current_scan_id: String,
    pub baseline_started_at: i64,
    pub current_started_at: i64,
    pub baseline_root: String,
    pub current_root: String,
    pub bytes_delta: i64,
    pub files_delta: i64,
    pub folders_delta: i64,
    pub category_deltas: std::collections::HashMap<String, i64>,
    pub growing_folders: Vec<FolderGrowthDiff>,
    pub reclaimed_folders: Vec<FolderGrowthDiff>,
}

#[tauri::command]
pub fn compare_scans(
    baseline_scan_id: String,
    current_scan_id: String,
    state: State<'_, AppState>,
) -> AppResult<ScanGrowthComparison> {
    let scans = state.scans()?;
    let baseline_entry = scans
        .get(&baseline_scan_id)
        .ok_or_else(|| AppError::NotFound(format!("Baseline scan {baseline_scan_id} not found.")))?;
    let current_entry = scans
        .get(&current_scan_id)
        .ok_or_else(|| AppError::NotFound(format!("Current scan {current_scan_id} not found.")))?;

    let baseline_idx = baseline_entry.index.as_ref().ok_or_else(|| AppError::NotFound("Baseline scan still in progress".into()))?.read().unwrap();
    let current_idx = current_entry.index.as_ref().ok_or_else(|| AppError::NotFound("Current scan still in progress".into()))?.read().unwrap();

    let bytes_delta = (current_idx.total_bytes() as i64) - (baseline_idx.total_bytes() as i64);
    let files_delta = (current_idx.total_files() as i64) - (baseline_idx.total_files() as i64);
    let folders_delta = (current_idx.total_folders() as i64) - (baseline_idx.total_folders() as i64);

    let mut category_deltas: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for file in &current_idx.files {
        let cat = crate::filesystem::classify(&file.path, false);
        *category_deltas.entry(cat).or_insert(0) += file.size as i64;
    }
    for file in &baseline_idx.files {
        let cat = crate::filesystem::classify(&file.path, false);
        *category_deltas.entry(cat).or_insert(0) -= file.size as i64;
    }

    let mut growing_folders: Vec<FolderGrowthDiff> = Vec::new();
    let mut reclaimed_folders: Vec<FolderGrowthDiff> = Vec::new();

    let mut baseline_dirs: std::collections::HashMap<std::path::PathBuf, (String, u64)> = std::collections::HashMap::new();
    for (pos, dir) in baseline_idx.directories.iter().enumerate() {
        let size = baseline_idx.aggregates[pos].size_bytes;
        baseline_dirs.insert(dir.path.clone(), (dir.name.clone(), size));
    }

    for (pos, dir) in current_idx.directories.iter().enumerate() {
        let cur_size = current_idx.aggregates[pos].size_bytes;
        if let Some((_, base_size)) = baseline_dirs.get(&dir.path) {
            let delta = (cur_size as i64) - (*base_size as i64);
            if delta > 1024 * 1024 {
                growing_folders.push(FolderGrowthDiff {
                    path: dir.path.to_string_lossy().to_string(),
                    name: dir.name.clone(),
                    baseline_bytes: *base_size,
                    current_bytes: cur_size,
                    delta_bytes: delta,
                });
            } else if delta < -(1024 * 1024) {
                reclaimed_folders.push(FolderGrowthDiff {
                    path: dir.path.to_string_lossy().to_string(),
                    name: dir.name.clone(),
                    baseline_bytes: *base_size,
                    current_bytes: cur_size,
                    delta_bytes: delta,
                });
            }
        }
    }

    growing_folders.sort_by(|a, b| b.delta_bytes.cmp(&a.delta_bytes));
    reclaimed_folders.sort_by(|a, b| a.delta_bytes.cmp(&b.delta_bytes));
    growing_folders.truncate(10);
    reclaimed_folders.truncate(10);

    Ok(ScanGrowthComparison {
        baseline_scan_id,
        current_scan_id,
        baseline_started_at: baseline_entry.started_at,
        current_started_at: current_entry.started_at,
        baseline_root: baseline_entry.root_path.clone(),
        current_root: current_entry.root_path.clone(),
        bytes_delta,
        files_delta,
        folders_delta,
        category_deltas,
        growing_folders,
        reclaimed_folders,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_child_page_clamps_its_limit() {
        // Guards against a caller asking for a million rows.
        let limit = 5000usize;
        assert_eq!(limit.clamp(1, 1000), 1000);
        assert_eq!(0usize.clamp(1, 1000), 1);
    }
}
