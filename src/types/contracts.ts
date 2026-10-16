/** Diskora native IPC contracts. Byte counts are logical u64 values serialized as JS numbers. */

/* ------------------------------------------------------------------ *
 * Drives
 * ------------------------------------------------------------------ */

export type DriveKind =
  | 'fixed'
  | 'removable'
  | 'network'
  | 'optical'
  | 'ramdisk'
  | 'unknown';

export interface DriveInfo {
  /** Stable identifier: drive letter on Windows (`C:`), mount point elsewhere. */
  id: string;
  /** Volume label, falling back to e.g. `Local Disk`. */
  name: string;
  /** Absolute root path, e.g. `C:\`. */
  path: string;
  kind: DriveKind;
  /** Total capacity in bytes. */
  totalBytes: number;
  /** Free bytes available to the caller. */
  freeBytes: number;
  /** Used bytes (`totalBytes - freeBytes`). */
  usedBytes: number;
  /** Filesystem name, e.g. `NTFS`, `exFAT`, `ReFS`. */
  fileSystem: string;
  isSystemDrive: boolean;
  isReadOnly: boolean;
  /** True when the OS reports the volume is not ready (empty card reader, etc.). */
  isDisconnected: boolean;
}

/* ------------------------------------------------------------------ *
 * Storage categories
 * ------------------------------------------------------------------ */

export type StorageCategory =
  | 'system'
  | 'applications'
  | 'games'
  | 'documents'
  | 'media'
  | 'development'
  | 'downloads'
  | 'cache'
  | 'archives'
  | 'other'
  | 'free';

/* ------------------------------------------------------------------ *
 * Scanning
 * ------------------------------------------------------------------ */

export type ScanPhase =
  | 'preparing'
  | 'walking'
  | 'hashing'
  | 'aggregating'
  | 'finalizing'
  | 'done'
  | 'cancelled'
  | 'error';

export type ScanStatus = 'running' | 'complete' | 'partial' | 'cancelled' | 'error';

export type ScanErrorKind =
  | 'access-denied'
  | 'not-found'
  | 'long-path'
  | 'symlink-cycle'
  | 'io'
  | 'limit-exceeded'
  | 'other';

export interface ScanErrorInfo {
  path: string;
  kind: ScanErrorKind;
  /** OS error message, never containing file contents. */
  message: string;
}

export interface ScanOptions {
  /** Absolute path of the directory to scan. Must exist and be a directory. */
  rootPath: string;
  /**
   * Maximum directory depth below the root. `0` means unlimited.
   * Kept as a safety valve for pathological trees.
   */
  maxDepth?: number;
  /** Follow symbolic links / junction points. Defaults to `false` (recommended). */
  followLinks?: boolean;
  /** Include hidden entries. Defaults to `true`. */
  includeHidden?: boolean;
  /** Include entries flagged SYSTEM by the OS. Defaults to `true`. */
  includeSystem?: boolean;
  /** Hard cap on the number of entries visited. `0` means unlimited. */
  maxEntries?: number;
  /**
   * Number of children kept per folder in the tree sent to the UI. Deeper children are
   * collapsed into a synthetic `Other (n items)` node so the visualization stays accurate
   * without shipping millions of nodes. Defaults to 100.
   */
  maxChildrenPerNode?: number;
  /** Compute content hashes during the scan (duplicate detection). Defaults to `false`. */
  computeHashes?: boolean;
}

export interface ScanProgress {
  scanId: string;
  phase: ScanPhase;
  status: ScanStatus;
  filesScanned: number;
  foldersScanned: number;
  bytesAnalyzed: number;
  errors: number;
  /** Directory currently being read. */
  currentPath: string;
  elapsedMs: number;
  /**
   * Estimated remaining milliseconds, or `null` when no reliable estimate exists yet.
   * Only emitted once >5% of the expected byte total has been analyzed.
   */
  etaMs: number | null;
  /** Percentage of the expected byte total, 0..100, or null when unknown. */
  percent: number | null;
  /** True when the tree was truncated because a safety cap was hit. */
  truncated: boolean;
}

