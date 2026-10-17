//! The directory walker.
//!
//! A bounded pool of workers pulls directories from a shared LIFO worklist. Each worker reads one
//! directory, records its files, and pushes sub-directories back onto the worklist. Concurrency is
//! bounded so a huge tree cannot exhaust handles or memory.
//!
//! Safety rules enforced here:
//!   - `symlink_metadata` and component checks are used; links and reparse points are never followed,
//!   - a directory is only ever queued once: the visited set is keyed on the canonical path,
//!   - permission denials, missing paths, long paths and vanished files are recorded, not guessed,
//!   - cancellation is checked on every entry, so cancelling stops within one directory read.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

pub const DEFAULT_MAX_ENTRIES: u64 = 1_000_000;

pub fn effective_max_entries(requested: u64) -> u64 {
    if requested == 0 { DEFAULT_MAX_ENTRIES } else { requested.min(DEFAULT_MAX_ENTRIES) }
}

use serde::{Deserialize, Serialize};

use crate::filesystem::{read_metadata, FileRecord, Kind};
use crate::platform;
use crate::scanner::progress::{ProgressCounters, ScanPhase};

/// Mirrors `ScanOptions` in `src/types/contracts.ts`. Tauri converts the camelCase keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScanOptions {
    pub root_path: String,
    /// `0` = unlimited.
    pub max_depth: u32,
    /// Retained for wire compatibility. Diskora never follows links or reparse points.
    pub follow_links: bool,
    pub include_hidden: bool,
    pub include_system: bool,
    /// Hard cap on all entries including the scan root. `0` requests the hard default (1,000,000).
    pub max_entries: u64,
    pub max_children_per_node: u32,
    pub compute_hashes: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            root_path: String::new(),
            max_depth: 0,
            follow_links: false,
            include_hidden: true,
            include_system: true,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_children_per_node: 100,
            compute_hashes: false,
        }
    }
}

/// One unreadable location. Mirrors `ScanErrorInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkError {
    pub path: String,
    pub kind: String,
    pub message: String,
}

/// One directory in the flat table.
#[derive(Debug, Clone)]
pub struct DirectoryRecord {
    pub path: PathBuf,
    pub name: String,
    /// Index into `ScanRun::directories` of the parent, or `None` for the root.
    pub parent: Option<usize>,
    pub depth: usize,
    pub modified_at: Option<i64>,
    pub created_at: Option<i64>,
    pub is_symlink: bool,
    pub is_junction: bool,
    /// The directory itself could not be read.
    pub access_denied: bool,
}

/// Everything a scan produced, before aggregation into a tree.
#[derive(Debug, Default)]
pub struct ScanRun {
    pub files: Vec<FileRecord>,
    /// Directory table. Index 0 is always the root.
    pub directories: Vec<DirectoryRecord>,
    pub errors: Vec<WalkError>,
    /// True when the hard entry limit stopped the walk early. Cancellation is reported separately.
    pub hit_limit: bool,
    pub cancelled: bool,
    pub elapsed_ms: u64,
}

/// Shared work queue. Workers block on the condvar while the queue is empty but work remains.
struct WorkQueue {
    inner: Mutex<QueueState>,
    available: Condvar,
}

#[derive(Default)]
struct QueueState {
    pending: Vec<(PathBuf, usize)>,
    finished: bool,
    active: usize,
}

impl WorkQueue {
    fn with_root(root: PathBuf) -> Self {
        WorkQueue {
            inner: Mutex::new(QueueState {
                pending: vec![(root, 0)],
                finished: false,
                active: 0,
            }),
            available: Condvar::new(),
        }
    }

    fn push(&self, path: PathBuf, depth: usize) {
        if let Ok(mut state) = self.inner.lock() {
            if state.finished { return; }
            state.pending.push((path, depth));
            drop(state);
        }
        self.available.notify_one();
    }

    fn pop(&self) -> Option<(PathBuf, usize)> {
        let mut state = self.inner.lock().ok()?;
        loop {
            if let Some(item) = state.pending.pop() {
                state.active += 1;
                return Some(item);
            }
            if state.finished {
                return None;
            }
            state = self.available.wait(state).ok()?;
        }
    }

    fn complete(&self) {
        if let Ok(mut state) = self.inner.lock() {
            state.active = state.active.saturating_sub(1);
            if state.active == 0 && state.pending.is_empty() {
                state.finished = true;
            }
        }
        self.available.notify_all();
    }

