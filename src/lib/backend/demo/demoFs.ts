/**
 * Deterministic synthetic Windows filesystem for the **browser preview** transport.
 *
 * This exists so the interface can be reviewed, demoed and tested in a normal browser
 * (`npm run dev`) where no Tauri runtime and no real disk access exist. It is NOT used by the
 * packaged application: on Windows the Rust backend performs real scans (see
 * `src-tauri/src/scanner/`).
 *
 * The generator is seeded, so every run produces byte-identical data — which is what makes the
 * preview tests and screenshots reproducible.
 */

import type { StorageCategory, TreeNode } from '@/types/contracts';
import { extensionOf } from '@/lib/path';
import { stablePathId } from '@/lib/id';

const KB = 1024;
const MB = 1024 * KB;
const GB = 1024 * MB;

/** mulberry32 — small, fast, deterministic PRNG. */
function createRandom(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

interface Draft {
  name: string;
  kind: 'folder' | 'file';
  category: StorageCategory;
  path: string;
  sizeBytes: number;
  modifiedAt: number;
  createdAt: number;
  accessDenied: boolean;
  children: Draft[];
}

interface FileSpec {
  ext: string;
  min: number;
  max: number;
  /** Share of zero-byte files, 0..1. */
  emptyRate?: number;
}

interface FolderSpec {
  name: string;
  category: StorageCategory;
  accessDenied?: boolean;
  /** Sub-folder specs, created verbatim. */
  folders?: FolderSpec[];
  /** A sub-folder per entry, filled with `count` files. */
  fileGroups?: Array<{ label: string; count: number; spec: FileSpec }>;
  /** Procedurally generated sibling folders (installed apps, npm packages, …). */
  autoFolders?: number;
}

const NOW = Date.UTC(2026, 9, 10, 9, 30);

function gaussian(rand: () => number): number {
  const u = Math.max(rand(), 1e-9);
  const v = rand();
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v);
}

/** Log-normal size within [min,max] — realistic for real files. */
function pickSize(rand: () => number, spec: FileSpec): number {
  if (spec.emptyRate && rand() < spec.emptyRate) return 0;
  const lo = Math.log(Math.max(1, spec.min));
  const hi = Math.log(spec.max);
  const mid = (lo + hi) / 2;
  const sigma = Math.max(0.05, (hi - lo) / 4);
  const value = Math.exp(mid + gaussian(rand) * sigma);
  return Math.round(Math.min(spec.max, Math.max(spec.min, value)));
}

/** Age in days, biased towards recently modified files. */
function pickAgeDays(rand: () => number): number {
  return Math.round(rand() ** 2.2 * 1500);
}

const NAME_WORDS = [
  'report', 'notes', 'draft', 'invoice', 'photo', 'screenshot', 'backup', 'export', 'config',
  'manifest', 'readme', 'index', 'main', 'util', 'helper', 'sample', 'dataset', 'media', 'asset',
  'template', 'log', 'cache', 'session', 'profile', 'theme', 'patch', 'bundle', 'package',
  'kernel', 'driver', 'resource', 'locale', 'manifest', 'preview', 'thumbnail',
];

function pickName(rand: () => number): string {
  return NAME_WORDS[Math.floor(rand() * NAME_WORDS.length)];
}

/* ------------------------------------------------------------------ *
 * File specs
 * ------------------------------------------------------------------ */

