/**
 * The single seam between the UI and the outside world.
 *
 * `BackendTransport` describes every operation the app can perform. There are exactly two
 * implementations:
 *
 *  - `TauriTransport` (`./backend/tauriTransport.ts`) — the real product. Talks to the Rust
 *    backend through narrowly-scoped Tauri commands and events.
 *  - `DemoTransport` (`./backend/demo/demoTransport.ts`) — a browser-only fallback used when the
 *    UI is opened in a normal browser (e.g. `npm run dev` previews) where no Tauri runtime
 *    exists. It generates a deterministic synthetic filesystem so the interface can be
 *    reviewed and tested without Windows. It is clearly labelled in the UI and never used in a
 *    packaged build.
 *
 * Nothing else in `src/` is allowed to import `@tauri-apps/*` directly.
 */

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

export type Unsubscribe = () => void;

export interface BackendTransport {
  /** Which implementation is active. */
  readonly kind: 'tauri' | 'demo';
  /** Short label shown in the status bar, e.g. `Rust backend` / `Browser preview`. */
  readonly label: string;
  /** True when destructive operations are actually possible. */
  readonly canMutate: boolean;

  /* Drives & scanning */
  listDrives(): Promise<DriveInfo[]>;
  pickFolder(): Promise<string | null>;
  scanStart(options: ScanOptions): Promise<string>;
  scanCancel(scanId: string): Promise<void>;
  scanGet(scanId: string): Promise<ScanResult | null>;
  scanList(): Promise<ScanSummary[]>;
  scanChildren(scanId: string, path: string, offset: number, limit: number): Promise<ChildrenPage>;
  scanDelete(scanId: string): Promise<void>;

  /* Files */
  listLargeFiles(query: LargeFileQuery): Promise<LargeFilePage>;
  fileDetails(path: string): Promise<FileDetails>;
  revealInExplorer(path: string): Promise<void>;
  openPath(path: string): Promise<void>;

  /* Duplicates */
  findDuplicates(options: DuplicateOptions): Promise<string>;
  duplicatesGet(reportId: string): Promise<DuplicateReport | null>;
  verifyDuplicates(pathA: string, pathB: string): Promise<DuplicateVerifyResult>;

  /* Cleanup */
  cleanupScan(scanId: string, rules: CleanupRule[]): Promise<CleanupFinding[]>;
  cleanupPreview(plan: CleanupPlan): Promise<CleanupPreview>;
  cleanupExecute(plan: CleanupPlan): Promise<CleanupResult>;
  recycleBinSummary(): Promise<RecycleBinSummary>;

  /* Licensing */
  entitlementGet(): Promise<Entitlement>;
  licenseActivate(licenseKey: string): Promise<Entitlement>;
  licenseDeactivate(): Promise<Entitlement>;
  entitlementEvents(): Promise<EntitlementEvent[]>;

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
  } else if (import.meta.env.MODE === 'test') {
    const { createDemoTransport } = await import('./backend/demo/demoTransport');
    cached = createDemoTransport();
  }
  if (!cached) throw new Error('Diskora requires its Windows desktop runtime. Build and launch Diskora.exe; a browser cannot scan your disks.');
  return cached;
}

/** Test seam: forces a specific transport. */
export function __setTransport(transport: BackendTransport | null): void {
  cached = transport;
}