    fn finish(&self) {
        if let Ok(mut state) = self.inner.lock() {
            state.finished = true;
            state.pending.clear();
        }
        self.available.notify_all();
    }
}

/// Per-worker output, merged once the walk completes.
#[derive(Default)]
struct WorkOutput {
    files: Vec<FileRecord>,
    directories: Vec<DirectoryRecord>,
    errors: Vec<WalkError>,
    denied: Vec<PathBuf>,
}

/// Runs a full walk. Blocking: call it from a worker thread, never on the async runtime.
pub fn walk(options: &ScanOptions, counters: &Arc<ProgressCounters>) -> ScanRun {
    let started = Instant::now();
    let root = PathBuf::from(&options.root_path);
    let mut run = ScanRun::default();

    // Inspect path components from the volume root downward so a scan root nested below a
    // junction/symlink is rejected before any directory is enumerated.
    if let Err(error) = crate::filesystem::entries::ensure_no_reparse_components(&root) {
        run.errors.push(WalkError {
            path: options.root_path.clone(),
            kind: error_kind(&error),
            message: error.to_string(),
        });
        return run;
    }

    // Root metadata. A missing or non-directory root is an honest error, not an empty scan.
    match read_metadata(&root, 0) {
        Ok(meta) if meta.kind == Kind::Folder && !meta.is_reparse_point => {
            // The root itself counts against the same hard entry limit as its descendants.
            if !counters.reserve_entry(options.max_entries) {
                counters.mark_limit_reached();
                counters.add_error();
                run.errors.push(WalkError {
                    path: options.root_path.clone(),
                    kind: "limit-exceeded".to_string(),
                    message: format!("Stopped after at most {} entries.", effective_max_entries(options.max_entries)),
                });
                run.hit_limit = true;
                return run;
            }
            run.directories.push(DirectoryRecord {
                path: root.clone(),
                name: volume_name(&root),
                parent: None,
                depth: 0,
                modified_at: meta.modified_at,
                created_at: meta.created_at,
                is_symlink: meta.is_symlink,
                is_junction: meta.is_junction,
                access_denied: false,
            });
        }
        Ok(meta) => {
            run.errors.push(WalkError {
                path: options.root_path.clone(),
                kind: if meta.is_reparse_point { "other" } else { "not-found" }.to_string(),
                message: if meta.is_reparse_point {
                    "The scan root is a symbolic link or reparse point. Diskora will not follow it.".to_string()
                } else {
                    "The scan root is not a directory.".to_string()
                },
            });
            return run;
        }
        Err(error) => {
            run.errors.push(WalkError {
                path: options.root_path.clone(),
                kind: error_kind(&error),
                message: error.to_string(),
            });
            return run;
        }
    }

    counters.set_phase(ScanPhase::Walking);

    let queue = Arc::new(WorkQueue::with_root(root.clone()));
    let visited: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::from([std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone())])));
    let output: Arc<Mutex<WorkOutput>> = Arc::new(Mutex::new(WorkOutput::default()));

    let workers = worker_count();
    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let queue = Arc::clone(&queue);
        let visited = Arc::clone(&visited);
        let output = Arc::clone(&output);
        let counters = Arc::clone(counters);
        let options = options.clone();

        handles.push(std::thread::spawn(move || {
            while let Some((dir, depth)) = queue.pop() {
                if counters.is_cancelled() || counters.is_limit_reached() {
                    queue.finish();
                    queue.complete();
                    break;
                }
                if options.max_depth > 0 && depth >= options.max_depth as usize {
                    queue.complete();
                    continue;
                }
                counters.set_current_path(&dir.to_string_lossy());
                let worker = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    scan_directory(&dir, depth, &options, &counters, &queue, &visited, &output)
                }));
                if worker.is_err() {
                    counters.add_error();
                    record(&output, |out| {
                        out.errors.push(WalkError {
                            path: dir.to_string_lossy().to_string(),
                            kind: "worker-panic".to_string(),
                            message: "A scanner worker failed unexpectedly; remaining work was stopped.".to_string(),
                        });
                    });
                    queue.finish();
                }
                queue.complete();
            }
        }));
    }

    for handle in handles {
        if handle.join().is_err() {
            counters.add_error();
            record(&output, |out| {
                out.errors.push(WalkError {
                    path: options.root_path.clone(),
                    kind: "worker-panic".to_string(),
                    message: "A scanner worker terminated unexpectedly.".to_string(),
                });
            });
            queue.finish();
        }
    }
    queue.finish();

    counters.set_phase(ScanPhase::Aggregating);

    if let Ok(mut collected) = output.lock() {
        collected.files.sort_by(|a, b| a.path.cmp(&b.path));
        collected
            .directories
            .sort_by(|a, b| a.path.cmp(&b.path));
        run.files = std::mem::take(&mut collected.files);
        run.directories.extend(std::mem::take(&mut collected.directories));
        run.errors.extend(std::mem::take(&mut collected.errors));

        // Mark directories that turned out to be unreadable.
        let denied: Vec<PathBuf> = std::mem::take(&mut collected.denied);
        for path in denied {
            if let Some(directory) = run.directories.iter_mut().find(|item| item.path == path) {
                directory.access_denied = true;
            }
        }
    }

    run.cancelled = counters.is_cancelled();
    run.hit_limit = counters.is_limit_reached() || run.errors.iter().any(|error| error.kind == "limit-exceeded");
    run.elapsed_ms = started.elapsed().as_millis() as u64;
    run
}

