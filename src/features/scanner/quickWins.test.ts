import { describe, expect, it } from 'vitest';
import { computeQuickWins, totalQuickWinBytes } from './quickWins';
import { generateDemoMachine } from '@/lib/backend/demo/demoFs';
import type { TreeNode } from '@/types/contracts';
import { stablePathId } from '@/lib/id';

function file(path: string, sizeBytes: number, extension: string | null): TreeNode {
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
    extension,
    category: 'other',
    accessDenied: false,
    children: [],
    synthetic: false,
  };
}

function folder(path: string, children: TreeNode[]): TreeNode {
  return {
    id: stablePathId(path),
    path,
    name: path.split('\\').pop()!,
    kind: 'folder',
    sizeBytes: children.reduce((sum, child) => sum + child.sizeBytes, 0),
    diskSizeBytes: null,
    fileCount: children.reduce((sum, child) => sum + child.fileCount, 0),
    folderCount: children.reduce((sum, child) => sum + child.folderCount + (child.kind === 'folder' ? 1 : 0), 0),
    modifiedAt: 1_700_000_000_000,
    createdAt: 1_600_000_000_000,
    extension: null,
    category: 'other',
    accessDenied: false,
    children,
    synthetic: false,
  };
}

const TREE = folder('C:\\', [
  folder('C:\\Users\\alex', [
    folder('C:\\Users\\alex\\Downloads', [
      file('C:\\Users\\alex\\Downloads\\setup.exe', 200 * 1024 * 1024, 'exe'),
      file('C:\\Users\\alex\\Downloads\\notes.pdf', 1024, 'pdf'),
    ]),
    folder('C:\\Users\\alex\\AppData\\Local\\Temp', [
      file('C:\\Users\\alex\\AppData\\Local\\Temp\\scratch.tmp', 50 * 1024 * 1024, 'tmp'),
    ]),
    folder('C:\\Users\\alex\\AppData\\Local\\Google\\Chrome\\Cache', [
      file('C:\\Users\\alex\\AppData\\Local\\Google\\Chrome\\Cache\\data.cache', 30 * 1024 * 1024, 'cache'),
    ]),
    folder('C:\\Users\\alex\\Videos', [
      file('C:\\Users\\alex\\Videos\\holiday.mp4', 2 * 1024 * 1024 * 1024, 'mp4'),
      file('C:\\Users\\alex\\Videos\\clip.mov', 150 * 1024 * 1024, 'mov'),
    ]),
    folder('C:\\Users\\alex\\Projects\\app\\node_modules', [
      file('C:\\Users\\alex\\Projects\\app\\node_modules\\index.js', 10 * 1024 * 1024, 'js'),
    ]),
  ]),
  folder('C:\\$Recycle.Bin', [file('C:\\$Recycle.Bin\\old.bin', 40 * 1024 * 1024, 'bin')]),
]);

describe('computeQuickWins', () => {
  const wins = computeQuickWins(TREE);
  const byId = new Map(wins.map((win) => [win.id, win]));

  it('finds Downloads from a real folder', () => {
    const downloads = byId.get('downloads')!;
    expect(downloads).toBeDefined();
    expect(downloads.bytes).toBe(200 * 1024 * 1024 + 1024);
    expect(downloads.fileCount).toBe(2);
    expect(downloads.path).toBe('C:\\Users\\alex\\Downloads');
  });

  it('finds temporary folders by name and by path', () => {
    expect(byId.get('temp')?.bytes).toBe(50 * 1024 * 1024);
  });

  it('finds application caches by folder name', () => {
    expect(byId.get('caches')?.bytes).toBe(30 * 1024 * 1024);
  });

  it('reports large media as an observation, not junk', () => {
    const media = byId.get('large-media')!;
    expect(media.bytes).toBe(2 * 1024 * 1024 * 1024 + 150 * 1024 * 1024);
    expect(media.fileCount).toBe(2);
    expect(media.hint.toLowerCase()).toContain('never');
  });

  it('finds development artifacts', () => {
    expect(byId.get('development')?.bytes).toBe(10 * 1024 * 1024);
  });

  it('finds the Recycle Bin', () => {
    expect(byId.get('recycle-bin')?.bytes).toBe(40 * 1024 * 1024);
  });

  it('omits categories that are not present', () => {
    const empty = computeQuickWins(folder('C:\\empty', []));
    expect(empty).toHaveLength(0);
  });

  it('sorts by size, largest first', () => {
    const bytes = wins.map((win) => win.bytes);
    expect([...bytes].sort((a, b) => b - a)).toEqual(bytes);
  });

  it('totals without double counting', () => {
    expect(totalQuickWinBytes(wins)).toBe(
      wins.reduce((sum, win) => sum + win.bytes, 0),
    );
  });

  it('never double counts when two wins share a path', () => {
    const duplicated = [...wins, { ...wins[0] }];
    expect(totalQuickWinBytes(duplicated)).toBe(totalQuickWinBytes(wins));
  });

  it('works on a realistic scan tree', () => {
    const machine = generateDemoMachine(99);
    const realWins = computeQuickWins(machine.root);
    expect(realWins.length).toBeGreaterThan(3);
    for (const win of realWins) {
      expect(win.bytes).toBeGreaterThan(0);
      expect(win.label.length).toBeGreaterThan(0);
      expect(win.hint.length).toBeGreaterThan(0);
    }
    // The totals must never exceed the drive contents.
    expect(totalQuickWinBytes(realWins)).toBeLessThanOrEqual(machine.root.sizeBytes);
  });
});
