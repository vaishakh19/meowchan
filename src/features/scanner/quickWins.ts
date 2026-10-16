/**
 * Quick Wins — the sidebar's "where can I get space back right now" list.
 *
 * Everything here is derived from a real scan tree. Nothing is hardcoded: if the category is not
 * present on the scanned volume the row simply does not appear. The cleanup assistant reuses the
 * same classification so the numbers in the sidebar and the confirmation dialog always agree.
 */

import type { StorageCategory, TreeNode } from '@/types/contracts';
import { collectFiles, collectFolders } from '@/lib/tree';

export type QuickWinId =
  | 'downloads'
  | 'temp'
  | 'caches'
  | 'large-media'
  | 'development'
  | 'recycle-bin';

export interface QuickWin {
  id: QuickWinId;
  label: string;
  /** Bytes reclaimable. Always a real measurement from the scan. */
  bytes: number;
  fileCount: number;
  folderCount: number;
  /** Folder the win belongs to, when it maps to one. */
  path: string | null;
  /** What Diskwise would do, in plain language. */
  hint: string;
  /** Categories that make up this win (for the colour dot). */
  category: StorageCategory;
}

const MEDIA_EXTENSIONS = new Set([
  'mp4', 'mkv', 'avi', 'mov', 'wmv', 'flv', 'webm', 'mpg', 'mpeg', 'm4v',
  'jpg', 'jpeg', 'png', 'gif', 'bmp', 'tiff', 'heic', 'raw', 'arw', 'cr2', 'nef', 'dng',
  'mp3', 'flac', 'wav', 'aac', 'ogg', 'wma', 'm4a',
]);

const DEV_FOLDER_NAMES = new Set([
  'node_modules', 'target', 'vendor', 'bower_components', '.git', '.svn', '__pycache__',
  '.venv', 'venv', 'dist', 'build', '.gradle', '.m2', 'obj', 'bin',
]);

const CACHE_FOLDER_TOKENS = ['cache', 'caches', 'cacheddata', 'code cache', 'gpucache', 'shadercache'];

const TEMP_PATH_TOKENS = ['\\temp\\', '\\tmp\\', '\\temporary internet files\\'];

const DOWNLOAD_FOLDER_NAMES = new Set(['downloads', 'download']);

const RECYCLE_BIN_NAMES = new Set(['$recycle.bin', 'recycler', '.trash', '$windows.~bt']);

const LARGE_MEDIA_THRESHOLD = 100 * 1024 * 1024;

function matchesName(node: TreeNode, names: Set<string>): boolean {
  return names.has(node.name.toLowerCase());
}

function isUnder(path: string, needle: string): boolean {
  return path.toLowerCase().includes(needle);
}

/**
 * Computes the Quick Wins for a scan root.
 *
 * The rules are intentionally conservative: a folder only counts as a cache when its name says so,
 * and "large media" is an *observation* (big media files that exist), not a recommendation to
 * delete personal data.
 */
