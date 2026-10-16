/**
 * Visualization layout math — pure, dependency-light and unit-tested.
 *
 * Everything here is computed from a `ScanResult.tree` and contains no React and no DOM access,
 * so the same functions power the app, the unit tests and the performance benchmarks
 * (`src/features/visualization/layout.bench.ts`).
 *
 * Design notes
 * ------------
 * - Angles are distributed *proportionally to logical size*, exactly like `d3.partition`.
 *   Parents are drawn as full annulus sectors, so a child that is too thin to render simply
 *   shows its parent's colour in that sliver instead of leaving a hole in the ring. The ring
 *   therefore always sums to 100% of the analyzed bytes.
 * - Nodes are pruned by *rendering*, never by *size*: totals stay honest.
 * - Zero-byte roots and empty folders produce an empty layout rather than a divide-by-zero.
 */

import { hierarchy, partition, treemap, treemapSquarify, type HierarchyRectangularNode } from 'd3';
import type { StorageCategory, TreeNode } from '@/types/contracts';

/* ------------------------------------------------------------------ *
 * Sunburst
 * ------------------------------------------------------------------ */

export interface SunburstArc {
  id: string;
  path: string;
  name: string;
  kind: 'folder' | 'file';
  /** 0 = root (not rendered), 1 = first ring. */
  depth: number;
  category: StorageCategory;
  sizeBytes: number;
  /** Radians, 0..2π, clockwise from 12 o'clock. */
  startAngle: number;
  endAngle: number;
  innerRadius: number;
  outerRadius: number;
  isSynthetic: boolean;
  accessDenied: boolean;
  /** Fraction of the whole root, 0..1. */
  fraction: number;
  /** Parent path, or `null` for the first ring. */
  parentPath: string | null;
}

export interface SunburstLayout {
  arcs: SunburstArc[];
  totalBytes: number;
  maxDepth: number;
  /** Arcs omitted because they were thinner than the angular threshold. */
  collapsedArcs: number;
  collapsedBytes: number;
  /** True when the root has no measurable size (empty folder / all zero-byte files). */
  empty: boolean;
}

export interface SunburstOptions {
  /** Hard cap on rendered arcs. Extra arcs are dropped by raising the angular threshold. */
  maxVisibleArcs?: number;
  /** Arcs thinner than this (degrees) are not rendered. */
  minAngleDeg?: number;
  /** Do not render rings deeper than this. `0` = unlimited. */
  maxDepth?: number;
  /** Radius of the outer ring in layout units (the component scales to pixels). */
  radius?: number;
  /** Fraction of the radius left empty in the middle for the centre button. */
  innerRadiusFraction?: number;
}

const DEG = Math.PI / 180;
const TAU = Math.PI * 2;

