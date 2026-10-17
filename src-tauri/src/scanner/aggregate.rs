//! Aggregation and tree pruning.
//!
//! The walker produces a flat list of files and a flat list of directories. Aggregation rolls those
//! up into per-directory totals, then the pruner turns them into the bounded tree the UI receives.
//!
//! The pruner is the Rust half of the contract documented in `src/lib/tree.ts` and must stay
//! behaviourally identical to it: the same `maxChildrenPerNode`, the same synthetic
//! `Other (n items)` node, the same node budget. Sizes stay truthful at every level; only the
//! *granularity* is reduced.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::filesystem::classify;
use crate::scanner::walker::{DirectoryRecord, ScanRun};
use crate::filesystem::FileRecord;

/// Children kept per folder in the tree sent to the UI.
pub const DEFAULT_MAX_CHILDREN: usize = 100;
/// Hard cap on the number of nodes in the returned tree.
pub const DEFAULT_MAX_NODES: usize = 150_000;
/// Maximum depth sent to the UI; caps recursive JSON/UI operations on hostile directory chains.
pub const MAX_TREE_DEPTH: usize = 256;
/// Path suffix of the synthetic aggregate node. The backend refuses filesystem ops on these.
pub const OTHER_NODE_SUFFIX: &str = "\\[Other]";

/// Per-directory totals.
#[derive(Debug, Default, Clone)]
pub struct DirectoryAggregate {
    pub size_bytes: u64,
    pub disk_size_bytes: Option<u64>,
    pub file_count: u64,
    pub folder_count: u64,
    pub modified_at: Option<i64>,
    pub created_at: Option<i64>,
    pub access_denied: bool,
}

