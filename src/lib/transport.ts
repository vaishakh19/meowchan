/** Native IPC boundary. A normal browser cannot scan local disks. */

import type {
  AppSettings,
  AppVersion,
  ChildrenPage,
  ChildrenQuery,
  DeleteItemsRequest,
  DeleteOutcome,
  DriveInfo,
  DuplicateOptions,
  DuplicateProgress,
  DuplicateReport,
  DuplicateVerifyResult,
  FileDetails,
  LargeFilePage,
  LargeFileQuery,
  PrivacyReport,
  ProStatus,
  ScanGrowthComparison,
  ScanOptions,
  ScanProgress,
  ScanResult,
  ScanSummary,
  StorageBreakdownReport,
  UserStorageReport,
} from '@/types/contracts';

export type Unsubscribe = () => void;

export interface BackendTransport {
  /** Which implementation is active. */
  readonly kind: 'tauri';
  /** Short label shown in the status bar, e.g. `Rust backend`. */
  readonly label: string;
  /**
   * True when the backend can mutate the filesystem. Only the guarded,
   * Pro-gated Recycle Bin deletion can — and it must confirm first.
   */
  readonly canMutate: boolean;

  /* Drives & scanning */
  listDrives(): Promise<DriveInfo[]>;
  storageBreakdown(driveId?: string): Promise<StorageBreakdownReport>;
  pickFolder(): Promise<string | null>;
  scanStart(options: ScanOptions): Promise<string>;
  scanCancel(scanId: string): Promise<void>;
  scanGet(scanId: string): Promise<ScanResult | null>;
  scanList(): Promise<ScanSummary[]>;
  scanChildren(query: ChildrenQuery): Promise<ChildrenPage>;
  scanDelete(scanId: string): Promise<void>;
  userStorageReport(scanId: string): Promise<UserStorageReport>;
  exportScanCsv(request: import('@/types/contracts').CsvExportRequest): Promise<string>;
  compareScans(baselineScanId: string, currentScanId: string): Promise<ScanGrowthComparison>;

  /* Files */
  listLargeFiles(query: LargeFileQuery): Promise<LargeFilePage>;
  fileDetails(path: string): Promise<FileDetails>;
  revealInExplorer(path: string): Promise<void>;
  openPath(path: string): Promise<void>;

  /* Diskora Pro */
  /** Cached licence state; fast, no network. */
  proStatus(): Promise<ProStatus>;
  /** Forces a fresh check with Dodo Payments. */
  proVerify(): Promise<ProStatus>;
  /** Opens the configured Dodo Payments checkout in the browser. */
  proCheckout(): Promise<{ url: string }>;
  proActivate(licenseKey: string): Promise<ProStatus>;
  proRestore(): Promise<ProStatus>;

  /* Safe deletion (Pro) */
  deleteItems(request: DeleteItemsRequest): Promise<DeleteOutcome>;

  /* Duplicates */
  findDuplicates(options: DuplicateOptions): Promise<string>;
  cancelDuplicates(reportId: string): Promise<void>;
  duplicatesGet(reportId: string): Promise<DuplicateReport | null>;
  verifyDuplicates(pathA: string, pathB: string): Promise<DuplicateVerifyResult>;

  /* Settings & privacy */
  settingsGet(): Promise<AppSettings>;
  settingsUpdate(patch: Partial<AppSettings>): Promise<AppSettings>;
  privacyReport(): Promise<PrivacyReport>;
  appVersion(): Promise<AppVersion>;

  /* Events */
  onScanProgress(handler: (progress: ScanProgress) => void): Promise<Unsubscribe>;
  onDuplicateProgress(handler: (progress: DuplicateProgress) => void): Promise<Unsubscribe>;
}

let cached: BackendTransport | null = null;

/** True when running inside a Tauri webview. */
export function isTauriRuntime(): boolean {
  if (typeof window === 'undefined') return false;
  const w = window as unknown as { __TAURI_INTERNALS__?: unknown; isTauri?: boolean };
  return Boolean(w.__TAURI_INTERNALS__ ?? w.isTauri);
}

/**
 * Returns the active transport. The choice is made once per session and is sticky so a
 * mid-session change cannot silently switch the app between a real and a synthetic backend.
 */
export async function getTransport(): Promise<BackendTransport> {
  if (cached) return cached;
  if (isTauriRuntime()) {
    const { createTauriTransport } = await import('./backend/tauriTransport');
    cached = createTauriTransport();
  }
  if (!cached) throw new Error('Run inside the desktop app to scan your local disks. A browser cannot access or analyze them.');
  return cached;
}

/** Test seam: forces a specific transport. */
export function __setTransport(transport: BackendTransport | null): void {
  cached = transport;
}