const DLL: FileSpec = { ext: 'dll', min: 24 * KB, max: 6 * MB };
const EXE: FileSpec = { ext: 'exe', min: 90 * KB, max: 40 * MB };
const SYS: FileSpec = { ext: 'sys', min: 8 * KB, max: 900 * KB };
const DAT: FileSpec = { ext: 'dat', min: 4 * KB, max: 2 * MB, emptyRate: 0.05 };
const LOG: FileSpec = { ext: 'log', min: 1 * KB, max: 12 * MB, emptyRate: 0.12 };
const NUPKG: FileSpec = { ext: 'nupkg', min: 200 * KB, max: 60 * MB };
const NODE: FileSpec = { ext: 'js', min: 400, max: 900 * KB };
const TSMAP: FileSpec = { ext: 'map', min: 2 * KB, max: 4 * MB };
const RUST: FileSpec = { ext: 'rs', min: 500, max: 300 * KB };
const OBJ: FileSpec = { ext: 'o', min: 20 * KB, max: 20 * MB };
const IMG: FileSpec = { ext: 'jpg', min: 1.2 * MB, max: 14 * MB, emptyRate: 0.01 };
const RAW: FileSpec = { ext: 'arw', min: 22 * MB, max: 48 * MB };
const VIDEO: FileSpec = { ext: 'mp4', min: 40 * MB, max: 4.2 * GB };
const AUDIO: FileSpec = { ext: 'flac', min: 12 * MB, max: 90 * MB };
const DOC: FileSpec = { ext: 'docx', min: 20 * KB, max: 12 * MB };
const PDF: FileSpec = { ext: 'pdf', min: 80 * KB, max: 40 * MB };
const XLSX: FileSpec = { ext: 'xlsx', min: 15 * KB, max: 8 * MB };
const INSTALLER: FileSpec = { ext: 'exe', min: 20 * MB, max: 320 * MB };
const ZIP: FileSpec = { ext: 'zip', min: 4 * MB, max: 3 * GB };
const ISO: FileSpec = { ext: 'iso', min: 700 * MB, max: 5.6 * GB };
const TMP: FileSpec = { ext: 'tmp', min: 512, max: 400 * MB, emptyRate: 0.25 };
const CACHE: FileSpec = { ext: 'cache', min: 1 * KB, max: 90 * MB, emptyRate: 0.2 };
const SQLITE: FileSpec = { ext: 'db', min: 2 * MB, max: 600 * MB };
const GAMEASSET: FileSpec = { ext: 'pak', min: 5 * MB, max: 2.4 * GB };
const GAMETEX: FileSpec = { ext: 'dds', min: 1 * MB, max: 120 * MB };
const FONT: FileSpec = { ext: 'ttf', min: 40 * KB, max: 4 * MB };
const MUI: FileSpec = { ext: 'mui', min: 12 * KB, max: 300 * KB };
const BLOB: FileSpec = { ext: 'bin', min: 2 * MB, max: 700 * MB };

/* ------------------------------------------------------------------ *
 * The machine layout
 * ------------------------------------------------------------------ */