export interface TreeNode {
  /** Path-derived stable id (`path` hashed). Never changes between scans of the same path. */
  id: string;
  path: string;
  name: string;
  kind: 'folder' | 'file';
  /** Sum of logical file sizes in this subtree. */
  sizeBytes: number;
  /** Sum of allocated on-disk sizes when the platform exposes them, else `null`. */
  diskSizeBytes: number | null;
  fileCount: number;
  folderCount: number;
  modifiedAt: number | null;
  createdAt: number | null;
  /** Lower-case extension without the dot, e.g. `mp4`. `null` for folders. */
  extension: string | null;
  category: StorageCategory;
  /** The subtree could not be read (access denied / unreadable device). */
  accessDenied: boolean;
  children: TreeNode[];
  /** True for the aggregated placeholder node created by child pruning. */
  synthetic: boolean;
}

export interface ScanSummary {
  scanId: string;
  rootPath: string;
  rootName: string;
  startedAt: number;
  finishedAt: number | null;
  durationMs: number | null;
  status: ScanStatus;
  totalBytes: number;
  fileCount: number;
  folderCount: number;
  errorCount: number;
  truncated: boolean;
  /** Drive the scan ran on, when the root is a drive root. */
  driveId: string | null;
  /** Set for scans that were persisted to the local database. */
  persisted: boolean;
}

export interface ScanResult {
  scanId: string;
  rootPath: string;
  rootName: string;
  startedAt: number;
  finishedAt: number | null;
  status: ScanStatus;
  /** Pruned hierarchical tree. Sizes at every level are exact. */
  tree: TreeNode;
  totalBytes: number;
  diskTotalBytes: number | null;
  fileCount: number;
  folderCount: number;
  errors: ScanErrorInfo[];
  truncated: boolean;
  /** Number of nodes dropped from the tree by pruning (children + depth budget). */
  prunedNodes: number;
  durationMs: number | null;
  /** Byte count the scan expected to find (from volume metadata), used for ETA/percent. */
  expectedBytes: number | null;
}

export interface ChildrenPage {
  path: string;
  offset: number;
  limit: number;
  total: number;
  items: TreeNode[];
}

/* ------------------------------------------------------------------ *
 * Large files
 * ------------------------------------------------------------------ */

export type LargeFileSort = 'size-desc' | 'size-asc' | 'modified-desc' | 'modified-asc' | 'name-asc' | 'name-desc';

export interface LargeFileQuery {
  scanId: string;
  /** Minimum logical size in bytes. */
  minBytes: number;
  /** Restrict to these lower-case extensions (without dot). Empty = all. */
  extensions?: string[];
  /** Only files modified at or after this timestamp (ms). */
  modifiedAfter?: number | null;
  /** Only files modified at or before this timestamp (ms). */
  modifiedBefore?: number | null;
  sort?: LargeFileSort;
  offset?: number;
  limit?: number;
}

export interface LargeFile {
  path: string;
  name: string;
  sizeBytes: number;
  diskSizeBytes: number | null;
  modifiedAt: number | null;
  extension: string | null;
  category: StorageCategory;
}

export interface LargeFilePage {
  total: number;
  totalBytes: number;
  offset: number;
  limit: number;
  items: LargeFile[];
}

/* ------------------------------------------------------------------ *
 * Duplicate detection
 * ------------------------------------------------------------------ */

export interface DuplicateOptions {
  scanId: string;
  /** Files smaller than this are ignored. Defaults to 1 KiB. */
  minBytes?: number;
  /** Files larger than this are ignored (hashing cost guard). Defaults to 5 GiB. */
  maxBytes?: number;
  /** Optional path prefixes that must not be considered. */
  excludePaths?: string[];
  /** Hash algorithm. `blake3` is fastest; `sha256` is the conservative default. */
  algorithm?: 'sha256' | 'blake3';
  /** Restrict to these lower-case extensions. Empty = all. */
  extensions?: string[];
}

export interface DuplicateFile {
  path: string;
  name: string;
  sizeBytes: number;
  modifiedAt: number | null;
  /** `true` when a byte-for-byte comparison confirmed the match (not just the hash). */
  byteVerified: boolean;
  /** Index of the copy the app suggests keeping (oldest, then shallowest path). */
  isSuggestedOriginal: boolean;
}

