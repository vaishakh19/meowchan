/**
 * Tree pruning — the frontend half of the "never ship millions of nodes" contract.
 *
 * The Rust backend aggregates the whole filesystem but only sends the UI a bounded tree:
 * `maxChildrenPerNode` children per folder (plus one synthetic `Other (n items)` node that keeps
 * the parent's size exact) and a global node budget enforced by limiting depth. Sizes stay
 * truthful at every level; only the *granularity* is reduced.
 *
 * This module mirrors `src-tauri/src/scanner/aggregate.rs::prune_tree` and is used by UI
 * quick-win summaries and unit tests.
 */

import type { TreeNode } from '@/types/contracts';
import { stablePathId } from '@/lib/id';

/** Path suffix of the synthetic aggregate node. The backend refuses filesystem ops on these. */
export const OTHER_NODE_SUFFIX = '\\[Other]';

export function otherNodeName(count: number): string {
  return `Other (${count} ${count === 1 ? 'item' : 'items'})`;
}

export function isSyntheticPath(path: string): boolean {
  return path.endsWith(OTHER_NODE_SUFFIX);
}

export interface PruneResult {
  tree: TreeNode;
  /** Nodes dropped from the tree (their bytes are still counted in their parent). */
  prunedNodes: number;
  truncated: boolean;
}

export interface PruneOptions {
  /** Children kept per folder. Must be >= 2. */
  maxChildren?: number;
  /** Hard cap on the number of nodes in the returned tree. */
  maxNodes?: number;
}

export const DEFAULT_PRUNE_OPTIONS: Required<PruneOptions> = {
  maxChildren: 100,
  maxNodes: 150_000,
};

export const MAX_TREE_DEPTH = 256;

export function pruneTree(root: TreeNode, options: PruneOptions = {}): PruneResult {
  const requestedChildren = options.maxChildren ?? DEFAULT_PRUNE_OPTIONS.maxChildren;
  const requestedNodes = options.maxNodes ?? DEFAULT_PRUNE_OPTIONS.maxNodes;
  const maxChildren = Math.max(2, Math.floor(Number.isFinite(requestedChildren)
    ? requestedChildren
    : DEFAULT_PRUNE_OPTIONS.maxChildren));
  const maxNodes = Math.max(1, Math.floor(Number.isFinite(requestedNodes)
    ? requestedNodes
    : DEFAULT_PRUNE_OPTIONS.maxNodes));
  const maxDepth = Math.min(treeDepth(root), MAX_TREE_DEPTH);

  // Node count is monotone as the depth limit decreases. Search for the deepest tree that fits,
  // without building the entire input tree first or recursing through hostile directory chains.
  let low = 0;
  let high = maxDepth;
  while (low < high) {
    const middle = Math.floor((low + high + 1) / 2);
    const candidate = buildLimited(root, maxChildren, middle);
    if (countNodes(candidate.tree) <= maxNodes) {
      low = middle;
    } else {
      high = middle - 1;
    }
  }
  return buildLimited(root, maxChildren, low);
}

interface BuildPlan {
  kept: TreeNode[];
  other: TreeNode | null;
}

type BuildTask =
  | { type: 'visit'; node: TreeNode; depth: number }
  | { type: 'finish'; node: TreeNode };