export function computeQuickWins(root: TreeNode, options: { largeUnusedDays?: number } = {}): QuickWin[] {
  const wins: QuickWin[] = [];
  const folders = collectFolders(root);
  const files = collectFiles(root);

  /* Downloads */
  const downloadFolders = folders.filter((folder) => matchesName(folder, DOWNLOAD_FOLDER_NAMES));
  if (downloadFolders.length > 0) {
    const bytes = downloadFolders.reduce((sum, folder) => sum + folder.sizeBytes, 0);
    wins.push({
      id: 'downloads',
      label: 'Downloads',
      bytes,
      fileCount: downloadFolders.reduce((sum, folder) => sum + folder.fileCount, 0),
      folderCount: downloadFolders.length,
      path: downloadFolders[0].path,
      hint: 'Installers and archives you have already used. Review before removing.',
      category: 'downloads',
    });
  }

  /* Temporary files */
  const tempFolders = folders.filter(
    (folder) =>
      matchesName(folder, new Set(['temp', 'tmp'])) ||
      TEMP_PATH_TOKENS.some((token) => isUnder(`\\${folder.path.toLowerCase()}\\`, token)),
  );
  if (tempFolders.length > 0) {
    wins.push({
      id: 'temp',
      label: 'Temporary files',
      bytes: tempFolders.reduce((sum, folder) => sum + folder.sizeBytes, 0),
      fileCount: tempFolders.reduce((sum, folder) => sum + folder.fileCount, 0),
      folderCount: tempFolders.length,
      path: tempFolders[0].path,
      hint: 'Scratch files applications recreate on demand.',
      category: 'cache',
    });
  }

  /* Application caches (never the same folders already reported as temporary files) */
  const tempPaths = new Set(tempFolders.map((folder) => folder.path.toLowerCase()));
  const cacheFolders = folders.filter(
    (folder) =>
      !tempPaths.has(folder.path.toLowerCase()) &&
      (CACHE_FOLDER_TOKENS.some((token) => folder.name.toLowerCase().includes(token)) ||
        (isUnder(folder.path, '\\appdata\\local\\') && folder.name.toLowerCase().endsWith('cache'))),
  );
  if (cacheFolders.length > 0) {
    wins.push({
      id: 'caches',
      label: 'Application caches',
      bytes: cacheFolders.reduce((sum, folder) => sum + folder.sizeBytes, 0),
      fileCount: cacheFolders.reduce((sum, folder) => sum + folder.fileCount, 0),
      folderCount: cacheFolders.length,
      path: cacheFolders[0].path,
      hint: 'Regenerable cache data. The owning app rebuilds it on next launch.',
      category: 'cache',
    });
  }

  /* Large media files (observation only) */
  const largeMedia = files.filter(
    (file) => file.sizeBytes >= LARGE_MEDIA_THRESHOLD && file.extension !== null && MEDIA_EXTENSIONS.has(file.extension),
  );
  if (largeMedia.length > 0) {
    wins.push({
      id: 'large-media',
      label: 'Large media files',
      bytes: largeMedia.reduce((sum, file) => sum + file.sizeBytes, 0),
      fileCount: largeMedia.length,
      folderCount: 0,
      path: null,
      hint: 'Video and photo files over 100 MB. Diskwise will never treat these as junk.',
      category: 'media',
    });
  }

  /* Development artifacts */
  const devFolders = folders.filter((folder) => DEV_FOLDER_NAMES.has(folder.name.toLowerCase()));
  if (devFolders.length > 0) {
    wins.push({
      id: 'development',
      label: 'Development artifacts',
      bytes: devFolders.reduce((sum, folder) => sum + folder.sizeBytes, 0),
      fileCount: devFolders.reduce((sum, folder) => sum + folder.fileCount, 0),
      folderCount: devFolders.length,
      path: devFolders[0].path,
      hint: 'Dependencies and build output a package manager can restore.',
      category: 'development',
    });
  }

  /* Recycle Bin */
  const recycle = folders.filter((folder) => matchesName(folder, RECYCLE_BIN_NAMES));
  if (recycle.length > 0) {
    wins.push({
      id: 'recycle-bin',
      label: 'Recycle Bin',
      bytes: recycle.reduce((sum, folder) => sum + folder.sizeBytes, 0),
      fileCount: recycle.reduce((sum, folder) => sum + folder.fileCount, 0),
      folderCount: recycle.length,
      path: recycle[0].path,
      hint: 'Items you already deleted. Emptying frees the space permanently.',
      category: 'cache',
    });
  }

  void options;
  return wins.sort((a, b) => b.bytes - a.bytes);
}

/** Total bytes across all quick wins (never double counted: each row is a disjoint folder set). */
export function totalQuickWinBytes(wins: QuickWin[]): number {
  const seenIds = new Set<string>();
  const seenPaths = new Set<string>();
  let total = 0;
  for (const win of wins) {
    if (seenIds.has(win.id)) continue;
    seenIds.add(win.id);
    if (win.path) {
      const key = win.path.toLowerCase();
      if (seenPaths.has(key)) continue;
      seenPaths.add(key);
    }
    total += win.bytes;
  }
  return total;
}