export function buildSunburstLayout(root: TreeNode, options: SunburstOptions = {}): SunburstLayout {
  const {
    maxVisibleArcs = 2400,
    minAngleDeg = 0.12,
    maxDepth = 0,
    radius = 100,
    innerRadiusFraction = 0.16,
  } = options;

  const totalBytes = Math.max(0, root.sizeBytes);
  if (totalBytes <= 0) {
    return {
      arcs: [],
      totalBytes: 0,
      maxDepth: 0,
      collapsedArcs: 0,
      collapsedBytes: 0,
      empty: true,
    };
  }

  const rootHierarchy = hierarchy<TreeNode>(root)
    .sum((node) => (node.kind === 'file' ? Math.max(0, node.sizeBytes) : 0))
    .sort((a, b) => (b.value ?? 0) - (a.value ?? 0) || a.data.name.localeCompare(b.data.name));

  const innerRadius = radius * innerRadiusFraction;
  const laidOut: HierarchyRectangularNode<TreeNode> = partition<TreeNode>().size([
    TAU,
    radius - innerRadius,
  ])(rootHierarchy);

  const all: SunburstArc[] = [];
  let layoutMaxDepth = 1;
  let collapsedZero = 0;

  laidOut.each((node: HierarchyRectangularNode<TreeNode>) => {
    if (node.depth === 0) return;
    // Zero-byte files and empty folders take no angle: nothing to draw, and drawing them
    // would produce degenerate paths.
    if (node.x1 - node.x0 <= 0) {
      collapsedZero += 1;
      return;
    }
    layoutMaxDepth = Math.max(layoutMaxDepth, node.depth);
    const data = node.data;
    const span = Math.max(0, node.x1 - node.x0);
    all.push({
      id: data.id,
      path: data.path,
      name: data.name,
      kind: data.kind,
      depth: node.depth,
      category: data.category,
      sizeBytes: Math.max(0, data.sizeBytes),
      startAngle: node.x0,
      endAngle: node.x1,
      innerRadius: innerRadius + node.y0,
      outerRadius: innerRadius + node.y1,
      isSynthetic: data.synthetic,
      accessDenied: data.accessDenied,
      fraction: span / TAU,
      parentPath: node.parent ? node.parent.data.path : null,
    });
  });

  const depthLimit = maxDepth > 0 ? maxDepth : Number.MAX_SAFE_INTEGER;
  const candidates = all.filter((arc) => arc.depth <= depthLimit);

  // Thinning strategy: keep the biggest arcs up to the cap, then drop anything thinner than
  // `minAngleDeg` (sub-pixel, invisible anyway). Sizes stay exact — a dropped arc simply shows
  // its parent's colour in that sliver. The chart is never allowed to come back empty.
  const bySize = [...candidates].sort((a, b) => b.sizeBytes - a.sizeBytes || a.path.localeCompare(b.path));
  let kept = bySize
    .slice(0, Math.max(1, maxVisibleArcs))
    .filter((arc) => arc.depth <= 2 || arc.endAngle - arc.startAngle >= minAngleDeg * DEG);
  if (kept.length === 0) {
    kept = bySize.slice(0, Math.min(maxVisibleArcs, 40));
  }
  // Render innermost rings first so deeper arcs paint over their parents.
  kept.sort((a, b) => a.depth - b.depth || a.startAngle - b.startAngle);

  const keptPaths = new Set(kept.map((arc) => arc.path));
  const collapsed = all.filter((arc) => !keptPaths.has(arc.path));

  return {
    arcs: kept,
    totalBytes,
    maxDepth: layoutMaxDepth,
    collapsedArcs: collapsed.length + collapsedZero,
    collapsedBytes: collapsed.reduce((sum, arc) => sum + arc.sizeBytes, 0),
    empty: false,
  };
}

/** SVG arc path for a sunburst arc, with an optional gap between segments. */
export function sunburstArcPath(arc: SunburstArc, gapDeg = 0): string {
  const span = arc.endAngle - arc.startAngle;
  if (!(span > 0)) return '';
  const gap = Math.max(0, Math.min(gapDeg * DEG, span / 2));
  const a0 = arc.startAngle + gap;
  const a1 = arc.endAngle - gap;
  if (a1 - a0 <= 1e-9) return '';
  const r0 = Math.max(0, arc.innerRadius);
  const r1 = Math.max(r0 + 0.01, arc.outerRadius);
  const large = a1 - a0 > Math.PI ? 1 : 0;
  // d3-shape convention: 0 rad is at 12 o'clock, clockwise.
  const x0 = r1 * Math.sin(a0);
  const y0 = -r1 * Math.cos(a0);
  const x1 = r1 * Math.sin(a1);
  const y1 = -r1 * Math.cos(a1);
  const x2 = r0 * Math.sin(a1);
  const y2 = -r0 * Math.cos(a1);
  const x3 = r0 * Math.sin(a0);
  const y3 = -r0 * Math.cos(a0);
  return [
    `M${x0.toFixed(3)},${y0.toFixed(3)}`,
    `A${r1.toFixed(3)},${r1.toFixed(3)} 0 ${large} 1 ${x1.toFixed(3)},${y1.toFixed(3)}`,
    `L${x2.toFixed(3)},${y2.toFixed(3)}`,
    `A${r0.toFixed(3)},${r0.toFixed(3)} 0 ${large} 0 ${x3.toFixed(3)},${y3.toFixed(3)}`,
    'Z',
  ].join(' ');
}

