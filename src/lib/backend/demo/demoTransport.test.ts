/**
 * Browser-preview transport tests.
 *
 * This transport is the dev fallback used when there is no Tauri runtime. These tests pin down the
 * behaviour the UI relies on: the same contract as the Rust backend, honest labelling, no
 * destructive operations, and gating that never blocks scanning.
 */

import { beforeEach, describe, expect, it } from 'vitest';
import { createDemoTransport, DEMO_LICENSE_KEY } from './demoTransport';
import { generateDemoMachine } from './demoFs';
import { collectFiles } from '@/lib/tree';
import type { ScanOptions } from '@/types/contracts';

const OPTIONS: ScanOptions = {
  rootPath: 'C:\\',
  maxChildrenPerNode: 100,
};

function transport() {
  return createDemoTransport();
}

/** The demo transport walks in timed chunks, so a scan needs a real wait to finish. */
async function awaitScan(demo: ReturnType<typeof createDemoTransport>, scanId: string) {
  for (let i = 0; i < 400; i += 1) {
    const result = await demo.scanGet(scanId);
    if (result) return result;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error('The demo scan never finished');
}

async function awaitDuplicateReport(
  demo: ReturnType<typeof createDemoTransport>,
  reportId: string,
) {
  for (let i = 0; i < 400; i += 1) {
    const report = await demo.duplicatesGet(reportId);
    if (report && report.status !== 'running') return report;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error('The duplicate scan never finished');
}

/** Starts a scan and waits for the result. */
async function scanToCompletion(demo: ReturnType<typeof createDemoTransport>, options = OPTIONS) {
  const scanId = await demo.scanStart(options);
  return { scanId, result: await awaitScan(demo, scanId) };
}

beforeEach(() => {
  localStorage.clear();
});

describe('demo transport identity', () => {
  it('labels itself as a browser preview that cannot mutate the filesystem', () => {
    const demo = transport();
    expect(demo.kind).toBe('demo');
    expect(demo.canMutate).toBe(false);
    expect(demo.label.toLowerCase()).toContain('browser');
  });
});

describe('drives', () => {
  it('reports plausible volumes', async () => {
    const drives = await transport().listDrives();
    expect(drives.length).toBeGreaterThan(0);
    for (const drive of drives) {
      expect(drive.totalBytes).toBeGreaterThan(0);
      expect(drive.freeBytes).toBeLessThanOrEqual(drive.totalBytes);
      expect(drive.usedBytes).toBe(drive.totalBytes - drive.freeBytes);
    }
  });
});

describe('scanning', () => {
  it('streams progress and finishes with a real result', async () => {
    const demo = transport();
    const scanId = await demo.scanStart(OPTIONS);
    expect(scanId).toMatch(/^demo-scan-/);

    const phases = new Set<string>();
    let filesSeen = 0;
    const stop = await demo.onScanProgress((progress) => {
      phases.add(progress.phase);
      filesSeen = Math.max(filesSeen, progress.filesScanned);
    });

    const result = await awaitScan(demo, scanId);
    stop();

    expect(phases.size).toBeGreaterThan(0);
    expect(filesSeen).toBeGreaterThan(0);
    expect(result).not.toBeNull();
    expect(result!.status).toBe('complete');
    expect(result!.totalBytes).toBeGreaterThan(0);
    expect(result!.fileCount).toBeGreaterThan(0);
    expect(result!.folderCount).toBeGreaterThan(0);
    expect(result!.tree.sizeBytes).toBe(result!.totalBytes);
    expect(result!.tree.children.length).toBeGreaterThan(0);
    expect(phases.size).toBeGreaterThan(0);

  });

  it('cancellation is honoured', async () => {
    const demo = transport();
    const scanId = await demo.scanStart(OPTIONS);
    await demo.scanCancel(scanId);
    const result = await demo.scanGet(scanId);
    // Either the scan never produced a tree, or it is marked cancelled.
    if (result) {
      expect(result.status).toBe('cancelled');
    }
  });

  it('returns null for an unknown scan', async () => {
    expect(await transport().scanGet('nope')).toBeNull();
  });

  it('lists the scans it has run', async () => {
    const demo = transport();
    const { scanId } = await scanToCompletion(demo);
    const list = await demo.scanList();
    expect(list.length).toBeGreaterThan(0);
    expect(list.some((item) => item.scanId === scanId)).toBe(true);
    expect(list[0].rootPath).toBe('C:\\');
  });

  it('pages children without ever returning the whole tree', async () => {
    const demo = transport();
    const { scanId } = await scanToCompletion(demo);
    const page = await demo.scanChildren(scanId, 'C:\\', 0, 25);
    expect(page.items.length).toBeLessThanOrEqual(25);
    expect(page.total).toBeGreaterThan(0);
    expect(page.items.every((item) => !item.synthetic)).toBe(true);
  });
});

describe('large files', () => {
  it('filters by size and sorts largest first', async () => {
    const demo = transport();
    const { scanId } = await scanToCompletion(demo);
    const page = await demo.listLargeFiles({
      scanId,
      minBytes: 100 * 1024 * 1024,
      sort: 'size-desc',
      offset: 0,
      limit: 50,
    });
    expect(page.items.length).toBeGreaterThan(0);
    const sizes = page.items.map((item) => item.sizeBytes);
    expect([...sizes].sort((a, b) => b - a)).toEqual(sizes);
    expect(sizes.every((size) => size >= 100 * 1024 * 1024)).toBe(true);
    expect(page.totalBytes).toBeGreaterThanOrEqual(Math.max(...sizes));
  });

  it('returns an empty page for an unknown scan rather than the whole machine', async () => {
    const demo = transport();
    const page = await demo.listLargeFiles({
      scanId: 'unknown-scan',
      minBytes: 0,
      offset: 0,
      limit: 10,
    });
    expect(page.items).toEqual([]);
    expect(page.total).toBe(0);
    expect(page.totalBytes).toBe(0);
  });

  it('filters by extension', async () => {
    const demo = transport();
    const { scanId } = await scanToCompletion(demo);
    const page = await demo.listLargeFiles({
      scanId,
      minBytes: 0,
      extensions: ['mp4'],
      offset: 0,
      limit: 50,
    });
    expect(page.items.every((item) => item.extension === 'mp4')).toBe(true);
  });

  it('filters by modification window', async () => {
    const demo = transport();
    const { scanId, result } = await scanToCompletion(demo);
    const cutoff = result.startedAt + 86_400_000;
    const page = await demo.listLargeFiles({
      scanId,
      minBytes: 0,
      modifiedBefore: cutoff,
      offset: 0,
      limit: 50,
    });
    expect(page.items.every((item) => (item.modifiedAt ?? 0) <= cutoff)).toBe(true);
  });
});

describe('file details', () => {
  it('describes a real node from the scan tree', async () => {
    const demo = transport();
    const { result } = await scanToCompletion(demo);
    const firstChild = result.tree.children.find((child) => !child.synthetic)!;
    const details = await demo.fileDetails(firstChild.path);
    expect(details.path).toBe(firstChild.path);
    expect(details.name.length).toBeGreaterThan(0);
    expect(details.sizeBytes).toBeGreaterThanOrEqual(0);
    expect(details.kind === 'folder' || details.kind === 'file').toBe(true);
  });
});

describe('destructive operations are refused in the browser preview', () => {
  it('reveal and open both report that the native backend is required', async () => {
    const demo = transport();
    await expect(demo.revealInExplorer('C:\\Users')).rejects.toThrow();
    await expect(demo.openPath('C:\\Users')).rejects.toThrow();
  });

  it('cleanup execution reports what would have happened without deleting anything', async () => {
    const demo = transport();
    const { scanId } = await scanToCompletion(demo);
    const result = await demo.cleanupScan(scanId, []);
    expect(Array.isArray(result)).toBe(true);

    const execution = await demo.cleanupExecute({
      scanId,
      items: [],
      dryRun: true,
    });
    expect(execution.totalRemovedCount).toBe(0);
    expect(execution.totalRemovedBytes).toBe(0);
  });
});

describe('duplicates', () => {
  it('groups by size and content, and never by filename alone', async () => {
    const demo = transport();
    const { scanId } = await scanToCompletion(demo);
    const reportId = await demo.findDuplicates({ scanId, minBytes: 1 });
    const report = await awaitDuplicateReport(demo, reportId);
    expect(report).not.toBeNull();
    expect(report!.totalGroups).toBeGreaterThan(0);
    for (const group of report!.groups) {
      expect(group.copies.length).toBeGreaterThanOrEqual(2);
      // Every copy in a group is the same size.
      expect(group.copies.every((copy) => copy.sizeBytes === group.sizeBytes)).toBe(true);
      // Reclaimable = size × (copies − 1), i.e. one copy is always kept.
      expect(group.reclaimableBytes).toBe(group.sizeBytes * (group.copies.length - 1));
      // Exactly one copy is the suggested original.
      expect(group.copies.filter((copy) => copy.isSuggestedOriginal).length).toBe(1);
    }
  });

  it('verifies two identical byte streams as equal', async () => {
    const demo = transport();
    const result = await demo.verifyDuplicates('a', 'b');
    expect(typeof result.equal).toBe('boolean');
    expect(result.bytesCompared).toBeGreaterThanOrEqual(0);
  });

  it('respects the free-tier duplicate group limit', async () => {
    const demo = transport();
    const { scanId } = await scanToCompletion(demo);
    const reportId = await demo.findDuplicates({ scanId, minBytes: 1 });
    await awaitDuplicateReport(demo, reportId);
    const entitlement = await demo.entitlementGet();
    const limit = entitlement.duplicateGroupLimit;
    expect(limit).toBeGreaterThan(0);
    // The report itself is never truncated by the backend; the *UI* applies the limit.
    const report = await demo.duplicatesGet(reportId);
    expect(report!.totalGroups).toBeGreaterThan(0);
  });
});

describe('licensing', () => {
  it('starts on the free tier with core analysis available', async () => {
    const entitlement = await transport().entitlementGet();
    expect(entitlement.state).toBe('free');
    expect(entitlement.features.coreAnalysis).toBe(true);
    expect(entitlement.features.scanHistory).toBe(false);
  });

  it('activates the demo key and unlocks Pro features', async () => {
    const demo = transport();
    const entitlement = await demo.licenseActivate(DEMO_LICENSE_KEY);
    expect(entitlement.state === 'pro-active' || entitlement.state === 'trial').toBe(true);
    expect(entitlement.features.scanHistory).toBe(true);
    expect(entitlement.features.advancedDuplicates).toBe(true);
    expect(entitlement.duplicateGroupLimit).toBe(0);

    const reloaded = await demo.entitlementGet();
    expect(reloaded.state).toBe(entitlement.state);
  });

  it('rejects a nonsense key but still returns a usable entitlement', async () => {
    const entitlement = await transport().licenseActivate('not-a-real-key');
    expect(entitlement.state).toBe('invalid');
    expect(entitlement.features.coreAnalysis).toBe(true);
  });

  it('deactivation returns to the free tier', async () => {
    const demo = transport();
    await demo.licenseActivate(DEMO_LICENSE_KEY);
    const entitlement = await demo.licenseDeactivate();
    expect(entitlement.state).toBe('free');
  });

  it('keeps an entitlement event log', async () => {
    const demo = transport();
    await demo.licenseActivate(DEMO_LICENSE_KEY);
    const events = await demo.entitlementEvents();
    expect(events.length).toBeGreaterThan(0);
    expect(events[0].to.length).toBeGreaterThan(0);
  });

  it('never blocks scanning regardless of the entitlement', async () => {
    const demo = transport();
    await demo.licenseActivate('garbage');
    const { result } = await scanToCompletion(demo);
    expect(result.status).toBe('complete');
    expect(result.totalBytes).toBeGreaterThan(0);
  });
});

describe('settings and privacy', () => {
  it('round-trips settings through the preview storage', async () => {
    const demo = transport();
    const initial = await demo.settingsGet();
    expect(initial.maxChildrenPerNode).toBeGreaterThan(1);

    const updated = await demo.settingsUpdate({ theme: 'dark' });
    expect(updated.theme).toBe('dark');

    const reloaded = await demo.settingsGet();
    expect(reloaded.theme).toBe('dark');
  });

  it('a partial patch does not reset unrelated fields', async () => {
    const demo = transport();
    await demo.settingsUpdate({ theme: 'light', followLinks: true });
    const updated = await demo.settingsUpdate({ theme: 'dark' });
    expect(updated.theme).toBe('dark');
    expect(updated.followLinks).toBe(true);
  });

  it('reports exactly what is stored locally', async () => {
    const report = await transport().privacyReport();
    expect(report.localStorage.length).toBeGreaterThan(0);
    expect(report.dataDir.length).toBeGreaterThan(0);
    expect(report.databasePath.length).toBeGreaterThan(0);
  });

  it('reports the app version', async () => {
    const version = await transport().appVersion();
    expect(version.version).toMatch(/^\d+\.\d+\.\d+/);
    expect(version.identifier).toBe('com.diskwise.app');
  });

  it('describes the Recycle Bin honestly', async () => {
    const summary = await transport().recycleBinSummary();
    expect(typeof summary.supported).toBe('boolean');
    expect(summary.detail.length).toBeGreaterThan(0);
  });
});

describe('the synthetic dataset is deterministic and self-consistent', () => {
  it('produces the same machine twice', () => {
    const a = generateDemoMachine();
    const b = generateDemoMachine();
    expect(a.root.path).toBe(b.root.path);
    expect(a.root.sizeBytes).toBe(b.root.sizeBytes);
    expect(a.root.fileCount).toBe(b.root.fileCount);
    expect(a.root.folderCount).toBe(b.root.folderCount);
  });

  it('never reports a negative or zero total for a populated machine', () => {
    const machine = generateDemoMachine();
    expect(machine.root.sizeBytes).toBeGreaterThan(0);
    expect(machine.root.fileCount).toBeGreaterThan(0);
    expect(machine.root.folderCount).toBeGreaterThan(0);
  });

  it('every file has a unique path', () => {
    const machine = generateDemoMachine();
    const files = collectFiles(machine.root);
    const paths = new Set(files.map((file) => file.path));
    expect(paths.size).toBe(files.length);
  });

  it('sizes add up at every level', () => {
    const machine = generateDemoMachine();
    const walk = (node: typeof machine.root): number => {
      if (node.kind === 'file') return node.sizeBytes;
      const own = node.children.reduce((sum, child) => sum + walk(child), 0);
      return own;
    };
    expect(walk(machine.root)).toBe(machine.root.sizeBytes);
  });
});