/// The complete in-memory index of one scan.
#[derive(Debug, Default)]
pub struct ScanIndex {
    /// Directory table; index 0 is the root.
    pub directories: Vec<DirectoryRecord>,
    pub aggregates: Vec<DirectoryAggregate>,
    pub files: Vec<FileRecord>,
    /// `display path -> index into directories`.
    index: HashMap<PathBuf, usize>,
    /// Adjacency list: `children[i]` are the directory indices directly below `i`, biggest first.
    /// Built once so tree construction stays linear in the number of directories.
    children: Vec<Vec<usize>>,
    /// Directory positions for each file parent, biggest first. Each directory is represented by
    /// its vector position, avoiding a second copy of every path in a million-file index.
    files_by_parent: Vec<Vec<usize>>,
    /// Directory positions removed after the scan (Recycle Bin deletion).
    /// The tables above are never mutated; removals are an overlay so the
    /// shared `Arc<RwLock<ScanIndex>>` stays cheap to hold while a deletion runs.
    removed_directories: std::collections::HashSet<usize>,
    /// File indices removed after the scan.
    removed_files: std::collections::HashSet<usize>,
    /// Bounded cache of sorted/filtering indexes for paged directory listings.
    child_order_cache: Mutex<HashMap<ChildOrderKey, Vec<u64>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntrySort {
    Name,
    Size,
    Count,
    Modified,
}

#[derive(Debug, Clone, Copy)]
pub enum IndexedEntry {
    Directory(usize),
    File(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ChildOrderKey {
    position: usize,
    sort: EntrySort,
    descending: bool,
    search: String,
}

impl ScanIndex {
    /// Builds the index from a walk result. The command path uses `from_parts` to transfer ownership
    /// and avoid duplicating the full index; this clone-based helper is useful for tests/callers that
    /// still need their `ScanRun` afterward.
    pub fn from_run(run: &ScanRun) -> Self {
        Self::from_parts(run.files.clone(), run.directories.clone())
    }

    /// Builds the index by taking ownership of the walker's records.
    pub fn from_parts(mut files: Vec<FileRecord>, mut directories: Vec<DirectoryRecord>) -> Self {
        directories.sort_by(|a, b| a.path.cmp(&b.path));
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let mut index: HashMap<PathBuf, usize> = HashMap::with_capacity(directories.len());
        for (position, directory) in directories.iter().enumerate() {
            index.insert(directory.path.clone(), position);
        }

        // Resolve parent indices now that every directory is in the table.
        for directory in directories.iter_mut() {
            directory.parent = directory.path.parent().and_then(|parent| index.get(parent).copied());
        }

        let mut aggregates = vec![DirectoryAggregate::default(); directories.len()];
        // `true` means the aggregate includes an exact allocated-size value for every file below
        // that directory. One unknown file makes the entire directory's value unknown.
        let mut disk_size_complete = vec![true; directories.len()];

        // Own timestamps per directory and start exact disk sums at zero. Unknown file allocation
        // values permanently mark the directory and its ancestors as unknown below.
        for (position, directory) in directories.iter().enumerate() {
            aggregates[position].modified_at = directory.modified_at;
            aggregates[position].created_at = directory.created_at;
            aggregates[position].access_denied = directory.access_denied;
            aggregates[position].disk_size_bytes = Some(0);
        }

        let mut files_by_parent: Vec<Vec<usize>> = vec![Vec::new(); directories.len()];
        for (position, file) in files.iter().enumerate() {
            let parent = file.path.parent().and_then(|parent| index.get(parent).copied());
            if let Some(directory) = parent {
                files_by_parent[directory].push(position);
                let aggregate = &mut aggregates[directory];
                aggregate.size_bytes = aggregate.size_bytes.saturating_add(file.size);
                aggregate.file_count = aggregate.file_count.saturating_add(1);
                match (disk_size_complete[directory], file.disk_size) {
                    (true, Some(disk_size)) => {
                        aggregate.disk_size_bytes = Some(
                            aggregate.disk_size_bytes.unwrap_or(0).saturating_add(disk_size),
                        );
                    }
                    (true, None) => {
                        disk_size_complete[directory] = false;
                        aggregate.disk_size_bytes = None;
                    }
                    (false, _) => {}
                }
            }
        }
        for entries in files_by_parent.iter_mut() {
            entries.sort_by(|a, b| {
                files[*b]
                    .size
                    .cmp(&files[*a].size)
                    .then_with(|| files[*a].path.cmp(&files[*b].path))
            });
        }

        // Directories roll up from the deepest to the shallowest, so one pass suffices.
        let mut order: Vec<usize> = (0..directories.len()).collect();
        order.sort_by(|a, b| directories[*b].depth.cmp(&directories[*a].depth));
        for position in order {
            let (size, files, folders, disk, disk_complete) = {
                let aggregate = &aggregates[position];
                (
                    aggregate.size_bytes,
                    aggregate.file_count,
                    aggregate.folder_count,
                    aggregate.disk_size_bytes,
                    disk_size_complete[position],
                )
            };
            if let Some(parent) = directories[position].parent {
                let parent_aggregate = &mut aggregates[parent];
                parent_aggregate.size_bytes = parent_aggregate.size_bytes.saturating_add(size);
                parent_aggregate.file_count = parent_aggregate.file_count.saturating_add(files);
                parent_aggregate.folder_count = parent_aggregate
                    .folder_count
                    .saturating_add(folders.saturating_add(1));
                if disk_size_complete[parent] && disk_complete {
                    parent_aggregate.disk_size_bytes = Some(
                        parent_aggregate.disk_size_bytes.unwrap_or(0)
                            .saturating_add(disk.unwrap_or(0)),
                    );
                } else {
                    disk_size_complete[parent] = false;
                    parent_aggregate.disk_size_bytes = None;
                }
            }
        }

        // Adjacency list, each child list sorted by size so the pruner can take the top-N cheaply.
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); directories.len()];
        for (position, directory) in directories.iter().enumerate() {
            if let Some(parent) = directory.parent {
                children[parent].push(position);
            }
        }
        for list in children.iter_mut() {
            list.sort_by(|a, b| {
                aggregates[*b]
                    .size_bytes
                    .cmp(&aggregates[*a].size_bytes)
                    .then_with(|| directories[*a].path.cmp(&directories[*b].path))
            });
        }

        ScanIndex {
            directories,
            aggregates,
            files,
            index,
            children,
            files_by_parent,
            removed_directories: std::collections::HashSet::new(),
            removed_files: std::collections::HashSet::new(),
            child_order_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn directory_count(&self) -> usize {
        self.directories.len().saturating_sub(self.removed_directories.len())
    }

    pub fn file_count(&self) -> usize {
        self.files.len().saturating_sub(self.removed_files.len())
    }

    pub fn total_bytes(&self) -> u64 {
        self.aggregates.first().map(|aggregate| aggregate.size_bytes).unwrap_or(0)
    }

    pub fn total_disk_bytes(&self) -> Option<u64> {
        self.aggregates.first().and_then(|aggregate| aggregate.disk_size_bytes)
    }

    pub fn total_files(&self) -> u64 {
        self.aggregates.first().map(|aggregate| aggregate.file_count).unwrap_or(0)
    }

    pub fn total_folders(&self) -> u64 {
        self.aggregates.first().map(|aggregate| aggregate.folder_count).unwrap_or(0)
    }

    /// The real path behind an identity token's id (see `disclosure::path_for_ref`).
    ///
    /// A linear scan over the live (not removed) entries. It runs only when the user navigates
    /// with a token, never while a scan is being built, so it stays off the hot path.
    pub fn path_for_ref(&self, id: &str) -> Option<PathBuf> {
        let directory = self
            .directories
            .iter()
            .enumerate()
            .find(|(position, directory)| {
                !self.removed_directories.contains(position) && stable_path_id(&directory.path) == id
            })
            .map(|(_, directory)| directory.path.clone());
        if directory.is_some() {
            return directory;
        }
        self.files
            .iter()
            .enumerate()
            .find(|(position, file)| !self.removed_files.contains(position) && stable_path_id(&file.path) == id)
            .map(|(_, file)| file.path.clone())
    }

    pub fn position_of(&self, path: &Path) -> Option<usize> {
        let position = self.index.get(path).copied()?;
        if self.removed_directories.contains(&position) {
            return None;
        }
        Some(position)
    }

    /// Direct children of a directory, biggest first, minus anything deleted since the scan.
    pub fn children_of(&self, position: usize) -> Vec<usize> {
        self.children
            .get(position)
            .map(|list| {
                list.iter()
                    .copied()
                    .filter(|child| !self.removed_directories.contains(child))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Files whose parent is `position`, biggest first, minus anything deleted since the scan.
    pub fn files_of(&self, position: usize) -> Vec<&FileRecord> {
        self.files_by_parent
            .get(position)
            .map(|entries| {
                entries
                    .iter()
                    .copied()
                    .filter(|file| !self.removed_files.contains(file))
                    .map(|file| &self.files[file])
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Iterates files that remain present in the post-deletion overlay.
    pub fn active_files(&self) -> impl Iterator<Item = &FileRecord> + '_ {
        self.files
            .iter()
            .enumerate()
            .filter_map(|(position, file)| (!self.removed_files.contains(&position)).then_some(file))
    }

    /// Returns a bounded page of direct children in a stable, requested order. The sorted index is
    /// cached for up to eight query variants so scrolling does not repeatedly sort a million rows.
    pub fn children_page(
        &self,
        position: usize,
        offset: usize,
        limit: usize,
        sort: EntrySort,
        descending: bool,
        search: &str,
    ) -> Option<(usize, Vec<IndexedEntry>)> {
        if position >= self.directories.len() {
            return None;
        }
        let normalized_search = search.trim().to_lowercase();
        let key = ChildOrderKey {
            position,
            sort,
            descending,
            search: normalized_search.clone(),
        };
        let mut cache = self.child_order_cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !cache.contains_key(&key) {
            let order = self.build_child_order(position, sort, descending, &normalized_search);
            if cache.len() >= 8 {
                if let Some(oldest) = cache.keys().next().cloned() {
                    cache.remove(&oldest);
                }
            }
            cache.insert(key.clone(), order);
        }
        let order = cache.get(&key)?;
        let total = order.len();
        let page = order
            .iter()
            .skip(offset.min(total))
            .take(limit.min(1000))
            .copied()
            .map(|encoded| {
                let position = (encoded >> 1) as usize;
                if encoded & 1 == 1 {
                    IndexedEntry::Directory(position)
                } else {
                    IndexedEntry::File(position)
                }
            })
            .collect();
        Some((total, page))
    }

    fn build_child_order(
        &self,
        position: usize,
        sort: EntrySort,
        descending: bool,
        search: &str,
    ) -> Vec<u64> {
        let mut order = Vec::new();
        if let Some(children) = self.children.get(position) {
            for child in children.iter().copied() {
                if !self.removed_directories.contains(&child)
                    && self.entry_matches((child as u64) << 1 | 1, search)
                {
                    order.push((child as u64) << 1 | 1);
                }
            }
        }
        if let Some(files) = self.files_by_parent.get(position) {
            for file in files.iter().copied() {
                if !self.removed_files.contains(&file)
                    && self.entry_matches((file as u64) << 1, search)
                {
                    order.push((file as u64) << 1);
                }
            }
        }
        order.sort_by(|left, right| {
            let mut comparison = match sort {
                EntrySort::Name => self.entry_name(*left).to_lowercase().cmp(&self.entry_name(*right).to_lowercase()),
                EntrySort::Size => self.entry_size(*left).cmp(&self.entry_size(*right)),
                EntrySort::Count => self.entry_count(*left).cmp(&self.entry_count(*right)),
                EntrySort::Modified => self.entry_modified(*left).cmp(&self.entry_modified(*right)),
            };
            if descending {
                comparison = comparison.reverse();
            }
            comparison.then_with(|| self.entry_path(*left).cmp(self.entry_path(*right)))
        });
        order
    }

    fn entry_matches(&self, encoded: u64, search: &str) -> bool {
        search.is_empty() || self.entry_name(encoded).to_lowercase().contains(search)
    }

    fn entry_name(&self, encoded: u64) -> &str {
        let position = (encoded >> 1) as usize;
        if encoded & 1 == 1 {
            self.directories.get(position).map(|entry| entry.name.as_str()).unwrap_or("")
        } else {
            self.files.get(position).map(|entry| entry.name.as_str()).unwrap_or("")
        }
    }

    fn entry_size(&self, encoded: u64) -> u64 {
        let position = (encoded >> 1) as usize;
        if encoded & 1 == 1 {
            self.aggregates.get(position).map(|aggregate| aggregate.size_bytes).unwrap_or(0)
        } else {
            self.files.get(position).map(|file| file.size).unwrap_or(0)
        }
    }

    fn entry_count(&self, encoded: u64) -> u64 {
        let position = (encoded >> 1) as usize;
        if encoded & 1 == 1 {
            self.aggregates.get(position).map(|aggregate| aggregate.file_count).unwrap_or(0)
        } else {
            0
        }
    }

    fn entry_modified(&self, encoded: u64) -> Option<i64> {
        let position = (encoded >> 1) as usize;
        if encoded & 1 == 1 {
            self.directories.get(position).and_then(|entry| entry.modified_at)
        } else {
            self.files.get(position).and_then(|file| file.modified_at)
        }
    }

    fn entry_path(&self, encoded: u64) -> &Path {
        let position = (encoded >> 1) as usize;
        if encoded & 1 == 1 {
            self.directories.get(position).map(|entry| entry.path.as_path()).unwrap_or_else(|| Path::new(""))
        } else {
            self.files.get(position).map(|file| file.path.as_path()).unwrap_or_else(|| Path::new(""))
        }
    }

    /* ------------------------- post-scan deletion ------------------------- */

    /// Removes a file or folder (and, for folders, everything inside it) from the in-memory index,
    /// subtracting its totals from every ancestor.
    /// Returns `false` when the path is not part of this scan or was already removed.
    pub fn remove(&mut self, path: &Path) -> bool {
        let removed = if let Some(position) = self.index.get(path).copied() {
            self.remove_directory(position)
        } else {
            self.remove_file(path)
        };
        if removed {
            if let Ok(mut cache) = self.child_order_cache.lock() {
                cache.clear();
            }
        }
        removed
    }

    fn remove_directory(&mut self, position: usize) -> bool {
        if !self.removed_directories.insert(position) {
            return false;
        }
        let mut stack = vec![position];
        while let Some(current) = stack.pop() {
            for child in self.children[current].iter().copied() {
                if self.removed_directories.insert(child) {
                    stack.push(child);
                }
            }
            if let Some(files) = self.files_by_parent.get(current) {
                for file in files.iter().copied() {
                    self.removed_files.insert(file);
                }
            }
        }
        let lost = self.aggregates[position].clone();
        let mut ancestor = self.directories[position].parent;
        while let Some(parent) = ancestor {
            let aggregate = &mut self.aggregates[parent];
            aggregate.size_bytes = aggregate.size_bytes.saturating_sub(lost.size_bytes);
            aggregate.file_count = aggregate.file_count.saturating_sub(lost.file_count);
            aggregate.folder_count = aggregate.folder_count.saturating_sub(lost.folder_count.saturating_add(1));
            if let (Some(disk), Some(lost_disk)) = (aggregate.disk_size_bytes, lost.disk_size_bytes) {
                aggregate.disk_size_bytes = Some(disk.saturating_sub(lost_disk));
            }
            ancestor = self.directories[parent].parent;
        }
        true
    }

    fn remove_file(&mut self, path: &Path) -> bool {
        let Some(parent) = path.parent() else {
            return false;
        };
        let Some(position) = self.index.get(parent).copied() else {
            return false;
        };
        let Some(files) = self.files_by_parent.get(position) else {
            return false;
        };
        let Some(&file) = files.iter().find(|&&candidate| self.files[candidate].path == path) else {
            return false;
        };
        if !self.removed_files.insert(file) {
            return false;
        }
        let record = &self.files[file];
        let mut ancestor = Some(position);
        while let Some(parent_position) = ancestor {
            let aggregate = &mut self.aggregates[parent_position];
            aggregate.size_bytes = aggregate.size_bytes.saturating_sub(record.size);
            aggregate.file_count = aggregate.file_count.saturating_sub(1);
            if let (Some(disk), Some(record_disk)) = (aggregate.disk_size_bytes, record.disk_size) {
                aggregate.disk_size_bytes = Some(disk.saturating_sub(record_disk));
            }
            ancestor = self.directories[parent_position].parent;
        }
        true
    }
}

/// Pruning options. Mirrors `PruneOptions` in `src/lib/tree.ts`.
#[derive(Debug, Clone, Copy)]
pub struct PruneOptions {
    pub max_children: usize,
    pub max_nodes: usize,
}

impl Default for PruneOptions {
    fn default() -> Self {
        PruneOptions { max_children: DEFAULT_MAX_CHILDREN, max_nodes: DEFAULT_MAX_NODES }
    }
}

/// A node in the tree sent to the UI. Mirrors `TreeNode` in `src/types/contracts.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub id: String,
    pub path: String,
    pub name: String,
    pub kind: String,
    pub size_bytes: u64,
    pub disk_size_bytes: Option<u64>,
    pub file_count: u64,
    pub folder_count: u64,
    pub modified_at: Option<i64>,
    pub created_at: Option<i64>,
    pub extension: Option<String>,
    pub category: String,
    pub access_denied: bool,
    pub children: Vec<TreeNode>,
    pub synthetic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_symlink: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_junction: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_cloud_placeholder: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_status: Option<String>,
}

/// Result of pruning, including what was dropped.
#[derive(Debug, Clone)]
pub struct PruneResult {
    pub tree: TreeNode,
    pub pruned_nodes: usize,
    pub truncated: bool,
}

/// Builds a bounded-depth tree for the UI. Depth is capped even when the node budget would allow
/// a pathological chain, so JSON serialization and downstream visualization stay stack-safe.
pub fn prune_tree(index: &ScanIndex, root: usize, options: PruneOptions) -> PruneResult {
    if index.directories.get(root).is_none() {
        return PruneResult {
            tree: TreeNode {
                id: "empty".into(), path: String::new(), name: "Unavailable".into(), kind: "folder".into(),
                size_bytes: 0, disk_size_bytes: None, file_count: 0, folder_count: 0,
                modified_at: None, created_at: None, extension: None, category: "other".into(),
                access_denied: true, children: Vec::new(), synthetic: true,
                is_symlink: None, is_junction: None, is_cloud_placeholder: None, cloud_status: None,
            },
            pruned_nodes: 0, truncated: false,
        };
    }

    let max_children = options.max_children.max(2);
    let max_nodes = options.max_nodes.max(1);
    let mut low = 0usize;
    let mut high = index_depth(index, root).min(MAX_TREE_DEPTH);
    while low < high {
        let middle = low + (high - low + 1) / 2;
        if count_limited_nodes(index, root, max_children, middle) <= max_nodes {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    finish(build_limited(index, root, max_children, low))
}

fn finish(result: Limited) -> PruneResult {
    PruneResult {
        tree: result.tree,
        pruned_nodes: result.pruned_nodes,
        truncated: result.pruned_nodes > 0,
    }
}

struct Limited {
    tree: TreeNode,
    pruned_nodes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ChildCandidate {
    Directory(usize),
    File(usize),
}

struct BuildPlan {
    kept: Vec<ChildCandidate>,
    other: Option<TreeNode>,
}

enum BuildTask {
    Directory { position: usize, depth: usize },
    FinishDirectory(usize),
}

fn build_limited(index: &ScanIndex, root: usize, max_children: usize, max_depth: usize) -> Limited {
    let mut nodes: HashMap<usize, TreeNode> = HashMap::new();
    let mut plans: HashMap<usize, BuildPlan> = HashMap::new();
    let mut tasks = vec![BuildTask::Directory { position: root, depth: 0 }];
    let mut pruned_nodes = 0usize;

    while let Some(task) = tasks.pop() {
        match task {
            BuildTask::Directory { position, depth } => {
                let node = directory_node(index, position);
                if depth >= max_depth {
                    pruned_nodes = pruned_nodes
                        .saturating_add(subtree_size(index, position).saturating_sub(1));
                    nodes.insert(position, node);
                    continue;
                }

                let (kept, other, dropped_nodes) = select_children(index, position, max_children);
                pruned_nodes = pruned_nodes.saturating_add(dropped_nodes);
                nodes.insert(position, node);
                plans.insert(position, BuildPlan { kept: kept.clone(), other });
                tasks.push(BuildTask::FinishDirectory(position));
                for candidate in kept.iter().rev() {
                    if let ChildCandidate::Directory(child) = candidate {
                        tasks.push(BuildTask::Directory { position: *child, depth: depth + 1 });
                    }
                }
            }
            BuildTask::FinishDirectory(position) => {
                let Some(plan) = plans.remove(&position) else { continue };
                let other_count = if plan.other.is_some() { 1 } else { 0 };
                let mut children = Vec::with_capacity(plan.kept.len() + other_count);
                for candidate in plan.kept {
                    match candidate {
                        ChildCandidate::Directory(child) => {
                            if let Some(node) = nodes.remove(&child) {
                                children.push(node);
                            }
                        }
                        ChildCandidate::File(file) => children.push(file_node(&index.files[file])),
                    }
                }
                if let Some(other) = plan.other {
                    children.push(other);
                }
                if let Some(node) = nodes.get_mut(&position) {
                    node.children = children;
                }
            }
        }
    }

    let tree = nodes.remove(&root).unwrap_or_else(|| directory_node(index, root));
    Limited { tree, pruned_nodes }
}

fn count_limited_nodes(index: &ScanIndex, root: usize, max_children: usize, max_depth: usize) -> usize {
    let mut total = 0usize;
    let mut stack = vec![(root, 0usize)];
    while let Some((position, depth)) = stack.pop() {
        total = total.saturating_add(1);
        if depth >= max_depth {
            continue;
        }
        let directories = index.children_of(position);
        let files = direct_file_indices(index, position);
        if directories.len().saturating_add(files.len()) <= max_children {
            total = total.saturating_add(files.len());
            stack.extend(directories.into_iter().map(|child| (child, depth + 1)));
        } else {
            total = total.saturating_add(1); // the synthetic Other node
            let kept = top_candidates(index, &directories, &files, max_children - 1);
            for candidate in kept {
                match candidate {
                    ChildCandidate::Directory(child) => stack.push((child, depth + 1)),
                    ChildCandidate::File(_) => total = total.saturating_add(1),
                }
            }
        }
    }
    total
}

fn select_children(
    index: &ScanIndex,
    position: usize,
    max_children: usize,
) -> (Vec<ChildCandidate>, Option<TreeNode>, usize) {
    let directories = index.children_of(position);
    let files = direct_file_indices(index, position);
    let candidate_count = directories.len().saturating_add(files.len());
    if candidate_count <= max_children {
        let mut kept = Vec::with_capacity(candidate_count);
        kept.extend(directories.iter().copied().map(ChildCandidate::Directory));
        kept.extend(files.iter().copied().map(ChildCandidate::File));
        return (kept, None, 0);
    }

    let kept = top_candidates(index, &directories, &files, max_children - 1);
    let kept_set: HashSet<ChildCandidate> = kept.iter().copied().collect();
    let mut dropped_entries = 0usize;
    let mut dropped_nodes = 0usize;
    let mut dropped_bytes = 0u64;
    let mut dropped_files = 0u64;
    let mut dropped_folders = 0u64;
    let mut disk_bytes = 0u64;
    let mut disk_complete = true;

    for child in directories {
        if kept_set.contains(&ChildCandidate::Directory(child)) {
            continue;
        }
        dropped_entries = dropped_entries.saturating_add(1);
        dropped_nodes = dropped_nodes.saturating_add(subtree_size(index, child));
        let aggregate = &index.aggregates[child];
        dropped_bytes = dropped_bytes.saturating_add(aggregate.size_bytes);
        dropped_files = dropped_files.saturating_add(aggregate.file_count);
        dropped_folders = dropped_folders.saturating_add(aggregate.folder_count.saturating_add(1));
        if let Some(bytes) = aggregate.disk_size_bytes {
            disk_bytes = disk_bytes.saturating_add(bytes);
        } else {
            disk_complete = false;
        }
    }
    for file_position in files {
        if kept_set.contains(&ChildCandidate::File(file_position)) {
            continue;
        }
        dropped_entries = dropped_entries.saturating_add(1);
        dropped_nodes = dropped_nodes.saturating_add(1);
        let file = &index.files[file_position];
        dropped_bytes = dropped_bytes.saturating_add(file.size);
        dropped_files = dropped_files.saturating_add(1);
        if let Some(bytes) = file.disk_size {
            disk_bytes = disk_bytes.saturating_add(bytes);
        } else {
            disk_complete = false;
        }
    }

    let directory = &index.directories[position];
    let other_path = format!("{}{}", directory.path.to_string_lossy(), OTHER_NODE_SUFFIX);
    let other = TreeNode {
        id: stable_path_id(Path::new(&other_path)),
        path: other_path,
        name: other_node_name(dropped_entries),
        kind: "folder".to_string(),
        size_bytes: dropped_bytes,
        disk_size_bytes: if disk_complete { Some(disk_bytes) } else { None },
        file_count: dropped_files,
        folder_count: dropped_folders,
        modified_at: None,
        created_at: None,
        extension: None,
        category: "other".to_string(),
        access_denied: false,
        children: Vec::new(),
        synthetic: true,
        is_symlink: None,
        is_junction: None,
        is_cloud_placeholder: None,
        cloud_status: None,
    };
    (kept, Some(other), dropped_nodes)
}

fn direct_file_indices(index: &ScanIndex, position: usize) -> Vec<usize> {
    index
        .files_by_parent
        .get(position)
        .map(|files| {
            files
                .iter()
                .copied()
                .filter(|file| !index.removed_files.contains(file))
                .collect()
        })
        .unwrap_or_default()
}

fn top_candidates(
    index: &ScanIndex,
    directories: &[usize],
    files: &[usize],
    limit: usize,
) -> Vec<ChildCandidate> {
    let mut kept = Vec::with_capacity(limit.min(directories.len().saturating_add(files.len())));
    let mut directory_index = 0usize;
    let mut file_index = 0usize;
    while kept.len() < limit && (directory_index < directories.len() || file_index < files.len()) {
        let directory_candidate = directories
            .get(directory_index)
            .copied()
            .map(ChildCandidate::Directory);
        let file_candidate = files.get(file_index).copied().map(ChildCandidate::File);
        match (directory_candidate, file_candidate) {
            (Some(directory), Some(_file)) if compare_candidates(index, directory, _file) != std::cmp::Ordering::Greater => {
                kept.push(directory);
                directory_index += 1;
            }
            (Some(_), Some(file)) => {
                kept.push(file);
                file_index += 1;
            }
            (Some(directory), None) => {
                kept.push(directory);
                directory_index += 1;
            }
            (None, Some(file)) => {
                kept.push(file);
                file_index += 1;
            }
            (None, None) => break,
        }
    }
    kept
}

fn compare_candidates(index: &ScanIndex, left: ChildCandidate, right: ChildCandidate) -> std::cmp::Ordering {
    candidate_size(index, right)
        .cmp(&candidate_size(index, left))
        .then_with(|| candidate_path(index, left).cmp(candidate_path(index, right)))
}

fn candidate_size(index: &ScanIndex, candidate: ChildCandidate) -> u64 {
    match candidate {
        ChildCandidate::Directory(position) => index.aggregates[position].size_bytes,
        ChildCandidate::File(position) => index.files[position].size,
    }
}

fn candidate_path(index: &ScanIndex, candidate: ChildCandidate) -> &Path {
    match candidate {
        ChildCandidate::Directory(position) => index.directories[position].path.as_path(),
        ChildCandidate::File(position) => index.files[position].path.as_path(),
    }
}

fn directory_node(index: &ScanIndex, position: usize) -> TreeNode {
    let directory = &index.directories[position];
    let aggregate = &index.aggregates[position];
    TreeNode {
        id: stable_path_id(&directory.path),
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
        category: classify(&directory.path, true),
        access_denied: aggregate.access_denied,
        children: Vec::new(),
        synthetic: false,
        is_symlink: if directory.is_symlink { Some(true) } else { None },
        is_junction: if directory.is_junction { Some(true) } else { None },
        is_cloud_placeholder: None,
        cloud_status: None,
    }
}

fn file_node(file: &FileRecord) -> TreeNode {
    TreeNode {
        id: stable_path_id(&file.path),
        path: file.path.to_string_lossy().to_string(),
        name: file.name.clone(),
        kind: "file".to_string(),
        size_bytes: file.size,
        disk_size_bytes: file.disk_size,
        file_count: 1,
        folder_count: 0,
        modified_at: file.modified_at,
        created_at: None,
        extension: file.extension.clone(),
        category: classify(&file.path, false),
        access_denied: false,
        children: Vec::new(),
        synthetic: false,
        is_symlink: if file.is_symlink { Some(true) } else { None },
        is_junction: if file.is_junction { Some(true) } else { None },
        is_cloud_placeholder: if file.is_cloud_placeholder { Some(true) } else { None },
        cloud_status: file.cloud_status.clone(),
    }
}

/// Total number of nodes in a subtree, including the root.
fn subtree_size(index: &ScanIndex, position: usize) -> usize {
    // Iterative so a pathological tree cannot blow the stack.
    let mut total = 0usize;
    let mut stack = vec![position];
    while let Some(current) = stack.pop() {
        total = total.saturating_add(1usize.saturating_add(index.files_of(current).len()));
        for child in index.children_of(current) {
            stack.push(child);
        }
    }
    total
}

fn index_depth(index: &ScanIndex, root: usize) -> usize {
    let mut deepest = 0usize;
    let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
    while let Some((position, depth)) = stack.pop() {
        let file_depth = if index.files_of(position).is_empty() { depth } else { depth + 1 };
        deepest = deepest.max(file_depth);
        for child in index.children_of(position) {
            stack.push((child, depth + 1));
        }
    }
    deepest
}

#[cfg(test)]
fn count_nodes(node: &TreeNode) -> usize {
    let mut total = 0usize;
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        total = total.saturating_add(1);
        stack.extend(current.children.iter());
    }
    total
}

pub fn other_node_name(count: usize) -> String {
    if count == 1 {
        "Other (1 item)".to_string()
    } else {
        format!("Other ({count} items)")
    }
}

/// FNV-1a over the path, matching `stablePathId` in `src/lib/id.ts`.
pub fn stable_path_id(path: &Path) -> String {
    let bytes = path.to_string_lossy().as_bytes().to_vec();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::scanner::walker::{ScanOptions, WalkError};
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("diskwise-agg-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn scan(root: &Path) -> ScanRun {
        let options = ScanOptions {
            root_path: root.to_string_lossy().to_string(),
            ..Default::default()
        };
        let counters = Arc::new(crate::scanner::progress::ProgressCounters::new());
        crate::scanner::walker::walk(&options, &counters)
    }

    #[test]
    fn aggregates_sizes_up_the_tree() {
        let root = temp_dir("rollup");
        fs::create_dir_all(root.join("a").join("b")).unwrap();
        fs::write(root.join("a").join("b").join("deep.bin"), vec![0u8; 2048]).unwrap();
        fs::write(root.join("a").join("mid.bin"), vec![0u8; 512]).unwrap();
        fs::write(root.join("top.txt"), vec![0u8; 64]).unwrap();

        let run = scan(&root);
        let index = ScanIndex::from_run(&run);

        assert_eq!(index.total_bytes(), 2048 + 512 + 64);
        assert_eq!(index.total_files(), 3);
        assert_eq!(index.total_folders(), 2, "a and a\\b");

        let a = index.position_of(&root.join("a")).unwrap();
        assert_eq!(index.aggregates[a].size_bytes, 2048 + 512);
        assert_eq!(index.aggregates[a].file_count, 2);
        assert_eq!(index.aggregates[a].folder_count, 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pruned_tree_keeps_every_level_arithmetically_exact() {
        let root = temp_dir("exact");
        fs::create_dir_all(&root).unwrap();
        for index in 0..40 {
            fs::write(root.join(format!("f{index}.bin")), vec![0u8; 1024]).unwrap();
        }
        let run = scan(&root);
        let index = ScanIndex::from_run(&run);
        let root_position = index.position_of(&root).unwrap();

        let result = prune_tree(
            &index,
            root_position,
            PruneOptions {
                max_children: 10,
                max_nodes: 10_000,
            },
        );

        assert!(result.truncated);
        assert!(result.pruned_nodes > 0);
        // The parent's total is unchanged even though children were folded.
        assert_eq!(result.tree.size_bytes, 40 * 1024);
        assert_eq!(result.tree.file_count, 40);
        // The synthetic node holds exactly the dropped bytes.
        let other = result
            .tree
            .children
            .iter()
            .find(|child| child.synthetic)
            .expect("a synthetic Other node");
        assert!(other.size_bytes > 0);
        assert!(other.path.ends_with(OTHER_NODE_SUFFIX));
        let kept: u64 = result
            .tree
            .children
            .iter()
            .filter(|child| !child.synthetic)
            .map(|child| child.size_bytes)
            .sum();
        assert_eq!(kept + other.size_bytes, result.tree.size_bytes);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn very_deep_directory_chains_are_capped_without_recursive_building() {
        let levels = MAX_TREE_DEPTH + 700;
        let mut directories = Vec::with_capacity(levels + 1);
        let mut path = PathBuf::from("/");
        directories.push(DirectoryRecord {
            name: "root".to_string(),
            path: path.clone(),
            parent: None,
            depth: 0,
            modified_at: None,
            created_at: None,
            is_symlink: false,
            is_junction: false,
            access_denied: false,
        });
        for depth in 1..=levels {
            let name = format!("d{depth}");
            path.push(name.as_str());
            directories.push(DirectoryRecord {
                name,
                path: path.clone(),
                parent: None,
                depth,
                modified_at: None,
                created_at: None,
                is_symlink: false,
                is_junction: false,
                access_denied: false,
            });
        }
        let index = ScanIndex::from_parts(Vec::new(), directories);
        let root = index.position_of(Path::new("/")).unwrap();
        let result = prune_tree(&index, root, PruneOptions::default());
        assert_eq!(count_nodes(&result.tree), MAX_TREE_DEPTH + 1);
        assert_eq!(result.pruned_nodes, levels - MAX_TREE_DEPTH);
        assert!(result.truncated);
    }

    #[test]
    fn a_small_tree_is_not_truncated() {
        let root = temp_dir("small");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::write(root.join("a").join("one.bin"), vec![0u8; 8]).unwrap();
        let run = scan(&root);
        let index = ScanIndex::from_run(&run);
        let position = index.position_of(&root).unwrap();
        let result = prune_tree(&index, position, PruneOptions::default());
        assert!(!result.truncated);
        assert_eq!(result.pruned_nodes, 0);
        assert_eq!(result.tree.children.len(), 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_node_budget_is_respected() {
        let root = temp_dir("budget");
        fs::create_dir_all(&root).unwrap();
        for directory in 0..30 {
            let path = root.join(format!("d{directory}"));
            fs::create_dir_all(&path).unwrap();
            for file in 0..10 {
                fs::write(path.join(format!("f{file}.bin")), vec![0u8; 16]).unwrap();
            }
        }
        let run = scan(&root);
        let index = ScanIndex::from_run(&run);
        let position = index.position_of(&root).unwrap();
        let result = prune_tree(
            &index,
            position,
            PruneOptions {
                max_children: 8,
                max_nodes: 60,
            },
        );
        assert!(count_nodes(&result.tree) <= 60);
        assert_eq!(result.tree.size_bytes, 30 * 10 * 16);

        let _ = fs::remove_dir_all(&root);
    }

    /// A deep, narrow tree: the shape that made the depth search expensive when it walked linearly.
    ///
    /// `prune_tree` finds the depth limit with a binary search, which is only correct because the
    /// node count is monotone non-increasing in the depth limit. That property is asserted here
    /// rather than assumed, because a violation would silently return an over-budget tree.
    fn deep_chain(depth: usize) -> ScanRun {
        let mut directories = Vec::with_capacity(depth);
        let mut files = Vec::with_capacity(depth * 2);
        let mut path = std::path::PathBuf::from("C:\\");
        directories.push(DirectoryRecord {
            name: "C:".to_string(),
            path: path.clone(),
            parent: None,
            depth: 0,
            modified_at: None,
            created_at: None,
            is_symlink: false,
            is_junction: false,
            access_denied: false,
        });
        for level in 0..depth {
            path = path.join(format!("level-{level}"));
            directories.push(DirectoryRecord {
                name: format!("level-{level}"),
                path: path.clone(),
                parent: None,
                depth: level + 1,
                modified_at: None,
                created_at: None,
                is_symlink: false,
                is_junction: false,
                access_denied: false,
            });
            let parent_path = path.clone();
            for file in 0..2 {
                files.push(FileRecord {
                    path: parent_path.join(format!("f{file}.bin")),
                    name: format!("f{file}.bin"),
                    size: 1024 + file as u64,
                    disk_size: Some(4096),
                    modified_at: None,
                    extension: Some("bin".to_string()),
                    depth: level + 1,
                    ..Default::default()
                });
            }
        }
        ScanRun {
            files,
            directories,
            errors: Vec::new(),
            hit_limit: false,
            cancelled: false,
            elapsed_ms: 0,
        }
    }

    #[test]
    fn the_depth_search_returns_a_tree_within_the_budget_at_every_budget() {
        let run = deep_chain(400);
        let index = ScanIndex::from_run(&run);
        let root = index.position_of(Path::new("C:\\")).unwrap();

        let mut previous = usize::MAX;
        for max_nodes in [100_000usize, 10_000, 2_000, 500, 200, 100, 50, 20, 10] {
            let result = prune_tree(
                &index,
                root,
                PruneOptions {
                    max_children: 100,
                    max_nodes,
                },
            );
            let nodes = count_nodes(&result.tree);
            assert!(nodes <= max_nodes, "{nodes} nodes exceeds the budget of {max_nodes}");
            // Monotone: tightening the budget never produces a bigger tree.
            assert!(nodes <= previous, "the node count grew when the budget tightened");
            previous = nodes;
        }
    }

    #[test]
    fn the_root_byte_total_survives_every_budget() {
        let run = deep_chain(300);
        let expected: u64 = run.files.iter().map(|file| file.size).sum();
        let index = ScanIndex::from_run(&run);
        let root = index.position_of(Path::new("C:\\")).unwrap();
        for max_nodes in [10usize, 100, 1_000, 10_000, 1_000_000] {
            let result = prune_tree(
                &index,
                root,
                PruneOptions {
                    max_children: 100,
                    max_nodes,
                },
            );
            assert_eq!(result.tree.size_bytes, expected);
        }
    }

    #[test]
    fn paged_children_are_sorted_filtered_and_capped() {
        let run = deep_chain(1);
        let index = ScanIndex::from_run(&run);
        let folder = index.children_of(0)[0];

        let (total, by_size) = index
            .children_page(folder, 0, 10_000, EntrySort::Size, true, "")
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(by_size.len(), 2);
        let first = match by_size[0] {
            IndexedEntry::File(position) => &index.files[position],
            IndexedEntry::Directory(_) => panic!("expected files in the test folder"),
        };
        assert_eq!(first.name, "f1.bin");

        let (filtered_total, filtered) = index
            .children_page(folder, 0, 100, EntrySort::Name, false, "f1")
            .unwrap();
        assert_eq!(filtered_total, 1);
        assert_eq!(filtered.len(), 1);

        let (empty_total, empty) = index
            .children_page(folder, 0, 1, EntrySort::Name, false, "missing")
            .unwrap();
        assert_eq!(empty_total, 0);
        assert!(empty.is_empty());
    }

    #[test]
    fn paged_children_cache_is_invalidated_after_a_removal() {
        let run = deep_chain(1);
        let mut index = ScanIndex::from_run(&run);
        let folder = index.children_of(0)[0];
        let path = index.files_of(folder)[0].path.clone();
        assert_eq!(index.children_page(folder, 0, 10, EntrySort::Name, false, "").unwrap().0, 2);
        assert!(index.remove(&path));
        assert_eq!(index.children_page(folder, 0, 10, EntrySort::Name, false, "").unwrap().0, 1);
    }

    #[test]
    fn no_folder_ever_reports_more_than_its_children_sum_to() {
        let run = deep_chain(200);
        let index = ScanIndex::from_run(&run);
        let root = index.position_of(Path::new("C:\\")).unwrap();
        let result = prune_tree(&index, root, PruneOptions::default());

        fn walk(node: &TreeNode) {
            if node.children.is_empty() {
                return;
            }
            let child_sum: u64 = node.children.iter().map(|child| child.size_bytes).sum();
            assert!(
                child_sum <= node.size_bytes,
                "{} reports {} but its children sum to {child_sum}",
                node.path,
                node.size_bytes
            );
            for child in &node.children {
                walk(child);
            }
        }
        walk(&result.tree);
    }

    #[test]
    fn stable_path_id_matches_the_frontend_hash_shape() {
        let id = stable_path_id(Path::new("C:\\Users\\me\\file.txt"));
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        // Deterministic.
        assert_eq!(id, stable_path_id(Path::new("C:\\Users\\me\\file.txt")));
    }

    #[test]
    fn an_empty_scan_index_produces_a_single_node_tree() {
        let index = ScanIndex::default();
        let result = prune_tree(&index, 0, PruneOptions::default());
        assert_eq!(count_nodes(&result.tree), 1);
        assert!(!result.truncated);
    }

    #[test]
    fn other_node_name_singular_and_plural() {
        assert_eq!(other_node_name(1), "Other (1 item)");
        assert_eq!(other_node_name(7), "Other (7 items)");
    }

    #[test]
    fn a_walk_error_is_preserved() {
        let error = WalkError {
            path: "C:\\secret".to_string(),
            kind: "access-denied".to_string(),
            message: "Access is denied.".to_string(),
        };
        assert_eq!(error.kind, "access-denied");
    }

    #[test]
    fn removing_a_file_subtracts_it_from_every_ancestor() {
        let root = temp_dir("remove-file");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::write(root.join("a\\keep.bin"), vec![0u8; 100]).unwrap();
        fs::write(root.join("a\\drop.bin"), vec![0u8; 50]).unwrap();
        let run = scan(&root);
        let mut index = ScanIndex::from_run(&run);

        assert_eq!(index.total_bytes(), 150);
        assert!(index.remove(&root.join("a\\drop.bin")));
        assert_eq!(index.total_bytes(), 100);
        assert_eq!(index.total_files(), 1);
        assert_eq!(index.active_files().count(), 1);
        // The folder keeps its remaining child.
        let a = index.position_of(&root.join("a")).unwrap();
        assert_eq!(index.files_of(a).len(), 1);
        assert_eq!(index.files_of(a)[0].name, "keep.bin");
        // Removing twice is a no-op.
        assert!(!index.remove(&root.join("a\\drop.bin")));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn removing_a_folder_removes_its_whole_subtree() {
        let root = temp_dir("remove-folder");
        fs::create_dir_all(root.join("a\\b")).unwrap();
        fs::write(root.join("a\\b\\deep.bin"), vec![0u8; 32]).unwrap();
        fs::write(root.join("a\\mid.bin"), vec![0u8; 16]).unwrap();
        fs::write(root.join("top.bin"), vec![0u8; 8]).unwrap();
        let run = scan(&root);
        let mut index = ScanIndex::from_run(&run);

        assert_eq!(index.total_bytes(), 56);
        assert_eq!(index.total_folders(), 2);
        assert!(index.remove(&root.join("a")));
        // Bytes, files and the folder itself all disappear from every total.
        assert_eq!(index.total_bytes(), 8);
        assert_eq!(index.total_files(), 1);
        assert_eq!(index.total_folders(), 0);
        // The folder and its contents are no longer reachable.
        assert!(index.position_of(&root.join("a")).is_none());
        assert!(index.position_of(&root.join("a\\b")).is_none());
        let root_position = index.position_of(&root).unwrap();
        assert!(index.children_of(root_position).is_empty());
        // Unknown paths report "nothing removed" rather than panicking.
        assert!(!index.remove(&root.join("nope")));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_removed_subtree_disappears_from_the_pruned_tree() {
        let root = temp_dir("remove-tree");
        fs::create_dir_all(root.join("gone")).unwrap();
        fs::write(root.join("gone\\x.bin"), vec![0u8; 64]).unwrap();
        fs::write(root.join("stays.bin"), vec![0u8; 32]).unwrap();
        let run = scan(&root);
        let mut index = ScanIndex::from_run(&run);
        index.remove(&root.join("gone"));

        let result = prune_tree(&index, 0, PruneOptions::default());
        assert_eq!(result.tree.size_bytes, 32);
        assert!(result
            .tree
            .children
            .iter()
            .all(|child| child.name != "gone"));
        assert_eq!(result.tree.children.len(), 1);

        let _ = fs::remove_dir_all(&root);
    }
}