const MACHINE: FolderSpec = {
  name: '',
  category: 'system',
  folders: [
    {
      name: 'Windows',
      category: 'system',
      autoFolders: 18,
      fileGroups: [
        { label: 'System32', count: 3200, spec: DLL },
        { label: 'SysWOW64', count: 1600, spec: DLL },
        { label: 'WinSxS', count: 2000, spec: DLL },
        { label: 'Drivers', count: 600, spec: SYS },
        { label: 'Fonts', count: 300, spec: FONT },
        { label: 'Globalization', count: 400, spec: MUI },
        { label: 'Logs', count: 500, spec: LOG },
        { label: 'Temp', count: 600, spec: TMP },
        { label: 'Installer', count: 800, spec: DAT },
        { label: 'SystemApps', count: 500, spec: DLL },
        { label: 'explorer', count: 140, spec: EXE },
      ],
    },
    {
      name: 'Program Files',
      category: 'applications',
      autoFolders: 26,
      fileGroups: [
        { label: 'Common Files', count: 1500, spec: DLL },
        { label: 'Windows NT', count: 600, spec: DLL },
        { label: 'Internet Explorer', count: 200, spec: DLL },
        { label: 'MSBuild', count: 500, spec: DLL },
        { label: 'Reference Assemblies', count: 600, spec: DLL },
        { label: 'Package Management', count: 200, spec: NUPKG },
      ],
    },
    {
      name: 'Program Files (x86)',
      category: 'applications',
      autoFolders: 14,
      fileGroups: [
        { label: 'Common Files', count: 1200, spec: DLL },
        { label: 'Windows Installer', count: 300, spec: DAT },
      ],
    },
    {
      name: 'Users',
      category: 'documents',
      folders: [
        {
          name: 'alex',
          category: 'documents',
          folders: [
            {
              name: 'Downloads',
              category: 'downloads',
              fileGroups: [
                { label: 'installers', count: 70, spec: INSTALLER },
                { label: 'archives', count: 100, spec: ZIP },
                { label: 'disk-images', count: 10, spec: ISO },
                { label: 'documents', count: 420, spec: PDF },
                { label: 'spreadsheets', count: 140, spec: XLSX },
                { label: 'misc', count: 600, spec: BLOB },
              ],
            },
            {
              name: 'Documents',
              category: 'documents',
              autoFolders: 10,
              fileGroups: [
                { label: 'reports', count: 900, spec: DOC },
                { label: 'contracts', count: 200, spec: PDF },
                { label: 'sheets', count: 380, spec: XLSX },
              ],
            },
            {
              name: 'Pictures',
              category: 'media',
              autoFolders: 7,
              fileGroups: [
                { label: 'Camera Roll', count: 1800, spec: IMG },
                { label: 'Raw Archive', count: 220, spec: RAW },
                { label: 'Screenshots', count: 500, spec: IMG },
              ],
            },
            {
              name: 'Videos',
              category: 'media',
              autoFolders: 3,
              fileGroups: [
                { label: 'Projects', count: 40, spec: VIDEO },
                { label: 'Clips', count: 260, spec: VIDEO },
              ],
            },
            {
              name: 'Music',
              category: 'media',
              autoFolders: 2,
              fileGroups: [
                { label: 'Albums', count: 1200, spec: AUDIO },
                { label: 'Podcasts', count: 300, spec: { ext: 'mp3', min: 8 * MB, max: 220 * MB } },
              ],
            },
            {
              name: 'Desktop',
              category: 'documents',
              fileGroups: [{ label: 'items', count: 90, spec: DOC }],
            },
            {
              name: 'Projects',
              category: 'development',
              folders: [
                {
                  name: 'diskwise',
                  category: 'development',
                  folders: [
                    {
                      name: 'node_modules',
                      category: 'development',
                      autoFolders: 40,
                      fileGroups: [
                        { label: 'dist', count: 500, spec: NODE },
                        { label: 'lib', count: 1500, spec: NODE },
                        { label: 'types', count: 400, spec: TSMAP },
                      ],
                    },
                    {
                      name: 'src-tauri',
                      category: 'development',
                      autoFolders: 5,
                      fileGroups: [
                        { label: 'target', count: 1600, spec: OBJ },
                        { label: 'src', count: 120, spec: RUST },
                      ],
                    },
                    {
                      name: 'src',
                      category: 'development',
                      autoFolders: 8,
                      fileGroups: [
                        { label: 'components', count: 200, spec: { ext: 'tsx', min: 600, max: 60 * KB } },
                        { label: 'features', count: 300, spec: { ext: 'ts', min: 400, max: 90 * KB } },
                      ],
                    },
                    {
                      name: '.git',
                      category: 'development',
                      autoFolders: 3,
                      fileGroups: [
                        { label: 'objects', count: 1100, spec: { ext: 'pack', min: 1 * KB, max: 12 * MB } },
                        { label: 'logs', count: 30, spec: LOG },
                      ],
                    },
                  ],
                },
                {
                  name: 'legacy-app',
                  category: 'development',
                  autoFolders: 10,
                  fileGroups: [
                    { label: 'vendor', count: 1800, spec: { ext: 'php', min: 800, max: 200 * KB } },
                    { label: 'build', count: 600, spec: TSMAP },
                  ],
                },
                {
                  name: 'data-pipeline',
                  category: 'development',
                  autoFolders: 4,
                  fileGroups: [
                    { label: 'datasets', count: 90, spec: SQLITE },
                    { label: 'notebooks', count: 45, spec: { ext: 'ipynb', min: 20 * KB, max: 12 * MB } },
                  ],
                },
              ],
            },
            {
              name: 'AppData',
              category: 'cache',
              folders: [
                {
                  name: 'Local',
                  category: 'cache',
                  folders: [
                    {
                      name: 'Temp',
                      category: 'cache',
                      fileGroups: [
                        { label: 'tmp', count: 1500, spec: TMP },
                        { label: 'cache', count: 500, spec: CACHE },
                      ],
                    },
                    {
                      name: 'Google',
                      category: 'cache',
                      autoFolders: 4,
                      fileGroups: [
                        { label: 'Chrome', count: 2600, spec: CACHE },
                        { label: 'DriveFS', count: 600, spec: SQLITE },
                      ],
                    },
                    {
                      name: 'Microsoft',
                      category: 'cache',
                      autoFolders: 8,
                      fileGroups: [
                        { label: 'Edge', count: 2200, spec: CACHE },
                        { label: 'Teams', count: 900, spec: CACHE },
                        { label: 'Outlook', count: 500, spec: { ext: 'ost', min: 20 * MB, max: 2.4 * GB } },
                      ],
                    },
                    {
                      name: 'npm-cache',
                      category: 'cache',
                      fileGroups: [{ label: '_cacache', count: 1600, spec: CACHE }],
                    },
                    {
                      name: 'pip',
                      category: 'cache',
                      fileGroups: [{ label: 'Cache', count: 500, spec: { ext: 'whl', min: 200 * KB, max: 90 * MB } }],
                    },
                  ],
                },
                {
                  name: 'Roaming',
                  category: 'cache',
                  autoFolders: 12,
                  fileGroups: [{ label: 'appdata', count: 1500, spec: CACHE }],
                },
              ],
            },
          ],
        },
        {
          name: 'Public',
          category: 'documents',
          fileGroups: [{ label: 'shared', count: 200, spec: DOC }],
        },
        {
          name: 'Default',
          category: 'system',
          accessDenied: true,
        },
      ],
    },
    {
      name: 'ProgramData',
      category: 'applications',
      autoFolders: 20,
      fileGroups: [
        { label: 'Package Cache', count: 900, spec: DAT },
        { label: 'Microsoft', count: 1400, spec: CACHE },
      ],
    },
    {
      name: 'Games',
      category: 'games',
      folders: [
        {
          name: 'Steam',
          category: 'games',
          folders: [
            {
              name: 'steamapps',
              category: 'games',
              folders: [
                {
                  name: 'common',
                  category: 'games',
                  autoFolders: 6,
                  fileGroups: [
                    { label: 'content', count: 900, spec: GAMEASSET },
                    { label: 'textures', count: 1400, spec: GAMETEX },
                  ],
                },
                {
                  name: 'downloading',
                  category: 'games',
                  fileGroups: [{ label: 'temp', count: 180, spec: GAMEASSET }],
                },
                {
                  name: 'shadercache',
                  category: 'cache',
                  fileGroups: [{ label: 'cache', count: 1100, spec: CACHE }],
                },
              ],
            },
          ],
        },
        {
          name: 'Epic Games',
          category: 'games',
          autoFolders: 2,
          fileGroups: [{ label: 'content', count: 600, spec: GAMEASSET }],
        },
      ],
    },
    {
      name: '$Recycle.Bin',
      category: 'cache',
      autoFolders: 2,
      fileGroups: [{ label: 'deleted', count: 600, spec: BLOB }],
    },
    {
      name: 'Recovery',
      category: 'system',
      fileGroups: [{ label: 'image', count: 6, spec: { ext: 'wim', min: 300 * MB, max: 2.4 * GB } }],
    },
    {
      name: 'System Volume Information',
      category: 'system',
      accessDenied: true,
    },
  ],
};

