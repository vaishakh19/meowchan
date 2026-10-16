import { describe, expect, it } from 'vitest';
import {
  ancestorChain,
  arcAtPoint,
  buildSunburstLayout,
  buildTreemapLayout,
  categoryBreakdown,
  findNode,
  indexTree,
  largestChildren,
  recomputeSize,
  sortNodes,
  sunburstArcPath,
} from './layout';
import type { TreeNode } from '@/types/contracts';
import { stablePathId } from '@/lib/id';

function makeFile(path: string, sizeBytes: number, category: TreeNode['category'] = 'other'): TreeNode {
  return {
    id: stablePathId(path),
    path,
    name: path.split('\\').pop()!,
    kind: 'file',
    sizeBytes,
    diskSizeBytes: sizeBytes,
    fileCount: 1,
    folderCount: 0,
    modifiedAt: 1_700_000_000_000,
    createdAt: 1_600_000_000_000,
    extension: null,
    category,
    accessDenied: false,
    children: [],
    synthetic: false,
  };
}

function makeFolder(path: string, children: TreeNode[], category: TreeNode['category'] = 'other'): TreeNode {
  return {
    id: stablePathId(path),
    path,
    name: path === 'C:\\' ? 'C:' : path.split('\\').pop()!,
    kind: 'folder',
    sizeBytes: children.reduce((sum, child) => sum + child.sizeBytes, 0),
    diskSizeBytes: null,
    fileCount: children.reduce((sum, child) => sum + child.fileCount, 0),
    folderCount: children.reduce((sum, child) => sum + child.folderCount + (child.kind === 'folder' ? 1 : 0), 0),
    modifiedAt: 1_700_000_000_000,
    createdAt: 1_600_000_000_000,
    extension: null,
    category,
    accessDenied: false,
    children,
    synthetic: false,
  };
}

const TREE = makeFolder('C:\\', [
  makeFolder('C:\\Users', [
    makeFile('C:\\Users\\a.mp4', 600, 'media'),
    makeFile('C:\\Users\\b.docx', 300, 'documents'),
    makeFolder('C:\\Users\\sub', [makeFile('C:\\Users\\sub\\c.dll', 100, 'applications')]),
  ]),
  makeFolder('C:\\Windows', [
    makeFile('C:\\Windows\\kernel32.dll', 800, 'system'),
    makeFile('C:\\Windows\\empty.txt', 0, 'documents'),
  ]),
]);