export interface DuplicateGroup {
  groupId: string;
  sizeBytes: number;
  /** `sizeBytes * (copies - 1)` — space reclaimable if all but one copy are removed. */
  reclaimableBytes: number;
  copies: DuplicateFile[];
  hash: string;
  /** `size` groups confirmed by hash only, `verified` groups also byte-compared. */
  confidence: 'hash' | 'verified';
}

export interface DuplicateReport {
  reportId: string;
  scanId: string;
  startedAt: number;
  finishedAt: number | null;
  status: ScanStatus;
  algorithm: 'sha256' | 'blake3';
  groups: DuplicateGroup[];
  totalGroups: number;
  totalCopies: number;
  totalReclaimableBytes: number;
  hashedBytes: number;
  filesHashed: number;
  errors: ScanErrorInfo[];
}

export interface DuplicateVerifyResult {
  equal: boolean;
  bytesCompared: number;
  firstDifferenceOffset: number | null;
}

/* ------------------------------------------------------------------ *
 * File details (details panel)
 * ------------------------------------------------------------------ */

export interface FileDetails {
  path: string;
  name: string;
  kind: 'folder' | 'file';
  sizeBytes: number;
  diskSizeBytes: number | null;
  fileCount: number;
  folderCount: number;
  createdAt: number | null;
  modifiedAt: number | null;
  accessedAt: number | null;
  extension: string | null;
  category: StorageCategory;
  isSymlink: boolean;
  isReadOnly: boolean;
  parentPath: string | null;
}

export interface AppVersion {
  version: string;
  identifier: string;
  platform: string;
  arch: string;
}

/* ------------------------------------------------------------------ *
 * Diskora Pro (licensing)
 * ------------------------------------------------------------------ */

/**
 * The licence state as verified by the native backend against Dodo
 * Payments. `enabled` is never decided by the frontend: it mirrors a
 * server-verified entitlement (or a recently verified one during the
 * documented offline grace window).
 */
export interface ProStatus {
  /** True when a verified Diskora Pro purchase is active on this device. */
  enabled: boolean;
  /** False when this build has no Dodo Payments product configured. */
  configured: boolean;
  /** Masked key (`ABCD…WXYZ`), when an entitlement is stored. */
  licenseKey: string | null;
  activatedAt: number | null;
  lastValidatedAt: number | null;
  /** True when the active state rests on the offline grace period right now. */
  offline: boolean;
  /** Human-readable explanation of the current state. */
  message: string;
}

/* ------------------------------------------------------------------ *
 * Safe deletion (Pro)
 * ------------------------------------------------------------------ */

export interface DeleteItemsRequest {
  scanId: string;
  paths: string[];
}

export interface DeletedItem {
  path: string;
  name: string;
  kind: 'file' | 'folder';
  sizeBytes: number;
}

export interface DeleteFailure {
  path: string;
  reason: string;
}

/** Per-item results: a failure never hides what succeeded, or vice versa. */
export interface DeleteOutcome {
  deleted: DeletedItem[];
  failed: DeleteFailure[];
}

/* ------------------------------------------------------------------ *
 * Settings & privacy
 * ------------------------------------------------------------------ */

export type ThemeMode = 'light' | 'dark' | 'system';

export interface AppSettings {
  theme: ThemeMode;
  maxChildrenPerNode: number;
  followLinks: boolean;
}

export interface PrivacyReport {
  /** Everything stored locally, as human-readable lines. */
  localStorage: string[];
  /** Everything that leaves the device, as human-readable lines. Empty by default. */
  networkActivity: string[];
  dataDir: string;
  databasePath: string;
}

/* ------------------------------------------------------------------ *
 * Backend → frontend events
 * ------------------------------------------------------------------ */

export interface ScanProgressEvent {
  payload: ScanProgress;
}

export interface DuplicateProgressEvent {
  payload: DuplicateProgress;
}

export interface DuplicateProgress {
  reportId: string;
  phase: 'grouping' | 'hashing' | 'verifying' | 'finalizing' | 'done' | 'error';
  filesHashed: number;
  bytesHashed: number;
  candidateGroups: number;
  groupsFound: number;
  elapsedMs: number;
  etaMs: number | null;
}
