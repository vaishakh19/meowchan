/**
 * Tauri transport — the real backend binding.
 *
 * Every call maps to exactly one narrowly-scoped Rust command in `src-tauri/src/commands/`.
 * Argument names are camelCase on this side and are converted to snake_case by Tauri.
 *
 * This module is the ONLY place in `src/` that imports `@tauri-apps/*`. It is loaded lazily so a
 * browser build never pulls the Tauri runtime in.
 */

import { invoke as coreInvoke } from '@tauri-apps/api/core';
import { listen as coreListen, type Event } from '@tauri-apps/api/event';
import type {
  AppSettings,
  AppVersion,
  ChildrenPage,
  CsvExportRequest,
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
import type { BackendTransport, Unsubscribe } from '@/lib/transport';

type Args = Record<string, unknown>;

// The Tauri APIs are imported statically on purpose: this whole module is only
// ever loaded lazily (see lib/transport.ts), so a browser build never pulls the
// Tauri runtime in. Static imports also keep every call on one module instance.
async function invoke<T>(command: string, args?: Args): Promise<T> {
  return coreInvoke<T>(command, args);
}

async function listen<T>(event: string, handler: (payload: T) => void): Promise<Unsubscribe> {
  const unsubscribe = await coreListen<T>(event, (raw: Event<T>) => handler(raw.payload));
  return () => {
    unsubscribe();
  };
}

export const SCAN_PROGRESS_EVENT = 'scan://progress';
export const DUPLICATE_PROGRESS_EVENT = 'duplicates://progress';

export function createTauriTransport(): BackendTransport {
  return {
    kind: 'tauri',
    label: 'Rust backend',
    // Only the guarded, Pro-gated Recycle Bin deletion mutates the filesystem.
    canMutate: true,

    /* Drives & scanning */
    listDrives: () => invoke<DriveInfo[]>('list_drives'),
    storageBreakdown: (driveId?: string) => invoke<StorageBreakdownReport>('storage_breakdown', { driveId }),
    pickFolder: async () => {
      const dialog = await import('@tauri-apps/plugin-dialog');
      const result = await dialog.open({ directory: true, multiple: false, title: 'Select a folder to scan' });
      return typeof result === 'string' ? result : null;
    },
    scanStart: (options: ScanOptions) => invoke<string>('scan_start', { options }),
    scanCancel: (scanId: string) => invoke<void>('scan_cancel', { scanId }),
    scanGet: (scanId: string) => invoke<ScanResult | null>('scan_get', { scanId }),
    scanList: () => invoke<ScanSummary[]>('scan_list'),
    scanChildren: (scanId: string, path: string, offset: number, limit: number) =>
      invoke<ChildrenPage>('scan_children', { scanId, path, offset, limit }),
    scanDelete: (scanId: string) => invoke<void>('scan_delete', { scanId }),
    userStorageReport: (scanId: string) => invoke<UserStorageReport>('user_storage_report', { scanId }),
    exportScanCsv: (request: CsvExportRequest) => invoke<string>('export_scan_csv', { request }),
    compareScans: (baselineScanId: string, currentScanId: string) =>
      invoke<ScanGrowthComparison>('compare_scans', { baselineScanId, currentScanId }),

    /* Files */
    listLargeFiles: (query: LargeFileQuery) => invoke<LargeFilePage>('list_large_files', { query }),
    fileDetails: (path: string) => invoke<FileDetails>('file_details', { path }),
    revealInExplorer: (path: string) => invoke<void>('reveal_in_explorer', { path }),
    openPath: (path: string) => invoke<void>('open_path', { path }),

    /* Duplicates */
    findDuplicates: (options: DuplicateOptions) => invoke<string>('find_duplicates', { options }),
    duplicatesGet: (reportId: string) => invoke<DuplicateReport | null>('duplicates_get', { reportId }),
    verifyDuplicates: (pathA: string, pathB: string) =>
      invoke<DuplicateVerifyResult>('verify_duplicates', { pathA, pathB }),

    /* Settings & privacy */
    settingsGet: () => invoke<AppSettings>('settings_get'),
    settingsUpdate: (patch: Partial<AppSettings>) => invoke<AppSettings>('settings_update', { patch }),
    privacyReport: () => invoke<PrivacyReport>('privacy_report'),
    appVersion: () => invoke<AppVersion>('app_version'),

    /* Diskora Pro */
    proStatus: () => invoke<ProStatus>('pro_status'),
    proVerify: () => invoke<ProStatus>('pro_verify'),
    proCheckout: () => invoke<{ url: string }>('pro_checkout'),
    proActivate: (licenseKey: string) => invoke<ProStatus>('pro_activate', { licenseKey }),
    proRestore: () => invoke<ProStatus>('pro_restore'),

    /* Safe deletion (Pro) */
    deleteItems: (request: DeleteItemsRequest) => invoke<DeleteOutcome>('delete_items', { request }),

    /* Events */
    onScanProgress: (handler: (progress: ScanProgress) => void) =>
      listen<ScanProgress>(SCAN_PROGRESS_EVENT, handler),
    onDuplicateProgress: (handler: (progress: DuplicateProgress) => void) =>
      listen<DuplicateProgress>(DUPLICATE_PROGRESS_EVENT, handler),
  };
}