describe('buildSunburstLayout', () => {
  it('reports an empty layout for a zero-byte root', () => {
    const layout = buildSunburstLayout(makeFolder('C:\\empty', []));
    expect(layout.empty).toBe(true);
    expect(layout.arcs).toHaveLength(0);
    expect(layout.totalBytes).toBe(0);
  });

  it('assigns proportional angles that sum to a full turn', () => {
    const layout = buildSunburstLayout(TREE);
    expect(layout.empty).toBe(false);
    expect(layout.totalBytes).toBe(1800);
    const ring1 = layout.arcs.filter((arc) => arc.depth === 1);
    expect(ring1).toHaveLength(2);
    const users = ring1.find((arc) => arc.name === 'Users')!;
    const windows = ring1.find((arc) => arc.name === 'Windows')!;
    // Users = 1000 of 1800, Windows = 800 of 1800.
    expect(users.fraction).toBeCloseTo(1000 / 1800, 6);
    expect(windows.fraction).toBeCloseTo(800 / 1800, 6);
    expect(users.endAngle - users.startAngle).toBeCloseTo((1000 / 1800) * Math.PI * 2, 6);
    expect(ring1.reduce((sum, arc) => sum + (arc.endAngle - arc.startAngle), 0)).toBeCloseTo(Math.PI * 2, 6);
  });

  it('places the root at the centre and deeper nodes further out', () => {
    const layout = buildSunburstLayout(TREE);
    const deep = layout.arcs.filter((arc) => arc.depth === 2);
    expect(deep.length).toBeGreaterThan(0);
    for (const arc of layout.arcs) {
      expect(arc.innerRadius).toBeGreaterThan(0);
      expect(arc.outerRadius).toBeGreaterThan(arc.innerRadius);
    }
    const ring1 = layout.arcs.filter((arc) => arc.depth === 1);
    const ring2 = layout.arcs.filter((arc) => arc.depth === 2);
    expect(Math.max(...ring2.map((a) => a.outerRadius))).toBeGreaterThan(
      Math.max(...ring1.map((a) => a.outerRadius)),
    );
  });

  it('ignores zero-byte files rather than dividing by zero', () => {
    const layout = buildSunburstLayout(TREE);
    const empty = layout.arcs.find((arc) => arc.name === 'empty.txt');
    // A zero-byte file contributes no angle, so it is not rendered.
    expect(empty).toBeUndefined();
    expect(layout.collapsedArcs).toBeGreaterThan(0);
    expect(layout.arcs.reduce((sum, arc) => sum + arc.sizeBytes, 0)).toBeGreaterThan(0);
  });

  it('always renders the first two rings even when thin', () => {
    const manySmall = Array.from({ length: 400 }, (_, i) => makeFile(`C:\\f${i}.bin`, 1));
    const tree = makeFolder('C:\\', manySmall);
    const layout = buildSunburstLayout(tree, { minAngleDeg: 1 });
    const ring1 = layout.arcs.filter((arc) => arc.depth === 1);
    expect(ring1).toHaveLength(400);
  });

  it('respects maxVisibleArcs', () => {
    const many = Array.from({ length: 4000 }, (_, i) => makeFile(`C:\\f${i}.bin`, (4000 - i) * 100));
    const tree = makeFolder('C:\\', many);
    const layout = buildSunburstLayout(tree, { maxVisibleArcs: 200, minAngleDeg: 0.01 });
    expect(layout.arcs.length).toBeLessThanOrEqual(200);
    expect(layout.arcs.length).toBeGreaterThan(0);
    expect(layout.collapsedArcs).toBeGreaterThan(0);
  });

  it('respects maxDepth', () => {
    const layout = buildSunburstLayout(TREE, { maxDepth: 1 });
    expect(layout.arcs.every((arc) => arc.depth <= 1)).toBe(true);
  });

  it('links each arc to its parent', () => {
    const layout = buildSunburstLayout(TREE);
    const child = layout.arcs.find((arc) => arc.name === 'sub')!;
    expect(child.parentPath).toBe('C:\\Users');
    const top = layout.arcs.find((arc) => arc.name === 'Users')!;
    expect(top.parentPath).toBe('C:\\');
  });

  it('keeps categories so colour mapping is stable', () => {
    const layout = buildSunburstLayout(TREE);
    const media = layout.arcs.find((arc) => arc.name === 'a.mp4')!;
    expect(media.category).toBe('media');
  });
});

describe('sunburstArcPath', () => {
  it('produces a closed arc path', () => {
    const layout = buildSunburstLayout(TREE);
    const arc = layout.arcs[0];
    const path = sunburstArcPath(arc);
    expect(path.startsWith('M')).toBe(true);
    expect(path.endsWith('Z')).toBe(true);
    expect(path).toContain('A');
  });

  it('returns an empty string when the gap swallows the arc', () => {
    const layout = buildSunburstLayout(TREE);
    const arc = layout.arcs[0];
    expect(sunburstArcPath({ ...arc, startAngle: 0.5, endAngle: 0.5 }, 1)).toBe('');
    expect(sunburstArcPath({ ...arc, startAngle: 0, endAngle: 0.0001 }, 10)).toBe('');
  });
});

describe('arcAtPoint', () => {
  it('finds the arc under a polar coordinate', () => {
    const layout = buildSunburstLayout(TREE);
    const users = layout.arcs.find((arc) => arc.name === 'Users')!;
    const midAngle = (users.startAngle + users.endAngle) / 2;
    const midRadius = (users.innerRadius + users.outerRadius) / 2;
    const hit = arcAtPoint(layout.arcs, midRadius, midAngle);
    expect(hit?.path).toBe('C:\\Users');
  });

  it('returns null outside the rings', () => {
    const layout = buildSunburstLayout(TREE);
    expect(arcAtPoint(layout.arcs, 500, 0.5)).toBeNull();
    expect(arcAtPoint(layout.arcs, 1, 0.5)).toBeNull();
  });
});

