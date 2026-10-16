/**
 * Diskwise — shared request/response contracts between the React frontend and the Rust backend.
 *
 * This file is the single source of truth for the wire format. Every struct here is mirrored
 * 1:1 by a `#[derive(Serialize, Deserialize)]` struct in `src-tauri/src/**` with
 * `#[serde(rename_all = "camelCase")]`. If you change a field here, change it there too
 * (see `src-tauri/src/commands/contracts.rs`).
 *
 * Wire encoding notes:
 *  - Byte counts are `u64` on the Rust side and `number` in TypeScript. This is exact up to
 *    2^53 bytes (9 PB), far beyond any consumer disk. The Rust side rejects inputs that would
 *    overflow and the frontend never does arithmetic on raw byte counts without `BigInt`-safe
 *    helpers in `src/lib/format.ts`.
 *  - Timestamps are Unix epoch milliseconds (`u64`), `null` when unavailable.
 *  - All paths are OS-native absolute paths. The backend canonicalizes and validates them.
 */

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
  /** Set for scans that were persisted to the local database (Pro feature). */
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
 * Cleanup
 * ------------------------------------------------------------------ */

export type CleanupRisk = 'informational' | 'low' | 'review';

export type CleanupCategory =
  | 'temp'
  | 'cache'
  | 'downloads'
  | 'large-unused'
  | 'recycle-bin'
  | 'duplicates'
  | 'development'
  | 'system'
  | 'custom';

export interface CleanupFinding {
  id: string;
  category: CleanupCategory;
  risk: CleanupRisk;
  /** Short human label, e.g. `Windows Temp files`. */
  title: string;
  /** Why this is a candidate, shown verbatim in the confirmation dialog. */
  reason: string;
  /** True when the item is protected and can only ever be informational. */
  protected: boolean;
  path: string | null;
  /** Exact items that would be removed. Always shown before any action. */
  paths: string[];
  fileCount: number;
  folderCount: number;
  sizeBytes: number;
  diskSizeBytes: number | null;
  /** Oldest / newest modification timestamps in the finding, when known. */
  oldestModifiedAt: number | null;
  newestModifiedAt: number | null;
  /** Set when the finding came from a user-supplied rule. */
  ruleId: string | null;
}

export interface CleanupRule {
  id: string;
  label: string;
  category: CleanupCategory;
  risk: CleanupRisk;
  /** Absolute directory the rule applies to. Must be inside an approved root. */
  path: string;
  /** File must not have been modified for this many days. `0` = any age. */
  olderThanDays: number;
  /** Minimum size in bytes for a file to be considered. `0` = any size. */
  minBytes: number;
  /** Lower-case extensions to include. Empty = all. */
  extensions: string[];
  /** Recurse into subdirectories. */
  recursive: boolean;
  enabled: boolean;
}

export interface CleanupPlanItem {
  findingId: string;
  /** Explicit user opt-in. The backend refuses items that are not listed here. */
  paths: string[];
  /** `recycle` = move to Recycle Bin (preferred), `permanent` = hard delete. */
  action: 'recycle' | 'permanent';
}

export interface CleanupPlan {
  scanId: string | null;
  items: CleanupPlanItem[];
  /** Simulates every action and reports exactly what would happen. */
  dryRun: boolean;
}

export interface CleanupPreviewItem {
  findingId: string;
  title: string;
  action: 'recycle' | 'permanent';
  pathCount: number;
  fileCount: number;
  folderCount: number;
  sizeBytes: number;
  /** Per-path outcome of the dry run. */
  paths: CleanupPathOutcome[];
}

export interface CleanupPathOutcome {
  path: string;
  /** `ok`, `missing` (already gone), `denied`, `changed` (modified after the scan), `skipped`. */
  status: 'ok' | 'missing' | 'denied' | 'changed' | 'skipped';
  sizeBytes: number;
  detail: string;
}

export interface CleanupPreview {
  items: CleanupPreviewItem[];
  totalSizeBytes: number;
  totalPathCount: number;
  /** Non-fatal problems found while simulating. */
  warnings: string[];
}

export interface CleanupResultItem {
  findingId: string;
  title: string;
  action: 'recycle' | 'permanent';
  removedCount: number;
  failedCount: number;
  removedBytes: number;
  results: CleanupPathOutcome[];
}

export interface CleanupResult {
  items: CleanupResultItem[];
  totalRemovedCount: number;
  totalFailedCount: number;
  totalRemovedBytes: number;
  completedAt: number;
  /** True when some but not all items succeeded. */
  partial: boolean;
}