/// Reads one directory level.
fn scan_directory(
    dir: &Path,
    depth: usize,
    options: &ScanOptions,
    counters: &ProgressCounters,
    queue: &WorkQueue,
    visited: &Mutex<HashSet<PathBuf>>,
    output: &Mutex<WorkOutput>,
) {
    if let Err(error) = crate::filesystem::entries::ensure_no_reparse_components(dir) {
        counters.add_error();
        record(output, |out| {
            out.errors.push(WalkError {
                path: dir.to_string_lossy().to_string(),
                kind: error_kind(&error),
                message: error.to_string(),
            });
            out.denied.push(dir.to_path_buf());
        });
        return;
    }
    let long_path = crate::filesystem::long_path::to_unc_long_path(dir);
    let entries = match std::fs::read_dir(&long_path) {
        Ok(entries) => entries,
        Err(error) => {
            counters.add_error();
            record(
                output,
                |out| {
                    out.errors.push(WalkError {
                        path: dir.to_string_lossy().to_string(),
                        kind: error_kind(&error),
                        message: error.to_string(),
                    });
                    out.denied.push(dir.to_path_buf());
                },
            );
            return;
        }
    };

    for entry in entries {
        if counters.is_cancelled() {
            return;
        }
        if counters.is_limit_reached() || !counters.reserve_entry(options.max_entries) {
            if counters.mark_limit_reached() {
                counters.add_error();
                record(output, |out| {
                    out.errors.push(WalkError {
                        path: dir.to_string_lossy().to_string(),
                        kind: "limit-exceeded".to_string(),
                        message: format!("Stopped after at most {} entries.", effective_max_entries(options.max_entries)),
                    });
                });
            }
            queue.finish();
            return;
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                counters.add_error();
                record(output, |out| {
                    out.errors.push(WalkError {
                        path: dir.to_string_lossy().to_string(),
                        kind: error_kind(&error),
                        message: error.to_string(),
                    });
                });
                continue;
            }
        };

        let path = crate::filesystem::long_path::normalize_long_path(&entry.path());
        let meta = match read_metadata(&path, depth + 1) {
            Ok(meta) => meta,
            Err(error) => {
                // A file that vanished between `read_dir` and `stat` is normal on a live system.
                counters.add_error();
                record(output, |out| {
                    out.errors.push(WalkError {
                        path: path.to_string_lossy().to_string(),
                        kind: error_kind(&error),
                        message: error.to_string(),
                    });
                });
                continue;
            }
        };

        if meta.is_hidden && !options.include_hidden {
            continue;
        }

        let is_link = meta.is_reparse_point;

        match meta.kind {
            Kind::File => {
                counters.add_file(meta.size);
                record(output, |out| {
                    out.files.push(FileRecord {
                        path: meta.path.clone(),
                        name: meta.name.clone(),
                        size: meta.size,
                        disk_size: meta.disk_size,
                        modified_at: meta.modified_at,
                        extension: meta.extension.clone(),
                        depth: meta.depth,
                        is_symlink: meta.is_symlink,
                        is_junction: meta.is_junction,
                        is_cloud_placeholder: meta.is_cloud_placeholder,
                        cloud_status: meta.cloud_status.clone(),
                    });
                });
            }
            Kind::Folder => {
                counters.add_folder();

                // Every reparse point is represented as a zero-size entry and never traversed.
                // The IPC option is intentionally ignored: following links is not supported.
                if is_link {
                    record(output, |out| {
                        out.directories.push(DirectoryRecord {
                            path: path.clone(),
                            name: meta.name.clone(),
                            parent: None,
                            depth: meta.depth,
                            modified_at: meta.modified_at,
                            created_at: meta.created_at,
                            is_symlink: meta.is_symlink,
                            is_junction: meta.is_junction,
                            access_denied: false,
                        });
                    });
                    continue;
                }

                let key = canonical_key(&path);
                let fresh = match visited.lock() {
                    Ok(mut guard) => guard.insert(key),
                    Err(_) => false,
                };
                if !fresh {
                    counters.add_error();
                    record(output, |out| {
                        out.errors.push(WalkError {
                            path: path.to_string_lossy().to_string(),
                            kind: "symlink-cycle".to_string(),
                            message: "This directory was already visited; skipping to avoid a loop."
                                .to_string(),
                        });
                    });
                    continue;
                }

                record(output, |out| {
                    out.directories.push(DirectoryRecord {
                        path: path.clone(),
                        name: meta.name.clone(),
                        parent: None,
                        depth: meta.depth,
                        modified_at: meta.modified_at,
                        created_at: meta.created_at,
                        is_symlink: meta.is_symlink,
                        is_junction: meta.is_junction,
                        access_denied: false,
                    });
                });
                queue.push(path, meta.depth);
            }
        }
    }
}

