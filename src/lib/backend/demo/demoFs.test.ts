import { describe, expect, it } from 'vitest';
import { collectFiles, generateDemoMachine } from './demoFs';
import { recomputeSize } from '@/features/visualization/layout';
import type { TreeNode } from '@/types/contracts';

/**
 * The demo filesystem is the data source for the browser preview. These tests pin down the
 * invariants the UI relies on: determinism, exact aggregation, and realistic scale.
 */

function sumChildren(node: TreeNode): number {
  if (node.kind === 'file') return node.sizeBytes;
  return node.children.reduce((total, child) => total + sumChildren(child), 0);
}

function countNodes(node: TreeNode): { files: number; folders: number } {
  if (node.kind === 'file') return { files: 1, folders: 0 };
  let files = 0;
  let folders = 1;
  for (const child of node.children) {
    const sub = countNodes(child);
    files += sub.files;
    folders += sub.folders;
  }
  return { files, folders };
}

describe('generateDemoMachine', () => {
  const machine = generateDemoMachine(1234);

  it('is deterministic for a given seed', () => {
    const again = generateDemoMachine(1234);
    expect(again.root.sizeBytes).toBe(machine.root.sizeBytes);
    expect(again.stats).toEqual(machine.stats);
    expect(again.drive).toEqual(machine.drive);
    expect(again.walkOrder.length).toBe(machine.walkOrder.length);
  });

  it('produces a different machine for a different seed', () => {
    const other = generateDemoMachine(4321);
    expect(other.root.sizeBytes).not.toBe(machine.root.sizeBytes);
  });

  it('aggregates parent sizes from children exactly', () => {
    expect(machine.root.sizeBytes).toBe(sumChildren(machine.root));
    for (const child of machine.root.children) {
      expect(child.sizeBytes).toBe(sumChildren(child));
    }
  });

  it('keeps recomputeSize() in agreement with the stored size', () => {
    for (const child of machine.root.children) {
      expect(recomputeSize(child)).toBe(child.sizeBytes);
    }
  });

  it('reports file and folder counts that match the tree', () => {
    const counted = countNodes(machine.root);
    expect(counted.files).toBe(machine.stats.files);
    expect(counted.folders - 1).toBe(machine.stats.folders); // stats excludes the root
  });

  it('produces a plausible workstation-scale scan', () => {
    // Enough entries to exercise pruning and virtualization, small enough to be instant.
    expect(machine.stats.files).toBeGreaterThan(20_000);
    expect(machine.stats.folders).toBeGreaterThan(800);
    expect(machine.drive.totalBytes).toBeGreaterThan(machine.drive.usedBytes);
    expect(machine.drive.freeBytes).toBe(machine.drive.totalBytes - machine.drive.usedBytes);
  });

  it('marks protected system locations as access denied', () => {
    const protectedFolder = machine.root.children.find((c) => c.name === 'System Volume Information');
    expect(protectedFolder).toBeDefined();
    expect(protectedFolder?.accessDenied).toBe(true);
    expect(protectedFolder?.children).toHaveLength(0);
  });

  it('includes large files, zero-byte files and a spread of modification dates', () => {
    const files = collectFiles(machine.root);
    const largest = files.reduce((max, f) => Math.max(max, f.sizeBytes), 0);
    const zeroByte = files.filter((f) => f.sizeBytes === 0).length;
    expect(largest).toBeGreaterThan(1024 * 1024 * 1024);
    expect(zeroByte).toBeGreaterThan(50);
    const dates = files.map((f) => f.modifiedAt ?? 0);
    expect(Math.max(...dates) - Math.min(...dates)).toBeGreaterThan(365 * 86_400_000);
  });

  it('assigns every file a category and a stable path-derived id', () => {
    const files = collectFiles(machine.root);
    const ids = new Set(files.map((f) => f.id));
    expect(ids.size).toBe(files.length);
    for (const file of files.slice(0, 500)) {
      expect(file.category).toBeTruthy();
      expect(file.extension).not.toBeUndefined();
      expect(file.path.startsWith('C:\\')).toBe(true);
    }
  });

  it('orders the walk so folders precede their children (pre-order)', () => {
    const positions = new Map(machine.walkOrder.map((node, index) => [node.path, index]));
    for (const node of machine.walkOrder) {
      for (const child of node.children) {
        expect(positions.get(child.path)!).toBeGreaterThan(positions.get(node.path)!);
      }
    }
  });
});