/* ------------------------------------------------------------------ *
 * Tree construction
 * ------------------------------------------------------------------ */

function buildDrafts(spec: FolderSpec, parentPath: string, rand: () => number): Draft {
  const draft: Draft = {
    name: spec.name,
    kind: 'folder',
    category: spec.category,
    path: parentPath,
    sizeBytes: 0,
    modifiedAt: NOW - Math.floor(rand() * 30 * 86_400_000),
    createdAt: NOW - Math.floor(rand() * 900 * 86_400_000),
    accessDenied: spec.accessDenied === true,
    children: [],
  };
  if (draft.accessDenied) return draft;

  const addFolder = (name: string, category: StorageCategory, children: Draft[]) => {
    draft.children.push({
      name,
      kind: 'folder',
      category,
      path: `${parentPath}\\${name}`,
      sizeBytes: 0,
      modifiedAt: NOW - Math.floor(rand() * 400 * 86_400_000),
      createdAt: NOW - Math.floor(rand() * 900 * 86_400_000),
      accessDenied: false,
      children,
    });
  };

  // A sub-folder per file group: `Windows\System32\report-0001.dll`.
  for (const group of spec.fileGroups ?? []) {
    const files: Draft[] = [];
    for (let i = 0; i < group.count; i += 1) {
      const name = `${pickName(rand)}-${String(i + 1).padStart(4, '0')}.${group.spec.ext}`;
      const ageDays = pickAgeDays(rand);
      files.push({
        name,
        kind: 'file',
        category: spec.category,
        path: `${parentPath}\\${group.label}\\${name}`,
        sizeBytes: pickSize(rand, group.spec),
        modifiedAt: NOW - ageDays * 86_400_000,
        createdAt: NOW - (ageDays + Math.floor(rand() * 400)) * 86_400_000,
        accessDenied: false,
        children: [],
      });
    }
    addFolder(group.label, spec.category, files);
  }

  for (const child of spec.folders ?? []) {
    draft.children.push(buildDrafts(child, `${parentPath}\\${child.name}`, rand));
  }

  // Procedural siblings: installed apps, npm packages, game titles. Each one gets the usual
  // bin/lib/resources shape so the tree has realistic depth and folder counts.
  const prefix = spec.category === 'development' ? 'pkg' : 'app';
  const subFolders = ['bin', 'lib', 'resources'];
  for (let i = 0; i < (spec.autoFolders ?? 0); i += 1) {
    const name = `${prefix}-${String(i + 1).padStart(3, '0')}`;
    const childFolders: Draft[] = [];
    const dataCount = 20 + Math.floor(rand() * 120);
    const perFolder = Math.max(1, Math.ceil(dataCount / subFolders.length));

    subFolders.forEach((sub, subIndex) => {
      const files: Draft[] = [];
      const start = subIndex * perFolder;
      const end = Math.min(dataCount, start + perFolder);
      for (let f = start; f < end; f += 1) {
        const fileName = `${pickName(rand)}-${String(f + 1).padStart(4, '0')}.${spec.category === 'development' ? 'rs' : 'dll'}`;
        files.push({
          name: fileName,
          kind: 'file',
          category: spec.category,
          path: `${parentPath}\\${name}\\${sub}\\${fileName}`,
          sizeBytes: pickSize(rand, spec.category === 'development' ? RUST : DLL),
          modifiedAt: NOW - Math.floor(rand() ** 1.6 * 1200) * 86_400_000,
          createdAt: NOW - Math.floor(rand() * 900) * 86_400_000,
          accessDenied: false,
          children: [],
        });
      }
      // Executables live in bin/.
      if (sub === 'bin') {
        const binCount = 4 + Math.floor(rand() * 24);
        for (let f = 0; f < binCount; f += 1) {
          const fileName = `bin-${String(f + 1).padStart(3, '0')}.exe`;
          files.push({
            name: fileName,
            kind: 'file',
            category: spec.category,
            path: `${parentPath}\\${name}\\${sub}\\${fileName}`,
            sizeBytes: pickSize(rand, EXE),
            modifiedAt: NOW - Math.floor(rand() ** 1.6 * 1200) * 86_400_000,
            createdAt: NOW - Math.floor(rand() * 900) * 86_400_000,
            accessDenied: false,
            children: [],
          });
        }
      }
      childFolders.push({
        name: sub,
        kind: 'folder',
        category: spec.category,
        path: `${parentPath}\\${name}\\${sub}`,
        sizeBytes: 0,
        modifiedAt: NOW - Math.floor(rand() * 400 * 86_400_000),
        createdAt: NOW - Math.floor(rand() * 900 * 86_400_000),
        accessDenied: false,
        children: files,
      });
    });
    addFolder(name, spec.category, childFolders);
  }

  return draft;
}