fn record<F>(output: &Mutex<WorkOutput>, apply: F)
where
    F: FnOnce(&mut WorkOutput),
{
    if let Ok(mut guard) = output.lock() {
        apply(&mut guard);
    }
}

fn error_kind(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => "access-denied".to_string(),
        std::io::ErrorKind::NotFound => "not-found".to_string(),
        _ => {
            let message = error.to_string().to_lowercase();
            if message.contains("too long") || message.contains("filename is too long") {
                "long-path".to_string()
            } else {
                "io".to_string()
            }
        }
    }
}

/// Canonicalisation is best-effort: when it fails the lexical path is used, which still catches the
/// common self-referential junction case.
fn canonical_key(path: &Path) -> PathBuf {
    let long = crate::filesystem::long_path::to_unc_long_path(path);
    std::fs::canonicalize(&long).unwrap_or(long)
}

/// Display name for a scan root: `C:\` becomes `Local Disk (C:)`, a folder becomes its name.
pub fn volume_name(root: &Path) -> String {
    let text = root.to_string_lossy().to_string();
    if text.len() == 3 && text.ends_with('\\') {
        return format!("Local Disk ({})", &text[..2]);
    }
    root.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or(text)
}

/// Worker pool size. Bounded so a pathological tree cannot exhaust handles.
pub fn worker_count() -> usize {
    let available = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4);
    available.clamp(2, 16)
}

