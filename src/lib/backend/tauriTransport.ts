/**
 * Tauri transport — the real backend binding.
 *
 * Every call maps to exactly one narrowly-scoped Rust command in `src-tauri/src/commands/`.
 * Argument names are camelCase on this side and are converted to snake_case by Tauri.
 *
 * This module is the ONLY place in `src/` that imports `@tauri-apps/*`. It is loaded lazily so a
 * browser build never pulls the Tauri runtime in.
 */

import type { Event } from '@tauri-apps/api/event';
import type {
  AppSettings,
  AppVersion,
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
  FileDetails,
  LargeFilePage,
  LargeFileQuery,
  PrivacyReport,
  RecycleBinSummary,
  ScanOptions,
  ScanProgress,
  ScanResult,
  ScanSummary,
} from '@/types/contracts';
import type { BackendTransport, Unsubscribe } from '@/lib/transport';

type Args = Record<string, unknown>;

async function invoke<T>(command: string, args?: Args): Promise<T> {
  const core = await import('@tauri-apps/api/core');
  return core.invoke<T>(command, args);
}

async function listen<T>(event: string, handler: (payload: T) => void): Promise<Unsubscribe> {
  const api = await import('@tauri-apps/api/event');
  const unsubscribe = await api.listen<T>(event, (raw: Event<T>) => handler(raw.payload));
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
    canMutate: true,

    /* Drives & scanning */
    listDrives: () => invoke<DriveInfo[]>('list_drives'),
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

    /* Cleanup */
    cleanupScan: (scanId: string, rules: CleanupRule[]) => invoke<CleanupFinding[]>('cleanup_scan', { scanId, rules }),
    cleanupPreview: (plan: CleanupPlan) => invoke<CleanupPreview>('cleanup_preview', { plan }),
    cleanupExecute: (plan: CleanupPlan) => invoke<CleanupResult>('cleanup_execute', { plan }),
    recycleBinSummary: () => invoke<RecycleBinSummary>('recycle_bin_summary'),

    /* Licensing */
    entitlementGet: () => invoke<Entitlement>('entitlement_get'),
    licenseActivate: (licenseKey: string) => invoke<Entitlement>('license_activate', { licenseKey }),
    licenseDeactivate: () => invoke<Entitlement>('license_deactivate'),
    entitlementEvents: () => invoke<EntitlementEvent[]>('entitlement_events'),

    /* Settings & privacy */
    settingsGet: () => invoke<AppSettings>('settings_get'),
    settingsUpdate: (patch: Partial<AppSettings>) => invoke<AppSettings>('settings_update', { patch }),
    privacyReport: () => invoke<PrivacyReport>('privacy_report'),
    appVersion: () => invoke<AppVersion>('app_version'),

    /* Events */
    onScanProgress: (handler: (progress: ScanProgress) => void) =>
      listen<ScanProgress>(SCAN_PROGRESS_EVENT, handler),
    onDuplicateProgress: (handler: (progress: DuplicateProgress) => void) =>
      listen<DuplicateProgress>(DUPLICATE_PROGRESS_EVENT, handler),
  };
}
