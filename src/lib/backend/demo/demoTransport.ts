/**
 * Browser-preview transport.
 *
 * Used only when the UI is opened outside a Tauri webview (e.g. `npm run dev`, `vite preview`,
 * or the component tests). It serves a deterministic synthetic filesystem and simulates
 * asynchronous scanning so the whole interface — progress, cancellation, empty states, gating —
 * can be exercised without Windows.
 *
 * It is **not** part of the packaged application. `getTransport()` only selects it when
 * `window.__TAURI_INTERNALS__` is absent.
 */

import type {
  AppSettings,
  ChildrenPage,
  CleanupFinding,
  CleanupPlan,
  CleanupPreview,
  CleanupResult,
  CleanupRule,
  DriveInfo,
  DuplicateOptions,
  DuplicateProgress,
  DuplicateReport,
  DuplicateVerifyResult,
  Entitlement,
  EntitlementEvent,
  EntitlementState,
  FileDetails,
  LargeFilePage,
  LargeFileQuery,
  RecycleBinSummary,
  ScanOptions,
  ScanProgress,
  ScanResult,
  ScanSummary,
  TreeNode,
} from '@/types/contracts';
import type { BackendTransport } from '@/lib/transport';
import { collectFiles, collectFolders, DEMO_DRIVES, DEMO_NOW, generateDemoMachine, type DemoMachine } from './demoFs';
import { pruneTree } from '@/lib/tree';
import { stablePathId } from '@/lib/id';
import { sortNodes } from '@/features/visualization/layout';
import {
  applyDuplicateLimit,
  entitlementFromVerifiedLicense,
  freeEntitlement,
  FREE_DUPLICATE_GROUP_LIMIT,
} from '@/features/licensing/entitlements';

const DAY = 86_400_000;
const SETTINGS_KEY = 'diskwise.settings.v1';
const ENTITLEMENT_KEY = 'diskwise.entitlement.v1';

/** Demo license key accepted by the preview transport. Real keys are verified in Rust. */
export const DEMO_LICENSE_KEY = 'DWPRO-DEMO-2026-PRO';

interface DemoScan {
  scanId: string;
  options: ScanOptions;
  result: ScanResult | null;
  status: ScanResult['status'];
  startedAt: number;
  finishedAt: number | null;
  cancelled: boolean;
  timer: number | null;
  cursor: number;
  filesScanned: number;
  foldersScanned: number;
  bytesAnalyzed: number;
  errors: number;
}