/** Hit-test: which arc sits under the given polar coordinates (radius, angle in radians)? */
export function arcAtPoint(arcs: SunburstArc[], radius: number, angle: number): SunburstArc | null {
  const r = Math.max(0, radius);
  const a = ((angle % TAU) + TAU) % TAU;
  // Deepest (innermost) match wins, so the first ring stays clickable when deeper rings overlap.
  let best: SunburstArc | null = null;
  for (const arc of arcs) {
    if (r < arc.innerRadius || r > arc.outerRadius) continue;
    if (a < arc.startAngle || a > arc.endAngle) continue;
    if (!best || arc.innerRadius > best.innerRadius) best = arc;
  }
  return best;
}

/* ------------------------------------------------------------------ *
 * Treemap
 * ------------------------------------------------------------------ */

export interface TreemapCell {
  id: string;
  path: string;
  name: string;
  depth: number;
  category: StorageCategory;
  sizeBytes: number;
  x: number;
  y: number;
  width: number;
  height: number;
  isSynthetic: boolean;
  /** Children count, used to decide whether a label can be shown. */
  childCount: number;
}

export interface TreemapLayout {
  cells: TreemapCell[];
  totalBytes: number;
  maxDepth: number;
  empty: boolean;
}

export interface TreemapOptions {
  width: number;
  height: number;
  /** Deepest ring rendered. Folders deeper than this are folded into their parent. */
  maxDepth?: number;
  /** Minimum cell area in px² before it is skipped. */
  minArea?: number;
}

export function buildTreemapLayout(root: TreeNode, options: TreemapOptions): TreemapLayout {
  const { width, height, maxDepth = 5, minArea = 220 } = options;
  const totalBytes = Math.max(0, root.sizeBytes);
  if (totalBytes <= 0 || width <= 0 || height <= 0) {
    return { cells: [], totalBytes, maxDepth: 0, empty: true };
  }

  const rootHierarchy = hierarchy<TreeNode>(root)
    .sum((node) => (node.kind === 'file' ? Math.max(0, node.sizeBytes) : 0))
    .sort((a, b) => (b.value ?? 0) - (a.value ?? 0));

  const laidOut: HierarchyRectangularNode<TreeNode> = treemap<TreeNode>()
    .tile(treemapSquarify)
    .size([width, height])
    .paddingInner(2)
    .paddingOuter(0)
    .round(true)(rootHierarchy);

  const cells: TreemapCell[] = [];
  let renderedMaxDepth = 0;
  laidOut.each((node: HierarchyRectangularNode<TreeNode>) => {
    if (node.depth === 0) return;
    if (node.depth > maxDepth) return;
    const w = Math.max(0, node.x1 - node.x0);
    const h = Math.max(0, node.y1 - node.y0);
    if (w * h < minArea) return;
    renderedMaxDepth = Math.max(renderedMaxDepth, node.depth);
    cells.push({
      id: node.data.id,
      path: node.data.path,
      name: node.data.name,
      depth: node.depth,
      category: node.data.category,
      sizeBytes: Math.max(0, node.data.sizeBytes),
      x: node.x0,
      y: node.y0,
      width: w,
      height: h,
      isSynthetic: node.data.synthetic,
      childCount: node.children?.length ?? 0,
    });
  });

  return { cells, totalBytes, maxDepth: renderedMaxDepth, empty: false };
}

/* ------------------------------------------------------------------ *
 * Shared helpers
 * ------------------------------------------------------------------ */

