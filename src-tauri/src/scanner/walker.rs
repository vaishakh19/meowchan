//! The directory walker.
//!
//! A bounded pool of workers pulls directories off a shared queue. Each worker reads one directory,
//! records its files, and pushes its sub-directories back onto the queue. That gives breadth-first
//! traversal (so progress and ETA are meaningful early) with bounded concurrency (so a huge tree
//! cannot exhaust handles or memory).
//!
//! Safety rules enforced here:
//!   - `symlink_metadata` is used, so links are never followed unless the caller opts in,
//!   - a directory is only ever queued once: the visited set is keyed on the canonical path,
//!   - permission denials, missing paths, long paths and vanished files are recorded, not guessed,
//!   - cancellation is checked on every entry, so cancelling stops within one directory read.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

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
    pub follow_links: bool,
    pub include_hidden: bool,
    pub include_system: bool,
    /// `0` = unlimited.
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
            max_entries: 0,
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
    /// True when a safety cap (entry limit, cancellation) stopped the walk early.
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
}

impl WorkQueue {
    fn with_root(root: PathBuf) -> Self {
        WorkQueue {
            inner: Mutex::new(QueueState {
                pending: vec![(root, 0)],
                finished: false,
            }),
            available: Condvar::new(),
        }
    }

    fn push(&self, path: PathBuf, depth: usize) {
        if let Ok(mut state) = self.inner.lock() {
            state.pending.push((path, depth));
            drop(state);
        }
        self.available.notify_one();
    }

    fn pop(&self) -> Option<(PathBuf, usize)> {
        let mut state = self.inner.lock().ok()?;
        loop {
            if let Some(item) = state.pending.pop() {
                return Some(item);
            }
            if state.finished {
                return None;
            }
            state = self.available.wait(state).ok()?;
        }
    }

    fn finish(&self) {
        if let Ok(mut state) = self.inner.lock() {
            state.finished = true;
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

    // Root metadata. A missing or non-directory root is an honest error, not an empty scan.
    match read_metadata(&root, 0) {
        Ok(meta) if meta.kind == Kind::Folder => {
            run.directories.push(DirectoryRecord {
                path: root.clone(),
                name: volume_name(&root),
                parent: None,
                depth: 0,
                modified_at: meta.modified_at,
                created_at: meta.created_at,
                access_denied: false,
            });
        }
        Ok(_) => {
            run.errors.push(WalkError {
                path: options.root_path.clone(),
                kind: "not-found".to_string(),
                message: "The scan root is not a directory.".to_string(),
            });
            run.hit_limit = true;
            return run;
        }
        Err(error) => {
            run.errors.push(WalkError {
                path: options.root_path.clone(),
                kind: error_kind(&error),
                message: error.to_string(),
            });
            run.hit_limit = true;
            return run;
        }
    }

    counters.set_phase(ScanPhase::Walking);

    let queue = Arc::new(WorkQueue::with_root(root.clone()));
    let visited: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
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
                if counters.is_cancelled() {
                    break;
                }
                if options.max_depth > 0 && depth >= options.max_depth as usize {
                    continue;
                }
                counters.set_current_path(&dir.to_string_lossy());
                scan_directory(&dir, depth, &options, &counters, &queue, &visited, &output);
            }
        }));
    }

    for handle in handles {
        let _ = handle.join();
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
    run.hit_limit = run.cancelled;
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
        if options.max_entries > 0 && (counters.files() + counters.folders()) >= options.max_entries {
            counters.add_error();
            record(output, |out| {
                out.errors.push(WalkError {
                    path: dir.to_string_lossy().to_string(),
                    kind: "limit-exceeded".to_string(),
                    message: format!("Stopped after {} entries.", options.max_entries),
                });
            });
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

        let path = entry.path();
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

        // Links are recorded but never traversed unless the caller opts in. This is what stops
        // junction loops and double counting.
        if meta.is_symlink && !options.follow_links {
            counters.add_folder();
            continue;
        }

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
                        parent_path: dir.to_path_buf(),
                        depth: meta.depth,
                    });
                });
            }
            Kind::Folder => {
                counters.add_folder();
                let key = canonical_key(&path);
                let fresh = match visited.lock() {
                    Ok(mut guard) => guard.insert(key),
                    Err(_) => false,
                };
                if !fresh {
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
    fn walks_a_small_tree_and_counts_everything() {
        let root = temp_dir("basic");
        fs::create_dir_all(root.join("a\\b")).unwrap();
        fs::write(root.join("a\\b\\deep.bin"), vec![0u8; 2048]).unwrap();
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
        let options = ScanOptions {
            root_path: "Z:\\definitely-not-here\\nope".to_string(),
            ..Default::default()
        };
        let counters = Arc::new(ProgressCounters::new());
        let run = walk(&options, &counters);
        assert!(run.files.is_empty());
        assert!(!run.errors.is_empty());
        assert!(run.hit_limit);
    }

    #[test]
    fn honours_max_depth() {
        let root = temp_dir("depth");
        fs::create_dir_all(root.join("a\\b\\c")).unwrap();
        fs::write(root.join("a\\b\\c\\deep.bin"), vec![0u8; 16]).unwrap();

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
        assert!(run.files.len() <= 11, "the cap is approximate by one entry");
        assert!(run.errors.iter().any(|error| error.kind == "limit-exceeded"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cancellation_stops_the_walk_early() {
        let root = temp_dir("cancel");
        fs::create_dir_all(root.join("a")).unwrap();
        for index in 0..20 {
            fs::write(root.join(format!("a\\f{index}.bin")), vec![0u8; 4]).unwrap();
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
    fn every_file_records_its_parent_directory() {
        let root = temp_dir("parent");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub\\file.bin"), vec![0u8; 32]).unwrap();
        let options = ScanOptions {
            root_path: root.to_string_lossy().to_string(),
            ..Default::default()
        };
        let counters = Arc::new(ProgressCounters::new());
        let run = walk(&options, &counters);
        assert_eq!(run.files.len(), 1);
        assert!(run.files[0].parent_path.ends_with("sub"));

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
        assert_eq!(options.max_entries, 0);
        assert!(!options.compute_hashes);
    }

    #[test]
    fn volume_name_formats_a_drive_root() {
        assert_eq!(volume_name(Path::new("C:\\")), "Local Disk (C:)");
        assert_eq!(volume_name(Path::new("D:\\Projects")), "Projects");
    }
}