function toTreeNode(draft: Draft): TreeNode {
  const node: TreeNode = {
    id: stablePathId(draft.path),
    path: draft.path,
    name: draft.name,
    kind: draft.kind,
    sizeBytes: 0,
    diskSizeBytes: null,
    fileCount: 0,
    folderCount: 0,
    modifiedAt: draft.modifiedAt,
    createdAt: draft.createdAt,
    extension: draft.kind === 'file' ? extensionOf(draft.name) : null,
    category: draft.category,
    accessDenied: draft.accessDenied,
    children: [],
    synthetic: false,
  };

  if (draft.kind === 'file') {
    node.sizeBytes = draft.sizeBytes;
    node.diskSizeBytes = draft.sizeBytes === 0 ? 0 : Math.ceil(draft.sizeBytes / 4096) * 4096;
    node.fileCount = 1;
    return node;
  }

  let diskKnown = false;
  for (const child of draft.children) {
    const childNode = toTreeNode(child);
    node.children.push(childNode);
    node.sizeBytes += childNode.sizeBytes;
    node.fileCount += childNode.fileCount;
    node.folderCount += childNode.folderCount + (childNode.kind === 'folder' ? 1 : 0);
    if (childNode.diskSizeBytes !== null) diskKnown = true;
    if (childNode.modifiedAt !== null && (node.modifiedAt === null || childNode.modifiedAt > node.modifiedAt)) {
      node.modifiedAt = childNode.modifiedAt;
    }
  }
  node.diskSizeBytes = diskKnown
    ? node.children.reduce((sum, c) => sum + (c.diskSizeBytes ?? 0), 0)
    : null;
  return node;
}