export function createDemoTransport(): BackendTransport {
  let machine: DemoMachine | null = null;
  const scans = new Map<string, DemoScan>();
  const reports = new Map<string, DuplicateReport>();
  let events: EntitlementEvent[] = [];
  const scanListeners = new Set<(progress: ScanProgress) => void>();
  const duplicateListeners = new Set<(progress: DuplicateProgress) => void>();
  let entitlement: Entitlement = loadEntitlement();
  let settings: AppSettings = loadSettings();

  function ensureMachine(): DemoMachine {
    if (!machine) machine = generateDemoMachine();
    return machine;
  }

  function emitScan(progress: ScanProgress): void {
    for (const listener of scanListeners) listener(progress);
  }

  function emitDuplicate(progress: DuplicateProgress): void {
    for (const listener of duplicateListeners) listener(progress);
  }

  /* ---------------------------------------------------------------- *
   * Drives
   * ---------------------------------------------------------------- */

  async function listDrives(): Promise<DriveInfo[]> {
    const m = ensureMachine();
    const drives: DriveInfo[] = DEMO_DRIVES.map((spec) => {
      const usedBytes = spec.id === 'C:' ? m.drive.usedBytes : Math.round(spec.totalBytes * spec.usedRatio);
      return {
        id: spec.id,
        name: spec.name,
        path: `${spec.id}\\`,
        kind: spec.kind,
        totalBytes: spec.totalBytes,
        freeBytes: spec.totalBytes - usedBytes,
        usedBytes,
        fileSystem: spec.fileSystem,
        isSystemDrive: spec.id === 'C:',
        isReadOnly: false,
        isDisconnected: false,
      };
    });
    return drives;
  }

  /* ---------------------------------------------------------------- *
   * Scanning
   * ---------------------------------------------------------------- */

  function scanStart(options: ScanOptions): Promise<string> {
    const scanId = `demo-scan-${scans.size + 1}-${Math.random().toString(36).slice(2, 8)}`;
    const scan: DemoScan = {
      scanId,
      options,
      result: null,
      status: 'running',
      startedAt: Date.now(),
      finishedAt: null,
      cancelled: false,
      timer: null,
      cursor: 0,
      filesScanned: 0,
      foldersScanned: 0,
      bytesAnalyzed: 0,
      errors: 0,
    };
    scans.set(scanId, scan);

    const m = ensureMachine();
    const order = m.walkOrder;
    const total = order.length;
    const expectedBytes = m.drive.usedBytes;
    const rootNode = findInMachine(m, options.rootPath) ?? m.root;
    const chunk = Math.max(400, Math.ceil(total / 40));

    const tick = () => {
      if (scan.cancelled) {
        finish('cancelled');
        return;
      }
      const end = Math.min(total, scan.cursor + chunk);
      for (let i = scan.cursor; i < end; i += 1) {
        const node = order[i];
        if (node.kind === 'file') {
          scan.filesScanned += 1;
          scan.bytesAnalyzed += node.sizeBytes;
        } else {
          scan.foldersScanned += 1;
          if (node.accessDenied) scan.errors += 1;
        }
      }
      scan.cursor = end;
      const elapsedMs = Date.now() - scan.startedAt;
      const fraction = expectedBytes > 0 ? scan.bytesAnalyzed / expectedBytes : 0;
      emitScan({
        scanId,
        phase: 'walking',
        status: 'running',
        filesScanned: scan.filesScanned,
        foldersScanned: scan.foldersScanned,
        bytesAnalyzed: scan.bytesAnalyzed,
        errors: scan.errors,
        currentPath: order[Math.min(end, total - 1)]?.path ?? options.rootPath,
        elapsedMs,
        etaMs: fraction > 0.05 ? Math.round((elapsedMs / fraction) * (1 - fraction)) : null,
        percent: fraction > 0 ? Math.min(99, fraction * 100) : null,
        truncated: false,
      });
      if (end >= total) {
        finish('complete');
      } else {
        scan.timer = setTimeout(tick, 55) as unknown as number;
      }
    };

    const finish = (status: ScanResult['status']) => {
      if (scan.timer !== null) clearTimeout(scan.timer);
      scan.timer = null;
      scan.status = status;
      scan.finishedAt = Date.now();
      const pruned = pruneTree(rootNode, {
        maxChildren: options.maxChildrenPerNode ?? 100,
        maxNodes: 150_000,
      });
      scan.result = {
        scanId,
        rootPath: rootNode.path,
        rootName: rootNode.name,
        startedAt: scan.startedAt,
        finishedAt: scan.finishedAt,
        status,
        tree: pruned.tree,
        totalBytes: rootNode.sizeBytes,
        diskTotalBytes: rootNode.diskSizeBytes,
        fileCount: rootNode.fileCount,
        folderCount: rootNode.folderCount,
        errors: collectErrors(rootNode),
        truncated: pruned.truncated,
        prunedNodes: pruned.prunedNodes,
        durationMs: scan.finishedAt - scan.startedAt,
        expectedBytes,
      };
      emitScan({
        scanId,
        phase: status === 'cancelled' ? 'cancelled' : 'done',
        status,
        filesScanned: scan.filesScanned,
        foldersScanned: scan.foldersScanned,
        bytesAnalyzed: scan.bytesAnalyzed,
        errors: scan.errors,
        currentPath: rootNode.path,
        elapsedMs: scan.finishedAt - scan.startedAt,
        etaMs: 0,
        percent: 100,
        truncated: pruned.truncated,
      });
    };

    scan.timer = setTimeout(tick, 120) as unknown as number;
    return Promise.resolve(scanId);
  }

  function collectErrors(root: TreeNode) {
    const out: ScanResult['errors'] = [];
    for (const folder of collectFolders(root)) {
      if (folder.accessDenied) {
        out.push({
          path: folder.path,
          kind: 'access-denied',
          message: 'Access is denied. (simulated)',
        });
      }
    }
    return out;
  }

  function findInMachine(m: DemoMachine, path: string): TreeNode | null {
    const wanted = path.replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase();
    const stack: TreeNode[] = [m.root];
    while (stack.length > 0) {
      const node = stack.pop()!;
      const candidate = node.path.replace(/\\+$/, '').toLowerCase();
      if (candidate === wanted || candidate === `${wanted}\\`) return node;
      for (const child of node.children) stack.push(child);
    }
    return null;
  }

  function scanGet(scanId: string): Promise<ScanResult | null> {
    const scan = scans.get(scanId);
    return Promise.resolve(scan?.result ?? null);
  }

  function scanCancel(scanId: string): Promise<void> {
    const scan = scans.get(scanId);
    if (scan && scan.status === 'running') scan.cancelled = true;
    return Promise.resolve();
  }

  function scanList(): Promise<ScanSummary[]> {
    return Promise.resolve(
      [...scans.values()]
        .filter((scan) => scan.result !== null)
        .map((scan) => ({
          scanId: scan.scanId,
          rootPath: scan.options.rootPath,
          rootName: scan.result?.rootName ?? scan.options.rootPath,
          startedAt: scan.startedAt,
          finishedAt: scan.finishedAt,
          durationMs: scan.finishedAt ? scan.finishedAt - scan.startedAt : null,
          status: scan.status,
          totalBytes: scan.result?.totalBytes ?? 0,
          fileCount: scan.result?.fileCount ?? 0,
          folderCount: scan.result?.folderCount ?? 0,
          errorCount: scan.result?.errors.length ?? 0,
          truncated: scan.result?.truncated ?? false,
          driveId: scan.options.rootPath.slice(0, 2),
          persisted: false,
        }))
        .sort((a, b) => b.startedAt - a.startedAt),
    );
  }

  function scanChildren(scanId: string, path: string, offset: number, limit: number): Promise<ChildrenPage> {
    const scan = scans.get(scanId);
    const root = scan?.result?.tree ?? ensureMachine().root;
    const node = findInTree(root, path) ?? root;
    const items = sortNodes(node.children, 'size', 'desc');
    return Promise.resolve({
      path: node.path,
      offset,
      limit,
      total: items.length,
      items: items.slice(offset, offset + limit),
    });
  }

  function scanDelete(scanId: string): Promise<void> {
    scans.delete(scanId);
    return Promise.resolve();
  }

  /* ---------------------------------------------------------------- *
   * Files
   * ---------------------------------------------------------------- */

  function listLargeFiles(query: LargeFileQuery): Promise<LargeFilePage> {
    const scan = scans.get(query.scanId);
    if (!scan || !scan.result) {
      // Matches the Rust backend: an unknown or unfinished scan is an empty result, never the
      // whole machine.
      return Promise.resolve({ total: 0, totalBytes: 0, offset: query.offset ?? 0, limit: query.limit ?? 200, items: [] });
    }
    const root = scan.result.tree;
    const extensions = new Set((query.extensions ?? []).map((e) => e.toLowerCase()));
    let files = collectFiles(root).filter((file) => file.sizeBytes >= query.minBytes);
    if (extensions.size > 0) {
      files = files.filter((file) => file.extension !== null && extensions.has(file.extension));
    }
    if (query.modifiedAfter !== null && query.modifiedAfter !== undefined) {
      files = files.filter((file) => (file.modifiedAt ?? 0) >= query.modifiedAfter!);
    }
    if (query.modifiedBefore !== null && query.modifiedBefore !== undefined) {
      files = files.filter((file) => (file.modifiedAt ?? 0) <= query.modifiedBefore!);
    }
    const sort = query.sort ?? 'size-desc';
    files = sortNodes(files, sort === 'name-asc' || sort === 'name-desc' ? 'name' : sort.startsWith('modified') ? 'modified' : 'size',
      sort.endsWith('asc') ? 'asc' : 'desc');
    const offset = query.offset ?? 0;
    const limit = query.limit ?? 200;
    return Promise.resolve({
      total: files.length,
      totalBytes: files.reduce((sum, file) => sum + file.sizeBytes, 0),
      offset,
      limit,
      items: files.slice(offset, offset + limit).map((file) => ({
        path: file.path,
        name: file.name,
        sizeBytes: file.sizeBytes,
        diskSizeBytes: file.diskSizeBytes,
        modifiedAt: file.modifiedAt,
        extension: file.extension,
        category: file.category,
      })),
    });
  }

  function fileDetails(path: string): Promise<FileDetails> {
    const m = ensureMachine();
    const node = findInMachine(m, path);
    if (!node) {
      return Promise.reject(new Error(`Path not found in the preview dataset: ${path}`));
    }
    return Promise.resolve({
      path: node.path,
      name: node.name,
      kind: node.kind,
      sizeBytes: node.sizeBytes,
      diskSizeBytes: node.diskSizeBytes,
      fileCount: node.fileCount,
      folderCount: node.folderCount,
      createdAt: node.createdAt,
      modifiedAt: node.modifiedAt,
      accessedAt: node.modifiedAt,
      extension: node.extension,
      category: node.category,
      isSymlink: false,
      isReadOnly: false,
      isHidden: node.name.startsWith('.'),
      attributes: node.accessDenied ? ['Access denied'] : [],
      largestChildren: sortNodes(node.children, 'size', 'desc').slice(0, 8),
      parentPath: node.path.slice(0, Math.max(node.path.lastIndexOf('\\'), 0)) || null,
      exists: true,
    });
  }

  function unavailable(action: string): Promise<void> {
    return Promise.reject(
      new Error(`${action} needs the Diskwise desktop app. This is the browser preview (synthetic data).`),
    );
  }

  /* ---------------------------------------------------------------- *
   * Duplicates
   * ---------------------------------------------------------------- */

  /**
   * Preview duplicate detection: group by exact size, then by a content-class derived from the
   * size and extension (the synthetic dataset has no file contents to hash). The packaged app
   * performs a real SHA-256 pass in Rust — see `src-tauri/src/duplicates/`.
   */
  function findDuplicates(options: DuplicateOptions): Promise<string> {
    const reportId = `demo-dup-${reports.size + 1}`;
    const scan = scans.get(options.scanId);
    const root = scan?.result?.tree ?? ensureMachine().root;
    const minBytes = options.minBytes ?? 1024;
    const maxBytes = options.maxBytes ?? 5 * 1024 * 1024 * 1024;
    const extensions = new Set((options.extensions ?? []).map((e) => e.toLowerCase()));

    const candidates = collectFiles(root).filter(
      (file) =>
        file.sizeBytes >= minBytes &&
        file.sizeBytes <= maxBytes &&
        (extensions.size === 0 || (file.extension !== null && extensions.has(file.extension))) &&
        (options.excludePaths ?? []).every((prefix) => !file.path.startsWith(prefix)),
    );

    const startedAt = Date.now();
    const bySize = new Map<number, TreeNode[]>();
    for (const file of candidates) {
      const bucket = bySize.get(file.sizeBytes);
      if (bucket) bucket.push(file);
      else bySize.set(file.sizeBytes, [file]);
    }

    let candidateGroups = 0;
    const groups: DuplicateReport['groups'] = [];
    let filesHashed = 0;
    for (const [size, bucket] of bySize) {
      if (bucket.length < 2) continue;
      candidateGroups += 1;
      const byClass = new Map<string, TreeNode[]>();
      for (const file of bucket) {
        const key = `${size}:${file.extension ?? ''}:${stablePathId(file.path).slice(0, 4)}`;
        const list = byClass.get(key);
        if (list) list.push(file);
        else byClass.set(key, [file]);
      }
      for (const members of byClass.values()) {
        if (members.length < 2) continue;
        filesHashed += members.length;
        const sorted = sortNodes(members, 'name', 'asc');
        const reclaimable = size * (sorted.length - 1);
        groups.push({
          groupId: stablePathId(`${size}:${sorted[0].path}`),
          sizeBytes: size,
          reclaimableBytes: reclaimable,
          hash: `demo-${size.toString(16)}-${stablePathId(sorted[0].path).slice(0, 12)}`,
          confidence: 'verified',
          copies: sorted.map((file, index) => ({
            path: file.path,
            name: file.name,
            sizeBytes: file.sizeBytes,
            modifiedAt: file.modifiedAt,
            byteVerified: true,
            isSuggestedOriginal: index === 0,
          })),
        });
      }
    }

    groups.sort((a, b) => b.reclaimableBytes - a.reclaimableBytes);
    const limit = entitlement.duplicateGroupLimit;
    const { groups: visible, limited } = applyDuplicateLimit(groups, limit);

    const report: DuplicateReport = {
      reportId,
      scanId: options.scanId,
      startedAt,
      finishedAt: Date.now(),
      status: 'complete',
      algorithm: options.algorithm ?? 'sha256',
      groups: visible,
      totalGroups: groups.length,
      totalCopies: groups.reduce((sum, group) => sum + group.copies.length, 0),
      totalReclaimableBytes: groups.reduce((sum, group) => sum + group.reclaimableBytes, 0),
      hashedBytes: candidates.reduce((sum, file) => sum + file.sizeBytes, 0),
      filesHashed,
      errors: [],
    };
    reports.set(reportId, { ...report, groups });
    void limited;
    emitDuplicate({
      reportId,
      phase: 'done',
      filesHashed,
      bytesHashed: report.hashedBytes,
      candidateGroups,
      groupsFound: groups.length,
      elapsedMs: 0,
      etaMs: 0,
    });
    return Promise.resolve(reportId);
  }

  function duplicatesGet(reportId: string): Promise<DuplicateReport | null> {
    const full = reports.get(reportId);
    if (!full) return Promise.resolve(null);
    const { groups: visible, limited } = applyDuplicateLimit(full.groups, entitlement.duplicateGroupLimit);
    void limited;
    return Promise.resolve({ ...full, groups: visible });
  }

  function verifyDuplicates(pathA: string, pathB: string): Promise<DuplicateVerifyResult> {
    const m = ensureMachine();
    const a = findInMachine(m, pathA);
    const b = findInMachine(m, pathB);
    if (!a || !b) return Promise.resolve({ equal: false, bytesCompared: 0, firstDifferenceOffset: null });
    return Promise.resolve({
      equal: a.sizeBytes === b.sizeBytes,
      bytesCompared: Math.min(a.sizeBytes, b.sizeBytes),
      firstDifferenceOffset: a.sizeBytes === b.sizeBytes ? null : Math.min(a.sizeBytes, b.sizeBytes),
    });
  }

  /* ---------------------------------------------------------------- *
   * Cleanup
   * ---------------------------------------------------------------- */

  function cleanupScan(scanId: string, rules: CleanupRule[]): Promise<CleanupFinding[]> {
    const scan = scans.get(scanId);
    const root = scan?.result?.tree ?? ensureMachine().root;
    const findings: CleanupFinding[] = [];
    const now = DEMO_NOW;

    const addFinding = (finding: CleanupFinding) => findings.push(finding);

    // 1. Recycle Bin
    const recycle = findInTree(root, 'C:\\$Recycle.Bin');
    if (recycle) {
      addFinding({
        id: 'recycle-bin',
        category: 'recycle-bin',
        risk: 'low',
        title: 'Recycle Bin',
        reason: 'Items you already deleted. Emptying the Recycle Bin is reversible only until you empty it.',
        protected: false,
        path: recycle.path,
        paths: [],
        fileCount: recycle.fileCount,
        folderCount: recycle.folderCount,
        sizeBytes: recycle.sizeBytes,
        diskSizeBytes: recycle.diskSizeBytes,
        oldestModifiedAt: null,
        newestModifiedAt: null,
        ruleId: null,
      });
    }

    // 2. Temp directories
    for (const path of settings.tempPaths.length > 0 ? settings.tempPaths : ['C:\\Users\\alex\\AppData\\Local\\Temp']) {
      const node = findInTree(root, path);
      if (!node) continue;
      addFinding({
        id: `temp-${stablePathId(path).slice(0, 8)}`,
        category: 'temp',
        risk: 'low',
        title: `Temporary files — ${node.name}`,
        reason: 'Files in a temporary location. Applications recreate what they need.',
        protected: false,
        path: node.path,
        paths: [],
        fileCount: node.fileCount,
        folderCount: node.folderCount,
        sizeBytes: node.sizeBytes,
        diskSizeBytes: node.diskSizeBytes,
        oldestModifiedAt: null,
        newestModifiedAt: null,
        ruleId: null,
      });
    }

    // 3. Cache directories
    const cachePaths = settings.cachePaths.length > 0
      ? settings.cachePaths
      : ['C:\\Users\\alex\\AppData\\Local\\Google\\Chrome', 'C:\\Users\\alex\\AppData\\Local\\npm-cache'];
    for (const path of cachePaths) {
      const node = findInTree(root, path);
      if (!node) continue;
      addFinding({
        id: `cache-${stablePathId(path).slice(0, 8)}`,
        category: 'cache',
        risk: 'low',
        title: `Application cache — ${node.name}`,
        reason: 'Regenerable cache data. The owning application will rebuild it on next launch.',
        protected: false,
        path: node.path,
        paths: [],
        fileCount: node.fileCount,
        folderCount: node.folderCount,
        sizeBytes: node.sizeBytes,
        diskSizeBytes: node.diskSizeBytes,
        oldestModifiedAt: null,
        newestModifiedAt: null,
        ruleId: null,
      });
    }

    // 4. Old downloads
    const downloads = findInTree(root, 'C:\\Users\\alex\\Downloads');
    if (downloads) {
      const files = collectFiles(downloads).filter((file) => (file.modifiedAt ?? now) < now - 90 * DAY);
      addFinding({
        id: 'old-downloads',
        category: 'downloads',
        risk: 'review',
        title: 'Downloads older than 90 days',
        reason: 'Installer and archive files in Downloads that have not been touched in 90 days. Review before removing.',
        protected: false,
        path: downloads.path,
        paths: [],
        fileCount: files.length,
        folderCount: 0,
        sizeBytes: files.reduce((sum, file) => sum + file.sizeBytes, 0),
        diskSizeBytes: files.reduce((sum, file) => sum + (file.diskSizeBytes ?? 0), 0),
        oldestModifiedAt: files.length > 0 ? Math.min(...files.map((f) => f.modifiedAt ?? now)) : null,
        newestModifiedAt: files.length > 0 ? Math.max(...files.map((f) => f.modifiedAt ?? now)) : null,
        ruleId: null,
      });
    }

    // 5. Large unused files
    const ageDays = settings.largeUnusedDays;
    const largeUnused = collectFiles(root).filter(
      (file) => file.sizeBytes > 200 * 1024 * 1024 && (file.modifiedAt ?? now) < now - ageDays * DAY,
    );
    largeUnused.sort((a, b) => b.sizeBytes - a.sizeBytes);
    addFinding({
      id: 'large-unused',
      category: 'large-unused',
      risk: 'review',
      title: `Large files not opened in ${ageDays} days`,
      reason: 'Big files that have not been modified recently. Diskwise never treats personal files as junk — review the list.',
      protected: false,
      path: null,
      paths: [],
      fileCount: largeUnused.length,
      folderCount: 0,
      sizeBytes: largeUnused.reduce((sum, file) => sum + file.sizeBytes, 0),
      diskSizeBytes: largeUnused.reduce((sum, file) => sum + (file.diskSizeBytes ?? 0), 0),
      oldestModifiedAt: largeUnused.length > 0 ? Math.min(...largeUnused.map((f) => f.modifiedAt ?? now)) : null,
      newestModifiedAt: largeUnused.length > 0 ? Math.max(...largeUnused.map((f) => f.modifiedAt ?? now)) : null,
      ruleId: null,
    });

    // 6. Development artifacts
    const devArtifacts = ['C:\\Users\\alex\\Projects\\diskwise\\node_modules', 'C:\\Users\\alex\\Projects\\legacy-app\\vendor'];
    for (const path of devArtifacts) {
      const node = findInTree(root, path);
      if (!node) continue;
      addFinding({
        id: `dev-${stablePathId(path).slice(0, 8)}`,
        category: 'development',
        risk: 'review',
        title: `Development artifacts — ${node.name}`,
        reason: 'Dependencies and build output that a package manager can reinstall. Review before removing.',
        protected: false,
        path: node.path,
        paths: [],
        fileCount: node.fileCount,
        folderCount: node.folderCount,
        sizeBytes: node.sizeBytes,
        diskSizeBytes: node.diskSizeBytes,
        oldestModifiedAt: null,
        newestModifiedAt: null,
        ruleId: null,
      });
    }

    // 7. Protected system locations — informational only.
    for (const folder of collectFolders(root)) {
      if (folder.accessDenied) {
        addFinding({
          id: `protected-${stablePathId(folder.path).slice(0, 8)}`,
          category: 'system',
          risk: 'informational',
          title: `Protected location — ${folder.name}`,
          reason: 'Diskwise cannot read this location and will never modify it.',
          protected: true,
          path: folder.path,
          paths: [],
          fileCount: 0,
          folderCount: 0,
          sizeBytes: 0,
          diskSizeBytes: null,
          oldestModifiedAt: null,
          newestModifiedAt: null,
          ruleId: null,
        });
      }
    }

    // 8. User-defined rules (Pro). Each rule only ever *proposes* items — nothing is removed
    //    without an explicit confirmation step.
    for (const rule of rules) {
      if (!rule.enabled) continue;
      const ruleRoot = findInTree(root, rule.path);
      if (!ruleRoot) continue;
      const cutoff = rule.olderThanDays > 0 ? now - rule.olderThanDays * DAY : 0;
      const extensions = new Set(rule.extensions.map((e) => e.toLowerCase()));
      const matches = collectFiles(ruleRoot).filter((file) => {
        if (rule.minBytes > 0 && file.sizeBytes < rule.minBytes) return false;
        if (rule.olderThanDays > 0 && (file.modifiedAt ?? now) > cutoff) return false;
        if (extensions.size > 0 && (file.extension === null || !extensions.has(file.extension))) return false;
        return true;
      });
      if (matches.length === 0) continue;
      matches.sort((a, b) => b.sizeBytes - a.sizeBytes);
      addFinding({
        id: `rule-${stablePathId(rule.id + rule.path).slice(0, 8)}`,
        category: rule.category,
        risk: rule.risk,
        title: rule.label,
        reason: `Your rule: ${rule.olderThanDays > 0 ? `older than ${rule.olderThanDays} days` : 'any age'}${
          rule.minBytes > 0 ? `, at least ${Math.round(rule.minBytes / (1024 * 1024))} MB` : ''
        }${extensions.size > 0 ? `, ${[...extensions].join(', ')}` : ''}.`,
        protected: false,
        path: rule.path,
        paths: [],
        fileCount: matches.length,
        folderCount: 0,
        sizeBytes: matches.reduce((sum, file) => sum + file.sizeBytes, 0),
        diskSizeBytes: matches.reduce((sum, file) => sum + (file.diskSizeBytes ?? 0), 0),
        oldestModifiedAt: Math.min(...matches.map((f) => f.modifiedAt ?? now)),
        newestModifiedAt: Math.max(...matches.map((f) => f.modifiedAt ?? now)),
        ruleId: rule.id,
      });
    }

    return Promise.resolve(findings);
  }

  function cleanupPreview(plan: CleanupPlan): Promise<CleanupPreview> {
    const items: CleanupPreview['items'] = [];
    const warnings: string[] = [];
    const root = ensureMachine().root;
    for (const item of plan.items) {
      const node = findInTree(root, item.paths[0] ?? '');
      if (!node) {
        warnings.push(`Not found in the preview dataset: ${item.paths[0] ?? '(unknown)'}`);
        continue;
      }
      items.push({
        findingId: item.findingId,
        title: node.name,
        action: item.action,
        pathCount: 1,
        fileCount: node.fileCount,
        folderCount: node.folderCount,
        sizeBytes: node.sizeBytes,
        paths: [{ path: node.path, status: 'ok', sizeBytes: node.sizeBytes, detail: 'Dry run (preview)' }],
      });
    }
    return Promise.resolve({
      items,
      totalSizeBytes: items.reduce((sum, item) => sum + item.sizeBytes, 0),
      totalPathCount: items.length,
      warnings,
    });
  }

  function cleanupExecute(plan: CleanupPlan): Promise<CleanupResult> {
    // The preview never touches a real filesystem. Report a successful simulation so the
    // confirmation → execution → refresh flow can be reviewed end to end.
    const items: CleanupResult['items'] = plan.items.map((item) => ({
      findingId: item.findingId,
      title: item.paths[0] ?? item.findingId,
      action: item.action,
      removedCount: item.paths.length,
      failedCount: 0,
      removedBytes: 0,
      results: item.paths.map((path) => ({ path, status: 'ok' as const, sizeBytes: 0, detail: 'Simulated (preview)' })),
    }));
    return Promise.resolve({
      items,
      totalRemovedCount: items.reduce((sum, item) => sum + item.removedCount, 0),
      totalFailedCount: 0,
      totalRemovedBytes: 0,
      completedAt: Date.now(),
      partial: false,
    });
  }

  function recycleBinSummary(): Promise<RecycleBinSummary> {
    const node = findInTree(ensureMachine().root, 'C:\\$Recycle.Bin');
    return Promise.resolve({
      itemCount: node?.fileCount ?? 0,
      sizeBytes: node?.sizeBytes ?? null,
      supported: true,
      detail: 'Simulated from the preview dataset.',
    });
  }

  /* ---------------------------------------------------------------- *
   * Licensing
   * ---------------------------------------------------------------- */

  function loadEntitlement(): Entitlement {
    try {
      const raw = localStorage.getItem(ENTITLEMENT_KEY);
      if (!raw) return freeEntitlement();
      const parsed = JSON.parse(raw) as { key: string; plan: 'pro' | 'trial'; days: number };
      return entitlementFromVerifiedLicense(
        {
          plan: parsed.plan,
          // The browser preview has no licence service to re-validate against, so it grants a
          // perpetual lease. This is explicitly a preview convenience and never the product path.
          permanent: true,
          leaseUntil: Date.now() + parsed.days * DAY,
          licenseId: parsed.key,
        },
      );
    } catch {
      return freeEntitlement();
    }
  }

  function persistEntitlement(next: Entitlement): void {
    entitlement = next;
    if (next.source === 'license-key' && next.state !== 'invalid') {
      try {
        localStorage.setItem(
          ENTITLEMENT_KEY,
          JSON.stringify({ key: next.licenseKeyMasked ?? 'demo', plan: next.tier === 'trial' ? 'trial' : 'pro', days: next.daysRemaining ?? 30 }),
        );
      } catch {
        /* storage unavailable — the entitlement simply does not persist */
      }
    } else {
      try {
        localStorage.removeItem(ENTITLEMENT_KEY);
      } catch {
        /* ignore */
      }
    }
  }

  /* ---------------------------------------------------------------- *
   * Settings
   * ---------------------------------------------------------------- */

  function defaultSettings(): AppSettings {
    return {
      theme: 'system',
      defaultScanRoot: 'C:\\',
      maxChildrenPerNode: 100,
      followLinks: false,
      confirmBeforeCleanup: true,
      preferRecycleBin: true,
      largeUnusedDays: 180,
      cachePaths: [],
      tempPaths: [],
      excludePaths: [],
      crashReporting: false,
      duplicateGroupLimit: FREE_DUPLICATE_GROUP_LIMIT,
    };
  }

  function loadSettings(): AppSettings {
    try {
      const raw = localStorage.getItem(SETTINGS_KEY);
      if (!raw) return defaultSettings();
      return { ...defaultSettings(), ...(JSON.parse(raw) as Partial<AppSettings>) };
    } catch {
      return defaultSettings();
    }
  }

  function persistSettings(): void {
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
    } catch {
      /* ignore */
    }
  }

  /* ---------------------------------------------------------------- *
   * Transport
   * ---------------------------------------------------------------- */

  return {
    kind: 'demo',
    label: 'Browser preview (synthetic data)',
    canMutate: false,

    listDrives,
    pickFolder: () => Promise.resolve(null),
    scanStart,
    scanCancel,
    scanGet,
    scanList,
    scanChildren,
    scanDelete,

    listLargeFiles,
    fileDetails,
    revealInExplorer: (path) => unavailable(`Reveal ${path}`),
    openPath: (path) => unavailable(`Open ${path}`),

    findDuplicates,
    duplicatesGet,
    verifyDuplicates,

    cleanupScan,
    cleanupPreview,
    cleanupExecute,
    recycleBinSummary,

    entitlementGet: () => Promise.resolve(entitlement),
    licenseActivate: (licenseKey: string) => {
      const trimmed = licenseKey.trim();
      if (trimmed.toUpperCase() !== DEMO_LICENSE_KEY) {
        const invalid: Entitlement = {
          ...freeEntitlement(),
          state: 'invalid',
          source: 'license-key',
          reason: 'This key is not valid for the preview build. The packaged app verifies keys with an Ed25519 signature in Rust.',
          licenseKeyMasked: mask(trimmed),
        };
        persistEntitlement(invalid);
        events = [
          {
            at: Date.now(),
            from: entitlement.state,
            to: 'invalid' as EntitlementState,
            reason: invalid.reason ?? 'Invalid key.',
          },
          ...events,
        ].slice(0, 50);
        return Promise.resolve(invalid);
      }
      const next = entitlementFromVerifiedLicense({
        plan: 'pro',
        permanent: true,
        leaseUntil: Date.now() + 365 * DAY,
        licenseId: 'demo',
      });
      persistEntitlement({ ...next, licenseKeyMasked: mask(trimmed) });
      events = [
        {
          at: Date.now(),
          from: entitlement.state,
          to: next.state,
          reason: 'Preview licence activated.',
        },
        ...events,
      ].slice(0, 50);
      return Promise.resolve({ ...next, licenseKeyMasked: mask(trimmed) });
    },
    licenseDeactivate: () => {
      const previous = entitlement;
      persistEntitlement(freeEntitlement());
      events = [
        {
          at: Date.now(),
          from: previous.state,
          to: 'free' as EntitlementState,
          reason: 'Licence deactivated.',
        },
        ...events,
      ].slice(0, 50);
      return Promise.resolve(entitlement);
    },
    entitlementEvents: () => Promise.resolve(events),

    settingsGet: () => Promise.resolve(settings),
    settingsUpdate: (patch: Partial<AppSettings>) => {
      settings = { ...settings, ...patch };
      persistSettings();
      return Promise.resolve(settings);
    },
    privacyReport: () =>
      Promise.resolve({
        localStorage: [
          'Scan results for the current session (in memory only).',
          'Your preferences and theme (browser localStorage).',
          'License entitlement cache (browser localStorage).',
        ],
        networkActivity: [],
        dataDir: '(not applicable in the browser preview)',
        databasePath: '(not applicable in the browser preview)',
      }),
    appVersion: () =>
      Promise.resolve({
        version: '0.1.0',
        identifier: 'com.diskwise.app',
        platform: 'browser-preview',
        arch: 'unknown',
      }),

    onScanProgress: (handler) => {
      scanListeners.add(handler);
      return Promise.resolve(() => scanListeners.delete(handler));
    },
    onDuplicateProgress: (handler) => {
      duplicateListeners.add(handler);
      return Promise.resolve(() => duplicateListeners.delete(handler));
    },
  };
}

function mask(key: string): string {
  const trimmed = key.trim();
  if (trimmed.length <= 8) return '*'.repeat(trimmed.length);
  return `${trimmed.slice(0, 6)}${'*'.repeat(Math.max(4, trimmed.length - 10))}${trimmed.slice(-4)}`;
}

function findInTree(root: TreeNode, path: string): TreeNode | null {
  const wanted = path.replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase();
  if (!wanted) return null;
  const stack: TreeNode[] = [root];
  while (stack.length > 0) {
    const node = stack.pop()!;
    if (node.path.replace(/\\+$/, '').toLowerCase() === wanted) return node;
    for (const child of node.children) stack.push(child);
  }
  return null;
}