/** Indexes a tree by path for O(1) lookups during selection sync. */
export function indexTree(root: TreeNode): Map<string, TreeNode> {
  const index = new Map<string, TreeNode>();
  const stack: TreeNode[] = [root];
  while (stack.length > 0) {
    const node = stack.pop()!;
    index.set(node.path, node);
    for (const child of node.children) stack.push(child);
  }
  return index;
}

/** Finds the node at `path`, or `null`. */
export function findNode(root: TreeNode, path: string): TreeNode | null {
  if (root.path === path) return root;
  const stack: TreeNode[] = [...root.children];
  while (stack.length > 0) {
    const node = stack.pop()!;
    if (node.path === path) return node;
    for (const child of node.children) stack.push(child);
  }
  return null;
}

/** Ancestor chain from the root down to (and including) `path`. */
export function ancestorChain(root: TreeNode, path: string): TreeNode[] {
  const wanted = normalize(path);
  const chain: TreeNode[] = [];
  const walk = (node: TreeNode, trail: TreeNode[]): boolean => {
    const next = [...trail, node];
    if (node.path === path || normalize(node.path) === wanted) {
      chain.push(...next);
      return true;
    }
    for (const child of node.children) {
      if (walk(child, next)) return true;
    }
    return false;
  };
  walk(root, []);
  return chain;
}

function normalize(p: string): string {
  return p.replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase();
}

export type SortKey = 'size' | 'name' | 'modified' | 'count';

export function sortNodes(nodes: TreeNode[], key: SortKey, direction: 'asc' | 'desc' = 'desc'): TreeNode[] {
  const factor = direction === 'desc' ? -1 : 1;
  const sorted = [...nodes].sort((a, b) => {
    let diff = 0;
    switch (key) {
      case 'size':
        diff = a.sizeBytes - b.sizeBytes;
        break;
      case 'name':
        diff = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
        break;
      case 'modified':
        diff = (a.modifiedAt ?? 0) - (b.modifiedAt ?? 0);
        break;
      case 'count':
        diff = a.fileCount + a.folderCount - (b.fileCount + b.folderCount);
        break;
      default:
        diff = 0;
    }
    if (diff === 0) diff = a.name.localeCompare(b.name);
    return diff * factor;
  });
  return sorted;
}

export interface CategorySlice {
  category: StorageCategory;
  bytes: number;
  fraction: number;
  fileCount: number;
}

/** Aggregates a node's direct children by storage category (used for breakdown bars). */
export function categoryBreakdown(node: TreeNode, categories?: StorageCategory[]): CategorySlice[] {
  const allowed = new Set(categories);
  const totals = new Map<StorageCategory, { bytes: number; fileCount: number }>();
  for (const child of node.children) {
    if (allowed.size > 0 && !allowed.has(child.category)) continue;
    const entry = totals.get(child.category) ?? { bytes: 0, fileCount: 0 };
    entry.bytes += Math.max(0, child.sizeBytes);
    entry.fileCount += child.fileCount + (child.kind === 'file' ? 1 : 0);
    totals.set(child.category, entry);
  }
  const total = [...totals.values()].reduce((sum, v) => sum + v.bytes, 0);
  return [...totals.entries()]
    .map(([category, v]) => ({
      category,
      bytes: v.bytes,
      fraction: total > 0 ? v.bytes / total : 0,
      fileCount: v.fileCount,
    }))
    .sort((a, b) => b.bytes - a.bytes);
}

/** Largest children of a node, folders first then files, both by size. */
export function largestChildren(node: TreeNode, count = 8): TreeNode[] {
  return sortNodes(node.children, 'size', 'desc').slice(0, count);
}

/** Total size of a subtree, recomputed from children (defensive against stale caches). */
export function recomputeSize(node: TreeNode): number {
  if (node.kind === 'file') return Math.max(0, node.sizeBytes);
  let total = 0;
  for (const child of node.children) total += recomputeSize(child);
  return total;
}