export interface RecycleBinSummary {
  itemCount: number;
  sizeBytes: number | null;
  /** `false` when the platform does not expose this information. */
  supported: boolean;
  detail: string;
}

/* ------------------------------------------------------------------ *
 * Licensing / entitlements
 * ------------------------------------------------------------------ */

export type PlanTier = 'free' | 'trial' | 'pro';

export type EntitlementState =
  | 'free'
  | 'trial'
  | 'pro-active'
  | 'pro-expired'
  | 'invalid';

export interface FeatureFlags {
  /** Sunburst, treemap, file list, large-file finder, manual navigation. Always true. */
  coreAnalysis: boolean;
  /** Multiple saved scans + historical comparison. */
  scanHistory: boolean;
  /** Advanced duplicate detection: unlimited groups, batch review, byte verification. */
  advancedDuplicates: boolean;
  /** Cleanup rules beyond the built-in suggestions. */
  customCleanupRules: boolean;
  /** Scheduled scans. */
  scheduledScans: boolean;
  /** Exportable reports (CSV / JSON / HTML). */
  exportReports: boolean;
  /** Advanced filters (type + age analytics). */
  advancedFilters: boolean;
}

export interface Entitlement {
  state: EntitlementState;
  tier: PlanTier;
  /** Features unlocked for this entitlement. Derived by the backend, never sent by the client. */
  features: FeatureFlags;
  licenseKeyMasked: string | null;
  /**
   * `true` for a one-time purchase: the entitlement itself never expires. A customer who paid once
   * keeps the paid features for the life of the product.
   */
  permanent: boolean;
  /**
   * When the offline validation lease ends (ms), or `null` when the lease is perpetual.
   *
   * This is **not** the entitlement expiry. It is how long a previously verified licence keeps
   * working with no re-validation, which is what lets a refunded or chargebacked licence be
   * withdrawn without an internet connection. A lapsed lease on a permanent licence means
   * "re-verify when you can", never "you no longer own this".
   */
  leaseUntil: number | null;
  /** The licence identifier, when a licence is active. Never an email address. */
  licenseId: string | null;
  /** When the entitlement was last verified locally (ms). */
  verifiedAt: number | null;
  /** How the current entitlement was established. */
  source: 'default' | 'license-key' | 'trial' | 'cache' | 'revocation';
  /** Human-readable reason for the current state, e.g. `Signature verification failed`. */
  reason: string | null;
  /** Days remaining on the offline lease, `null` for a perpetual lease or the free tier. */
  daysRemaining: number | null;
  /** How many devices this licence may be active on at once. */
  deviceLimit: number;
  /** How many devices are currently active. */
  activeDevices: number;
  /** Free-tier duplicate scan limit in groups. `0` = unlimited. */
  duplicateGroupLimit: number;
}

export interface LicenseActivationRequest {
  licenseKey: string;
}

export interface EntitlementEvent {
  at: number;
  from: EntitlementState;
  to: EntitlementState;
  reason: string;
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
  isHidden: boolean;
  /** Raw Windows file attributes, rendered as friendly chips. */
  attributes: string[];
  largestChildren: TreeNode[];
  parentPath: string | null;
  exists: boolean;
}

export interface AppVersion {
  version: string;
  identifier: string;
  platform: string;
  arch: string;
}

/* ------------------------------------------------------------------ *
 * Settings & privacy
 * ------------------------------------------------------------------ */

export type ThemeMode = 'light' | 'dark' | 'system';

export interface AppSettings {
  theme: ThemeMode;
  /** Default root for new scans. */
  defaultScanRoot: string | null;
  /** Max children kept per folder in the tree sent to the UI. */
  maxChildrenPerNode: number;
  /** Follow junctions / symlinks while scanning. */
  followLinks: boolean;
  /** Confirm before any cleanup action, even single items. */
  confirmBeforeCleanup: boolean;
  /** Prefer moving to the Recycle Bin over permanent deletion. */
  preferRecycleBin: boolean;
  /** Minimum age (days) for "large unused file" suggestions. */
  largeUnusedDays: number;
  /** Directories the user opted into as cache locations. */
  cachePaths: string[];
  /** Directories the user opted into as temporary locations. */
  tempPaths: string[];
  /** Never touch these paths. */
  excludePaths: string[];
  /** Optional crash reporting. Off by default. */
  crashReporting: boolean;
  /** Free-tier duplicate group limit (0 = unlimited). */
  duplicateGroupLimit: number;
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