describe('buildTreemapLayout', () => {
  it('lays out cells that fill the box', () => {
    const layout = buildTreemapLayout(TREE, { width: 800, height: 600 });
    expect(layout.empty).toBe(false);
    expect(layout.cells.length).toBeGreaterThan(0);
    // Every cell stays inside the box.
    for (const cell of layout.cells) {
      expect(cell.x).toBeGreaterThanOrEqual(0);
      expect(cell.y).toBeGreaterThanOrEqual(0);
      expect(cell.x + cell.width).toBeLessThanOrEqual(800 + 0.001);
      expect(cell.y + cell.height).toBeLessThanOrEqual(600 + 0.001);
    }
    // The first ring alone covers essentially the whole box.
    const ring1 = layout.cells.filter((cell) => cell.depth === 1);
    const ring1Area = ring1.reduce((sum, cell) => sum + cell.width * cell.height, 0);
    expect(ring1Area).toBeGreaterThan(800 * 600 * 0.9);
    expect(ring1Area).toBeLessThanOrEqual(800 * 600 + 0.001);
  });

  it('gives bigger files bigger cells', () => {
    const tree = makeFolder('C:\\', [
      makeFile('C:\\big.bin', 900),
      makeFile('C:\\small.bin', 100),
    ]);
    const layout = buildTreemapLayout(tree, { width: 400, height: 400 });
    const big = layout.cells.find((c) => c.name === 'big.bin')!;
    const small = layout.cells.find((c) => c.name === 'small.bin')!;
    expect(big.width * big.height).toBeGreaterThan(small.width * small.height);
  });

  it('returns an empty layout for a zero-byte root', () => {
    const layout = buildTreemapLayout(makeFolder('C:\\empty', []), { width: 100, height: 100 });
    expect(layout.empty).toBe(true);
    expect(layout.cells).toHaveLength(0);
  });

  it('skips cells below the minimum area', () => {
    const tree = makeFolder('C:\\', [
      makeFile('C:\\big.bin', 999_999),
      makeFile('C:\\tiny.bin', 1),
    ]);
    const layout = buildTreemapLayout(tree, { width: 400, height: 400, minArea: 5000 });
    expect(layout.cells.some((c) => c.name === 'tiny.bin')).toBe(false);
    expect(layout.cells.some((c) => c.name === 'big.bin')).toBe(true);
  });
});

describe('tree helpers', () => {
  it('indexes and finds nodes', () => {
    const index = indexTree(TREE);
    expect(index.size).toBe(9);
    expect(findNode(TREE, 'C:\\Users\\sub')?.name).toBe('sub');
    expect(findNode(TREE, 'C:\\nope')).toBeNull();
  });

  it('builds an ancestor chain', () => {
    const chain = ancestorChain(TREE, 'C:\\Users\\sub\\c.dll');
    expect(chain.map((node) => node.name)).toEqual(['C:', 'Users', 'sub', 'c.dll']);
    expect(ancestorChain(TREE, 'C:\\nope')).toHaveLength(0);
  });

  it('sorts by size, name and modified date', () => {
    const children = TREE.children[0].children;
    expect(sortNodes(children, 'size')[0].name).toBe('a.mp4');
    expect(sortNodes(children, 'size', 'asc')[0].name).toBe('sub');
    expect(sortNodes(children, 'name', 'asc')[0].name).toBe('a.mp4');
    expect(sortNodes(children, 'count')[0].name).toBe('sub');
  });

  it('aggregates a category breakdown', () => {
    const slices = categoryBreakdown(TREE.children[1]);
    expect(slices[0].category).toBe('system');
    expect(slices[0].bytes).toBe(800);
    expect(slices.reduce((sum, slice) => sum + slice.bytes, 0)).toBe(800);
    expect(slices.reduce((sum, slice) => sum + slice.fraction, 0)).toBeCloseTo(1, 6);
  });

  it('filters the breakdown by category', () => {
    const slices = categoryBreakdown(TREE.children[0], ['media']);
    expect(slices).toHaveLength(1);
    expect(slices[0].bytes).toBe(600);
  });

  it('returns the largest children', () => {
    const largest = largestChildren(TREE, 2);
    expect(largest.map((node) => node.name)).toEqual(['Users', 'Windows']);
  });

  it('recomputes sizes from children', () => {
    expect(recomputeSize(TREE)).toBe(1800);
    const stale = { ...TREE, sizeBytes: 1 };
    expect(recomputeSize(stale)).toBe(1800);
  });
});