function buildLimited(root: TreeNode, maxChildren: number, maxDepth: number): PruneResult {
  let prunedNodes = 0;
  const built = new Map<TreeNode, TreeNode>();
  const plans = new Map<TreeNode, BuildPlan>();
  const tasks: BuildTask[] = [{ type: 'visit', node: root, depth: 0 }];

  while (tasks.length > 0) {
    const task = tasks.pop()!;
    if (task.type === 'visit') {
      const { node, depth } = task;
      if (node.kind === 'file' || node.children.length === 0) {
        built.set(node, { ...node, children: [] });
        continue;
      }
      if (depth >= maxDepth) {
        prunedNodes += countNodes(node) - 1;
        built.set(node, { ...node, children: [] });
        continue;
      }

      let kept: TreeNode[];
      let other: TreeNode | null = null;
      if (node.children.length <= maxChildren) {
        kept = node.children;
      } else {
        // Match the backend: keep largest items first, with a path tie-breaker.
        const sorted = [...node.children].sort(compareBySizeThenPath);
        const keepCount = maxChildren - 1;
        kept = sorted.slice(0, keepCount);
        const dropped = sorted.slice(keepCount);
        for (const child of dropped) prunedNodes += countNodes(child);

        const droppedBytes = dropped.reduce((sum, child) => sum + child.sizeBytes, 0);
        const droppedFiles = dropped.reduce((sum, child) => sum + child.fileCount, 0);
        const droppedFolders = dropped.reduce(
          (sum, child) => sum + child.folderCount + (child.kind === 'folder' ? 1 : 0),
          0,
        );
        const otherPath = `${node.path}${OTHER_NODE_SUFFIX}`;
        other = {
          id: stablePathId(otherPath),
          path: otherPath,
          name: otherNodeName(dropped.length),
          kind: 'folder',
          sizeBytes: droppedBytes,
          diskSizeBytes: dropped.every((child) => child.diskSizeBytes !== null)
            ? dropped.reduce((sum, child) => sum + (child.diskSizeBytes ?? 0), 0)
            : null,
          fileCount: droppedFiles,
          folderCount: droppedFolders,
          modifiedAt: null,
          createdAt: null,
          extension: null,
          category: 'other',
          accessDenied: false,
          children: [],
          synthetic: true,
        };
      }

      plans.set(node, { kept, other });
      tasks.push({ type: 'finish', node });
      for (let index = kept.length - 1; index >= 0; index -= 1) {
        tasks.push({ type: 'visit', node: kept[index], depth: depth + 1 });
      }
      continue;
    }

    const plan = plans.get(task.node);
    if (!plan) continue;
    const children: TreeNode[] = [];
    for (const child of plan.kept) {
      const builtChild = built.get(child);
      if (builtChild) children.push(builtChild);
    }
    if (plan.other) children.push(plan.other);
    built.set(task.node, { ...task.node, children });
    plans.delete(task.node);
  }

  const tree = built.get(root) ?? { ...root, children: [] };
  return { tree, prunedNodes, truncated: prunedNodes > 0 };
}

function compareBySizeThenPath(left: TreeNode, right: TreeNode): number {
  const bySize = right.sizeBytes - left.sizeBytes;
  if (bySize !== 0) return bySize;
  return left.path < right.path ? -1 : left.path > right.path ? 1 : 0;
}

/** Node count including the root. */
export function countNodes(node: TreeNode): number {
  let total = 0;
  const stack = [node];
  while (stack.length > 0) {
    const current = stack.pop()!;
    total += 1;
    for (const child of current.children) stack.push(child);
  }
  return total;
}

/** Maximum depth of the tree (root = 0). */
export function treeDepth(node: TreeNode): number {
  let deepest = 0;
  const stack: Array<{ node: TreeNode; depth: number }> = [{ node, depth: 0 }];
  while (stack.length > 0) {
    const current = stack.pop()!;
    deepest = Math.max(deepest, current.depth);
    for (const child of current.node.children) {
      stack.push({ node: child, depth: current.depth + 1 });
    }
  }
  return deepest;
}

/**
 * Removes the synthetic `Other` node from a child list. Used by the file list, which can page
 * through real children instead of showing an aggregate.
 */
export function withoutSynthetic(nodes: TreeNode[]): TreeNode[] {
  return nodes.filter((node) => !node.synthetic);
}

/** Flattens a tree into its files (optionally filtered). */
export function collectFiles(root: TreeNode, predicate?: (node: TreeNode) => boolean): TreeNode[] {
  const out: TreeNode[] = [];
  const stack: TreeNode[] = [root];
  while (stack.length > 0) {
    const node = stack.pop()!;
    if (node.kind === 'file' && (!predicate || predicate(node))) out.push(node);
    for (const child of node.children) stack.push(child);
  }
  return out;
}

/** All folders in the tree, in no particular order. */
export function collectFolders(root: TreeNode): TreeNode[] {
  const out: TreeNode[] = [];
  const stack: TreeNode[] = [root];
  while (stack.length > 0) {
    const node = stack.pop()!;
    if (node.kind === 'folder') out.push(node);
    for (const child of node.children) stack.push(child);
  }
  return out;
}