export interface DemoMachine {
  root: TreeNode;
  drive: { totalBytes: number; usedBytes: number; freeBytes: number };
  /** Pre-order traversal order, used to fake scan progress. */
  walkOrder: TreeNode[];
  stats: { files: number; folders: number; bytes: number };
}

/** Builds the synthetic machine. Deterministic for a given seed. */
export function generateDemoMachine(seed = 20260910, rootPath = 'C:\\'): DemoMachine {
  const rand = createRandom(seed);
  const draft = buildDrafts(MACHINE, rootPath, rand);
  const root = toTreeNode(draft);
  root.name = 'Local Disk (C:)';

  const walkOrder: TreeNode[] = [];
  const stack: TreeNode[] = [root];
  while (stack.length > 0) {
    const node = stack.pop()!;
    walkOrder.push(node);
    for (let i = node.children.length - 1; i >= 0; i -= 1) stack.push(node.children[i]);
  }

  const usedBytes = root.sizeBytes;
  // Round the volume up to a tidy capacity so the usage ring looks like a real SSD.
  const totalBytes = Math.ceil((usedBytes * 1.65) / GB) * GB;
  return {
    root,
    drive: { totalBytes, usedBytes, freeBytes: totalBytes - usedBytes },
    walkOrder,
    stats: { files: root.fileCount, folders: root.folderCount, bytes: usedBytes },
  };
}

export { collectFiles, collectFolders } from '@/lib/tree';

export const DEMO_DRIVES: Array<{
  id: string;
  name: string;
  kind: 'fixed' | 'removable';
  totalBytes: number;
  usedRatio: number;
  fileSystem: string;
}> = [
  { id: 'C:', name: 'Local Disk', kind: 'fixed', totalBytes: 512 * GB, usedRatio: 0.61, fileSystem: 'NTFS' },
  { id: 'D:', name: 'Data', kind: 'fixed', totalBytes: 1024 * GB, usedRatio: 0.34, fileSystem: 'NTFS' },
  { id: 'E:', name: 'Backup', kind: 'fixed', totalBytes: 2048 * GB, usedRatio: 0.78, fileSystem: 'NTFS' },
  { id: 'F:', name: 'USB Drive', kind: 'removable', totalBytes: 64 * GB, usedRatio: 0.22, fileSystem: 'exFAT' },
];

export { NOW as DEMO_NOW };
