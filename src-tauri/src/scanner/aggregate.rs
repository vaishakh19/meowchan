//! Aggregation and tree pruning.
//!
//! The walker produces a flat list of files and a flat list of directories. Aggregation rolls those
//! up into per-directory totals, then the pruner turns them into the bounded tree the UI receives.
//!
//! The pruner is the Rust half of the contract documented in `src/lib/tree.ts` and must stay
//! behaviourally identical to it: the same `maxChildrenPerNode`, the same synthetic
//! `Other (n items)` node, the same node budget. Sizes stay truthful at every level; only the
//! *granularity* is reduced.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::filesystem::classify;
use crate::scanner::walker::{DirectoryRecord, FileRecord, ScanRun};

/// Children kept per folder in the tree sent to the UI.
pub const DEFAULT_MAX_CHILDREN: usize = 100;
/// Hard cap on the number of nodes in the returned tree.
pub const DEFAULT_MAX_NODES: usize = 150_000;
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
    /// `parent display path -> indices into files`, biggest first.
    files_by_parent: HashMap<PathBuf, Vec<usize>>,
}

impl ScanIndex {
    /// Builds the index from a walk result.
    pub fn from_run(run: &ScanRun) -> Self {
        let mut directories = run.directories.clone();
        directories.sort_by(|a, b| a.path.cmp(&b.path));

        let mut index: HashMap<PathBuf, usize> = HashMap::with_capacity(directories.len());
        for (position, directory) in directories.iter().enumerate() {
            index.insert(directory.path.clone(), position);
        }

        // Resolve parent indices now that every directory is in the table.
        for directory in directories.iter_mut() {
            directory.parent = directory
                .path
                .parent()
                .and_then(|parent| index.get(parent).copied());
        }

        let mut aggregates = vec![DirectoryAggregate::default(); directories.len()];

        // Own timestamps per directory.
        for (position, directory) in directories.iter().enumerate() {
            aggregates[position].modified_at = directory.modified_at;
            aggregates[position].created_at = directory.created_at;
            aggregates[position].access_denied = directory.access_denied;
        }

        // Files roll into their parent directory.
        let mut files_by_parent: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        for (position, file) in run.files.iter().enumerate() {
            files_by_parent
                .entry(file.parent_path.clone())
                .or_default()
                .push(position);
            if let Some(directory) = index.get(&file.parent_path) {
                let aggregate = &mut aggregates[*directory];
                aggregate.size_bytes += file.size;
                aggregate.disk_size_bytes =
                    Some(aggregate.disk_size_bytes.unwrap_or(0) + file.disk_size.unwrap_or(file.size));
                aggregate.file_count += 1;
            }
        }
        for entries in files_by_parent.values_mut() {
            entries.sort_by(|a, b| {
                run.files[*b]
                    .size
                    .cmp(&run.files[*a].size)
                    .then_with(|| run.files[*a].path.cmp(&run.files[*b].path))
            });
        }

        // Directories roll up from the deepest to the shallowest, so one pass suffices.
        let mut order: Vec<usize> = (0..directories.len()).collect();
        order.sort_by(|a, b| directories[*b].depth.cmp(&directories[*a].depth));
        for position in order {
            let (size, files, folders, disk) = {
                let aggregate = &aggregates[position];
                (aggregate.size_bytes, aggregate.file_count, aggregate.folder_count, aggregate.disk_size_bytes)
            };
            if let Some(parent) = directories[position].parent {
                let parent_aggregate = &mut aggregates[parent];
                parent_aggregate.size_bytes += size;
                parent_aggregate.disk_size_bytes = Some(parent_aggregate.disk_size_bytes.unwrap_or(0) + disk.unwrap_or(size));
                parent_aggregate.file_count += files;
                parent_aggregate.folder_count += folders + 1;
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
            files: run.files.clone(),
            index,
            children,
            files_by_parent,
        }
    }

    pub fn directory_count(&self) -> usize {
        self.directories.len()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn total_bytes(&self) -> u64 {
        self.aggregates.first().map(|a| a.size_bytes).unwrap_or(0)
    }

    pub fn total_disk_bytes(&self) -> Option<u64> {
        self.aggregates.first().and_then(|a| a.disk_size_bytes)
    }

    pub fn total_files(&self) -> u64 {
        self.aggregates.first().map(|a| a.file_count).unwrap_or(0)
    }

    pub fn total_folders(&self) -> u64 {
        self.aggregates.first().map(|a| a.folder_count).unwrap_or(0)
    }

    pub fn position_of(&self, path: &Path) -> Option<usize> {
        self.index.get(path).copied()
    }

    /// Direct children of a directory, biggest first.
    pub fn children_of(&self, position: usize) -> &[usize] {
        &self.children[position]
    }

    /// Files whose parent is `position`, biggest first.
    pub fn files_of(&self, position: usize) -> Vec<&FileRecord> {
        let target = &self.directories[position].path;
        self.files_by_parent
            .get(target)
            .map(|entries| entries.iter().map(|index| &self.files[*index]).collect())
            .unwrap_or_default()
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
        PruneOptions {
            max_children: DEFAULT_MAX_CHILDREN,
            max_nodes: DEFAULT_MAX_NODES,
        }
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
}

/// Result of pruning, including what was dropped.
#[derive(Debug, Clone)]
pub struct PruneResult {
    pub tree: TreeNode,
    pub pruned_nodes: usize,
    pub truncated: bool,
}

/// Builds the pruned tree for a scan index rooted at `root`.
///
/// The depth limit is found with a **binary search**, not a linear walk. `build_limited`'s node count
/// is monotone non-increasing in `max_depth` — cutting depth can only turn subtrees into leaves — so
/// the search is sound and costs `O(log depth)` builds instead of `O(depth)`.
///
/// This is the same algorithm `pruneTree` in `src/lib/tree.ts` uses, and the two must stay
/// equivalent: `src/lib/tree.test.ts` asserts the monotonicity the search relies on, and the
/// benchmark in `src-tauri/src/bin/bench.rs` measures the cost.
pub fn prune_tree(index: &ScanIndex, root: usize, options: PruneOptions) -> PruneResult {
    let max_children = options.max_children.max(2);
    let max_nodes = options.max_nodes.max(max_children + 1);

    let depth = index_depth(index, root);
    let mut result = build_limited(index, root, max_children, depth);
    if count_nodes(&result.tree) <= max_nodes {
        return finish(result);
    }

    // The tree is too big, so the depth has to come down.
    let mut low = 1usize;
    let mut high = depth.saturating_sub(1).max(1);
    while low < high {
        let middle = low + (high - low + 1) / 2;
        let candidate = build_limited(index, root, max_children, middle);
        if count_nodes(&candidate.tree) <= max_nodes {
            low = middle;
            result = candidate;
        } else {
            high = middle - 1;
        }
    }

    // Rebuild once at the deepest level that fits, so the returned result is the best one found.
    result = build_limited(index, root, max_children, low);
    if count_nodes(&result.tree) > max_nodes && low > 1 {
        result = build_limited(index, root, max_children, 1);
    }
    finish(result)
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

fn build_limited(index: &ScanIndex, root: usize, max_children: usize, max_depth: usize) -> Limited {
    let mut pruned_nodes = 0usize;
    let tree = build_node(index, root, 0, max_children, max_depth, &mut pruned_nodes);
    Limited { tree, pruned_nodes }
}

fn build_node(
    index: &ScanIndex,
    position: usize,
    depth: usize,
    max_children: usize,
    max_depth: usize,
    pruned_nodes: &mut usize,
) -> TreeNode {
    let directory = &index.directories[position];
    let aggregate = &index.aggregates[position];
    let path = &directory.path;

    let mut node = TreeNode {
        id: stable_path_id(path),
        path: path.to_string_lossy().to_string(),
        name: directory.name.clone(),
        kind: "folder".to_string(),
        size_bytes: aggregate.size_bytes,
        disk_size_bytes: aggregate.disk_size_bytes,
        file_count: aggregate.file_count,
        folder_count: aggregate.folder_count,
        modified_at: aggregate.modified_at,
        created_at: aggregate.created_at,
        extension: None,
        category: classify(path, true),
        access_denied: aggregate.access_denied,
        children: Vec::new(),
        synthetic: false,
    };

    if depth >= max_depth {
        // Everything below this point is folded into the node's totals.
        *pruned_nodes += subtree_size(index, position);
        return node;
    }

    let mut children: Vec<TreeNode> = Vec::new();

    // Folders first (biggest first), then the folder's own files.
    let child_dirs: Vec<usize> = index.children_of(position).to_vec();
    let child_files = index.files_of(position);

    let mut candidates: Vec<TreeNode> = Vec::with_capacity(child_dirs.len() + child_files.len());
    for child in &child_dirs {
        candidates.push(build_node(index, *child, depth + 1, max_children, max_depth, pruned_nodes));
    }
    for file in &child_files {
        candidates.push(file_node(file));
    }

    if candidates.len() <= max_children {
        node.children = candidates;
        return node;
    }

    // Keep the biggest consumers; fold the rest into one honest aggregate node.
    candidates.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then_with(|| a.path.cmp(&b.path)));
    let keep = max_children - 1;
    let dropped: Vec<TreeNode> = candidates.split_off(keep);
    *pruned_nodes += dropped.len();

    let dropped_bytes: u64 = dropped.iter().map(|node| node.size_bytes).sum();
    let dropped_files: u64 = dropped.iter().map(|node| node.file_count).sum();
    let dropped_folders: u64 = dropped
        .iter()
        .map(|node| node.folder_count + if node.kind == "folder" { 1 } else { 0 })
        .sum();
    let dropped_disk: Option<u64> = if dropped.iter().any(|node| node.disk_size_bytes.is_some()) {
        Some(dropped.iter().map(|node| node.disk_size_bytes.unwrap_or(0)).sum())
    } else {
        None
    };

    let other_path = format!("{}{}", path.to_string_lossy(), OTHER_NODE_SUFFIX);
    let other = TreeNode {
        id: stable_path_id(Path::new(&other_path)),
        path: other_path.clone(),
        name: other_node_name(dropped.len()),
        kind: "folder".to_string(),
        size_bytes: dropped_bytes,
        disk_size_bytes: dropped_disk,
        file_count: dropped_files,
        folder_count: dropped_folders,
        modified_at: None,
        created_at: None,
        extension: None,
        category: "other".to_string(),
        access_denied: false,
        children: Vec::new(),
        synthetic: true,
    };

    node.children = candidates;
    node.children.push(other);
    node
}

fn file_node(file: &FileRecord) -> TreeNode {
    let path = &file.path;
    TreeNode {
        id: stable_path_id(path),
        path: path.to_string_lossy().to_string(),
        name: file.name.clone(),
        kind: "file".to_string(),
        size_bytes: file.size,
        disk_size_bytes: file.disk_size,
        file_count: 0,
        folder_count: 0,
        modified_at: file.modified_at,
        created_at: None,
        extension: file.extension.clone(),
        category: classify(path, false),
        access_denied: false,
        children: Vec::new(),
        synthetic: false,
    }
}

/// Total number of nodes in a subtree, including the root.
fn subtree_size(index: &ScanIndex, position: usize) -> usize {
    // Iterative so a pathological tree cannot blow the stack.
    let mut total = 0usize;
    let mut stack = vec![position];
    while let Some(current) = stack.pop() {
        total += 1 + index.files_of(current).len();
        for child in index.children_of(current) {
            stack.push(*child);
        }
    }
    total
}

fn index_depth(index: &ScanIndex, root: usize) -> usize {
    let mut deepest = 0usize;
    let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
    while let Some((position, depth)) = stack.pop() {
        deepest = deepest.max(depth);
        for child in index.children_of(position) {
            stack.push((*child, depth + 1));
        }
    }
    deepest
}

fn count_nodes(node: &TreeNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
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
        hash ^= *byte as u64;
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
        fs::create_dir_all(root.join("a\\b")).unwrap();
        fs::write(root.join("a\\b\\deep.bin"), vec![0u8; 2048]).unwrap();
        fs::write(root.join("a\\mid.bin"), vec![0u8; 512]).unwrap();
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
    fn a_small_tree_is_not_truncated() {
        let root = temp_dir("small");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::write(root.join("a\\one.bin"), vec![0u8; 8]).unwrap();
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
                    parent_path: parent_path.clone(),
                    depth: level + 1,
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
}