/// Drive letter for a path, when the path is on a drive-rooted volume.
pub fn drive_id_for(path: &Path) -> Option<String> {
    platform::drive_letter(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("diskwise-walk-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn queue_finishes_when_the_last_in_flight_directory_completes() {
        let queue = WorkQueue::with_root(PathBuf::from("root"));
        assert!(queue.pop().is_some());
        queue.push(PathBuf::from("child"), 1);
        queue.complete();
        assert!(queue.pop().is_some());
        queue.complete();
        assert!(queue.pop().is_none());
    }

    #[test]
    fn walks_a_small_tree_and_counts_everything() {
        let root = temp_dir("basic");
        fs::create_dir_all(root.join("a").join("b")).unwrap();
        fs::write(root.join("a").join("b").join("deep.bin"), vec![0u8; 2048]).unwrap();
        fs::write(root.join("top.txt"), vec![0u8; 512]).unwrap();

        let options = ScanOptions {
            root_path: root.to_string_lossy().to_string(),
            ..Default::default()
        };
        let counters = Arc::new(ProgressCounters::new());
        let run = walk(&options, &counters);

        assert_eq!(run.files.len(), 2);
        assert_eq!(run.directories.len(), 3, "root + a + a\\b");
        assert_eq!(counters.files(), 2);
        assert_eq!(counters.folders(), 2);
        assert_eq!(counters.bytes(), 2048 + 512);
        assert!(run.errors.is_empty());
        assert!(!run.hit_limit);
        assert_eq!(run.directories[0].parent, None);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reports_a_missing_root_as_an_error() {
        let missing_root = std::env::temp_dir().join(format!("diskwise-missing-{}", std::process::id()));
        let options = ScanOptions {
            root_path: missing_root.to_string_lossy().to_string(),
            ..Default::default()
        };
        let counters = Arc::new(ProgressCounters::new());
        let run = walk(&options, &counters);
        assert!(run.files.is_empty());
        assert!(!run.errors.is_empty());
        assert!(!run.hit_limit);
    }

    #[test]
    fn honours_max_depth() {
        let root = temp_dir("depth");
        fs::create_dir_all(root.join("a").join("b").join("c")).unwrap();
        fs::write(root.join("a").join("b").join("c").join("deep.bin"), vec![0u8; 16]).unwrap();

        let options = ScanOptions {
            root_path: root.to_string_lossy().to_string(),
            max_depth: 2,
            ..Default::default()
        };
        let counters = Arc::new(ProgressCounters::new());
        let run = walk(&options, &counters);
        assert!(run.files.is_empty(), "depth 2 never enters `c`");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn honours_max_entries() {
        let root = temp_dir("limit");
        for index in 0..50 {
            fs::write(root.join(format!("file-{index}.bin")), vec![0u8; 8]).unwrap();
        }
        let options = ScanOptions {
            root_path: root.to_string_lossy().to_string(),
            max_entries: 10,
            ..Default::default()
        };
        let counters = Arc::new(ProgressCounters::new());
        let run = walk(&options, &counters);
        assert!(run.files.len() + run.directories.len() <= options.max_entries as usize);
        assert_eq!(counters.entries(), options.max_entries);
        assert!(run.hit_limit);
        assert!(run.errors.iter().any(|error| error.kind == "limit-exceeded"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parallel_workers_never_reserve_beyond_the_global_entry_limit() {
        let root = temp_dir("parallel-limit");
        for directory in 0..32 {
            let child = root.join(format!("dir-{directory}"));
            fs::create_dir_all(&child).unwrap();
            for file in 0..16 {
                fs::write(child.join(format!("file-{file}.bin")), vec![0u8; 8]).unwrap();
            }
        }
        let options = ScanOptions {
            root_path: root.to_string_lossy().to_string(),
            max_entries: 40,
            ..Default::default()
        };
        let counters = Arc::new(ProgressCounters::new());
        let run = walk(&options, &counters);
        assert_eq!(counters.entries(), options.max_entries);
        assert!(run.files.len() + run.directories.len() <= options.max_entries as usize);
        assert_eq!(run.errors.iter().filter(|error| error.kind == "limit-exceeded").count(), 1);
        assert!(run.hit_limit);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cancellation_stops_the_walk_early() {
        let root = temp_dir("cancel");
        fs::create_dir_all(root.join("a")).unwrap();
        for index in 0..20 {
            fs::write(root.join("a").join(format!("f{index}.bin")), vec![0u8; 4]).unwrap();
        }
        let counters = Arc::new(ProgressCounters::new());
        counters.cancel();
        let options = ScanOptions {
            root_path: root.to_string_lossy().to_string(),
            ..Default::default()
        };
        let run = walk(&options, &counters);
        assert!(run.files.len() < 20, "cancellation must stop the walk");
        assert!(run.cancelled);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_parent_is_derived_without_copying_a_second_path() {
        let root = temp_dir("parent");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("file.bin"), vec![0u8; 32]).unwrap();
        let options = ScanOptions {
            root_path: root.to_string_lossy().to_string(),
            ..Default::default()
        };
        let counters = Arc::new(ProgressCounters::new());
        let run = walk(&options, &counters);
        assert_eq!(run.files.len(), 1);
        let expected_parent = root.join("sub");
        assert_eq!(run.files[0].path.parent(), Some(expected_parent.as_path()));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn worker_pool_is_bounded() {
        assert!(worker_count() >= 2);
        assert!(worker_count() <= 16);
    }

    #[test]
    fn options_defaults_match_the_documented_contract() {
        let options = ScanOptions::default();
        assert_eq!(options.max_children_per_node, 100);
        assert!(!options.follow_links);
        assert!(options.include_hidden);
        assert!(options.include_system);
        assert_eq!(options.max_depth, 0);
        assert_eq!(options.max_entries, 1_000_000);
        assert!(!options.compute_hashes);
    }

    #[test]
    fn volume_name_formats_a_drive_root() {
        assert_eq!(volume_name(Path::new("C:\\")), "Local Disk (C:)");
        assert_eq!(volume_name(Path::new("D:\\Projects")), "Projects");
    }
}
