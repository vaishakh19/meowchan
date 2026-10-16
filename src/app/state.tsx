/**
 * Application state.
 *
 * One reducer owns every piece of UI state; side effects live in the provider as plain async
 * functions that call the transport. Nothing in `src/components` or `src/features` talks to the
 * backend directly — they read state and call actions.
 *
 * Scanning is deliberately *outside* React's render path: progress arrives as events, is stored
 * as a single immutable object, and the visualization re-derives its layout with `useMemo`. A
 * scan of a million files therefore never triggers a million renders.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  type ReactNode,
} from 'react';
import type {
  AppSettings,
  AppVersion,
  DeleteItemsRequest,
  DeleteOutcome,
  DriveInfo,
  DuplicateOptions,
  DuplicateProgress,
  DuplicateReport,
  FileDetails,
  LargeFilePage,
  LargeFileQuery,
  PrivacyReport,
  ProStatus,
  ScanProgress,
  ScanResult,
  ScanSummary,
  TreeNode,
} from '@/types/contracts';
import { getTransport, type BackendTransport, type Unsubscribe } from '@/lib/transport';
import { findNode } from '@/features/visualization/layout';
import { formatBytes } from '@/lib/format';
import { parentPath } from '@/lib/path';
import { nextId } from '@/lib/id';

export type Route =
  | 'home'
  | 'analyze'
  | 'large-files'
  | 'duplicates'
  | 'history'
  | 'pro'
  | 'settings'
  | 'privacy';

export type ViewMode = 'sunburst' | 'treemap' | 'list';
export type GroupBy = 'none' | 'type' | 'folder';
export type SortDirection = 'asc' | 'desc';
export type SortKey = 'size' | 'name' | 'modified' | 'count';

export interface Toast {
  id: string;
  kind: 'info' | 'success' | 'warning' | 'error';
  title: string;
  detail?: string;
  action?: { label: string; run: () => void };
  timeoutMs: number;
}

export interface AppState {
  ready: boolean;
  route: Route;
  transportKind: BackendTransport['kind'];
  canMutate: boolean;
  appVersion: AppVersion | null;

  drives: DriveInfo[];
  selectedDriveId: string | null;
  drivesError: string | null;

  scan: {
    scanning: boolean;
    activeScanId: string | null;
    progress: ScanProgress | null;
    result: ScanResult | null;
    history: ScanSummary[];
    error: string | null;
    cancelling: boolean;
  };

  viz: {
    mode: ViewMode;
    /** Root of the current visualization focus (a folder inside the scan result). */
    focusPath: string | null;
    selectedPath: string | null;
    sort: SortKey;
    sortDir: SortDirection;
    groupBy: GroupBy;
    search: string;
    /** Directory shown in the file list. */
    listPath: string | null;
  };

  largeFiles: {
    query: LargeFileQuery | null;
    page: LargeFilePage | null;
    loading: boolean;
    error: string | null;
  };

  duplicates: {
    options: DuplicateOptions | null;
    report: DuplicateReport | null;
    progress: DuplicateProgress | null;
    loading: boolean;
    error: string | null;
    selectedPaths: string[];
    excludePaths: string[];
  };

  details: {
    path: string | null;
    data: FileDetails | null;
    loading: boolean;
    error: string | null;
  };

  /**
   * Diskora Pro licence state, as verified by the native backend against
   * Dodo Payments. `null` until the first check completes.
   */
  pro: ProStatus | null;

  /** Pending safe-deletion request awaiting its confirmation dialog. */
  deletion: {
    paths: string[] | null;
    working: boolean;
  };

  settings: AppSettings | null;
  privacy: PrivacyReport | null;
  toasts: Toast[];
}

const initialState: AppState = {
  ready: false,
  route: 'home',
  transportKind: 'demo',
  canMutate: false,
  appVersion: null,
  drives: [],
  selectedDriveId: null,
  drivesError: null,
  scan: {
    scanning: false,
    activeScanId: null,
    progress: null,
    result: null,
    history: [],
    error: null,
    cancelling: false,
  },
  viz: {
    mode: 'sunburst',
    focusPath: null,
    selectedPath: null,
    sort: 'size',
    sortDir: 'desc',
    groupBy: 'none',
    search: '',
    listPath: null,
  },
  largeFiles: { query: null, page: null, loading: false, error: null },
  duplicates: {
    options: null,
    report: null,
    progress: null,
    loading: false,
    error: null,
    selectedPaths: [],
    excludePaths: [],
  },
  details: { path: null, data: null, loading: false, error: null },
  pro: null,
  deletion: { paths: null, working: false },
  settings: null,
  privacy: null,
  toasts: [],
};

type Action =
  | { type: 'ready'; kind: BackendTransport['kind']; canMutate: boolean }
  | { type: 'app-version'; version: AppVersion }
  | { type: 'route'; route: Route }
  | { type: 'drives'; drives: DriveInfo[] }
  | { type: 'drives-error'; message: string }
  | { type: 'select-drive'; driveId: string }
  | { type: 'scan-start'; scanId: string }
  | { type: 'scan-progress'; progress: ScanProgress }
  | { type: 'scan-result'; result: ScanResult }
  | { type: 'scan-cancelling' }
  | { type: 'scan-reset' }
  | { type: 'scan-error'; message: string }
  | { type: 'scan-history'; history: ScanSummary[] }
  | { type: 'viz-mode'; mode: ViewMode }
  | { type: 'viz-focus'; path: string | null }
  | { type: 'viz-select'; path: string | null }
  | { type: 'viz-sort'; sort: SortKey; dir: SortDirection }
  | { type: 'viz-group'; groupBy: GroupBy }
  | { type: 'viz-search'; search: string }
  | { type: 'viz-list-path'; path: string | null }
  | { type: 'large-files-query'; query: LargeFileQuery }
  | { type: 'large-files-loading' }
  | { type: 'large-files-page'; page: LargeFilePage }
  | { type: 'large-files-error'; message: string }
  | { type: 'duplicates-options'; options: DuplicateOptions }
  | { type: 'duplicates-loading' }
  | { type: 'duplicates-progress'; progress: DuplicateProgress }
  | { type: 'duplicates-report'; report: DuplicateReport }
  | { type: 'duplicates-error'; message: string }
  | { type: 'duplicates-toggle-path'; path: string }
  | { type: 'duplicates-clear-selection' }
  | { type: 'duplicates-exclude-add'; path: string }
  | { type: 'duplicates-exclude-remove'; path: string }
  | { type: 'details-path'; path: string | null }
  | { type: 'details-loading' }
  | { type: 'details-data'; data: FileDetails }
  | { type: 'details-error'; message: string }
  | { type: 'pro-status'; status: ProStatus }
  | { type: 'deletion-request'; paths: string[] }
  | { type: 'deletion-working'; working: boolean }
  | { type: 'deletion-done' }
  | { type: 'scan-refreshed'; result: ScanResult }
  | { type: 'settings'; settings: AppSettings }
  | { type: 'privacy'; report: PrivacyReport }
  | { type: 'toast-push'; toast: Toast }
  | { type: 'toast-dismiss'; id: string };

function reducer(state: AppState, action: Action): AppState {
  switch (action.type) {
    case 'ready':
      return { ...state, ready: true, transportKind: action.kind, canMutate: action.canMutate };
    case 'app-version':
      return { ...state, appVersion: action.version };
    case 'route':
      return { ...state, route: action.route };
    case 'drives':
      return { ...state, drives: action.drives, drivesError: null };
    case 'drives-error':
      return { ...state, drivesError: action.message };
    case 'select-drive':
      return { ...state, selectedDriveId: action.driveId };
    case 'scan-start':
      return {
        ...state,
        scan: {
          ...state.scan,
          scanning: true,
          cancelling: false,
          activeScanId: action.scanId,
          progress: null,
          error: null,
        },
      };
    case 'scan-progress':
      if (action.progress.scanId !== state.scan.activeScanId) return state;
      return { ...state, scan: { ...state.scan, progress: action.progress } };
    case 'scan-result': {
      const { result } = action;
      return {
        ...state,
        route: state.route === 'home' ? 'analyze' : state.route,
        scan: {
          ...state.scan,
          scanning: false,
          cancelling: false,
          result,
          progress: null,
          activeScanId: result.scanId,
        },
        viz: { ...state.viz, focusPath: result.rootPath, selectedPath: null, listPath: result.rootPath },
        duplicates: { ...state.duplicates, report: null, options: null, selectedPaths: [] },
      };
    }
    case 'scan-cancelling':
      return { ...state, scan: { ...state.scan, cancelling: true } };
    case 'scan-reset':
      return {
        ...state,
        scan: { ...initialState.scan, history: state.scan.history },
        viz: { ...initialState.viz, mode: state.viz.mode },
      };
    case 'scan-error':
      return { ...state, scan: { ...state.scan, scanning: false, cancelling: false, error: action.message } };
    case 'scan-history':
      return { ...state, scan: { ...state.scan, history: action.history } };
    case 'viz-mode':
      return { ...state, viz: { ...state.viz, mode: action.mode } };
    case 'viz-focus':
      return { ...state, viz: { ...state.viz, focusPath: action.path, selectedPath: null, listPath: action.path } };
    case 'viz-select':
      return { ...state, viz: { ...state.viz, selectedPath: action.path } };
    case 'viz-sort':
      return { ...state, viz: { ...state.viz, sort: action.sort, sortDir: action.dir } };
    case 'viz-group':
      return { ...state, viz: { ...state.viz, groupBy: action.groupBy } };
    case 'viz-search':
      return { ...state, viz: { ...state.viz, search: action.search } };
    case 'viz-list-path':
      return { ...state, viz: { ...state.viz, listPath: action.path } };
    case 'large-files-query':
      return { ...state, largeFiles: { ...state.largeFiles, query: action.query, error: null } };
    case 'large-files-loading':
      return { ...state, largeFiles: { ...state.largeFiles, loading: true, error: null } };
    case 'large-files-page':
      return { ...state, largeFiles: { ...state.largeFiles, loading: false, page: action.page } };
    case 'large-files-error':
      return { ...state, largeFiles: { ...state.largeFiles, loading: false, error: action.message } };
    case 'duplicates-options':
      return { ...state, duplicates: { ...state.duplicates, options: action.options, error: null } };
    case 'duplicates-loading':
      return { ...state, duplicates: { ...state.duplicates, loading: true, error: null, report: null } };
    case 'duplicates-progress':
      return { ...state, duplicates: { ...state.duplicates, progress: action.progress } };
    case 'duplicates-report':
      return { ...state, duplicates: { ...state.duplicates, loading: false, report: action.report, progress: null } };
    case 'duplicates-error':
      return { ...state, duplicates: { ...state.duplicates, loading: false, error: action.message } };
    case 'duplicates-toggle-path': {
      const selected = state.duplicates.selectedPaths.includes(action.path)
        ? state.duplicates.selectedPaths.filter((p) => p !== action.path)
        : [...state.duplicates.selectedPaths, action.path];
      return { ...state, duplicates: { ...state.duplicates, selectedPaths: selected } };
    }
    case 'duplicates-clear-selection':
      return { ...state, duplicates: { ...state.duplicates, selectedPaths: [] } };
    case 'duplicates-exclude-add':
      return {
        ...state,
        duplicates: {
          ...state.duplicates,
          excludePaths: state.duplicates.excludePaths.includes(action.path)
            ? state.duplicates.excludePaths
            : [...state.duplicates.excludePaths, action.path],
        },
      };
    case 'duplicates-exclude-remove':
      return {
        ...state,
        duplicates: {
          ...state.duplicates,
          excludePaths: state.duplicates.excludePaths.filter((p) => p !== action.path),
        },
      };
    case 'details-path':
      return { ...state, details: { ...state.details, path: action.path, data: null, error: null } };
    case 'details-loading':
      return { ...state, details: { ...state.details, loading: true } };
    case 'details-data':
      return { ...state, details: { ...state.details, loading: false, data: action.data } };
    case 'details-error':
      return { ...state, details: { ...state.details, loading: false, error: action.message } };
    case 'pro-status':
      return { ...state, pro: action.status };
    case 'deletion-request':
      return { ...state, deletion: { paths: action.paths, working: false } };
    case 'deletion-working':
      return { ...state, deletion: { ...state.deletion, working: action.working } };
    case 'deletion-done':
      return { ...state, deletion: { paths: null, working: false } };
    case 'scan-refreshed': {
      // A post-deletion refresh: the result changes, but the user's place
      // in the visualization must not move (unlike a brand-new scan).
      return { ...state, scan: { ...state.scan, result: action.result } };
    }
    case 'settings':
      return { ...state, settings: action.settings };
    case 'privacy':
      return { ...state, privacy: action.report };
    case 'toast-push':
      return { ...state, toasts: [...state.toasts.slice(-4), action.toast] };
    case 'toast-dismiss':
      return { ...state, toasts: state.toasts.filter((toast) => toast.id !== action.id) };
    default:
      return state;
  }
}

export interface AppActions {
  navigate(route: Route): void;
  refreshDrives(): Promise<void>;
  selectDrive(driveId: string): void;
  scanDrive(driveId?: string): Promise<void>;
  scanPath(path: string): Promise<void>;
  pickAndScanFolder(): Promise<void>;
  cancelScan(): Promise<void>;
  clearScan(): void;
  setViewMode(mode: ViewMode): void;
  focusPath(path: string | null): void;
  selectNode(path: string | null): void;
  setSort(sort: SortKey, dir?: SortDirection): void;
  setGroupBy(groupBy: GroupBy): void;
  setSearch(search: string): void;
  openListPath(path: string): void;
  runLargeFileQuery(patch?: Partial<LargeFileQuery>): Promise<void>;
  selectDetails(path: string | null): Promise<void>;
  reveal(path: string): Promise<void>;
  open(path: string): Promise<void>;
  copyPath(path: string): Promise<void>;
  runDuplicateScan(patch?: Partial<DuplicateOptions>): Promise<void>;
  toggleDuplicatePath(path: string): void;
  clearDuplicateSelection(): void;
  excludeDuplicatePath(path: string): void;
  removeDuplicateExclusion(path: string): void;
  updateSettings(patch: Partial<AppSettings>): Promise<void>;

  /* Diskora Pro */
  /** Loads the cached licence state (no network). */
  refreshProStatus(): Promise<void>;
  /** Forces a fresh check with Dodo Payments. */
  verifyPro(): Promise<void>;
  /** Opens the Dodo Payments checkout for the one-time $12.99 purchase. */
  openProCheckout(): Promise<void>;
  activateLicense(licenseKey: string): Promise<void>;
  /** Re-validates the stored purchase — the restore-purchases flow. */
  restorePurchases(): Promise<void>;
  /** Explains that a feature is Pro and offers the upgrade path. */
  promptUpgrade(feature: string): void;

  /* Safe deletion (Pro) */
  /** Stores the selection and opens the confirmation dialog. */
  requestDeletion(paths: string[]): void;
  cancelDeletion(): void;
  /** Confirmed deletion: Recycle Bin via the native backend, then refresh. */
  confirmDeletion(): Promise<void>;

  toast(toast: Omit<Toast, 'id' | 'timeoutMs'> & { timeoutMs?: number }): void;
  dismissToast(id: string): void;
}

interface AppContextValue {
  state: AppState;
  actions: AppActions;
  transport: BackendTransport | null;
}

const AppContext = createContext<AppContextValue | null>(null);

export function AppStateProvider({ children }: { children: ReactNode }): React.JSX.Element {
  const [state, dispatch] = useReducer(reducer, initialState);
  const transportRef = useRef<BackendTransport | null>(null);
  const stateRef = useRef(state);
  stateRef.current = state;

  const toast = useCallback((toast: Omit<Toast, 'id' | 'timeoutMs'> & { timeoutMs?: number }) => {
    const full: Toast = {
      ...toast,
      id: nextId('toast'),
      timeoutMs: toast.timeoutMs ?? (toast.kind === 'error' ? 9000 : 5000),
    };
    dispatch({ type: 'toast-push', toast: full });
    if (full.timeoutMs > 0) {
      window.setTimeout(() => dispatch({ type: 'toast-dismiss', id: full.id }), full.timeoutMs);
    }
  }, []);

  /* ------------------------------- bootstrap ------------------------------- */
  useEffect(() => {
    let cancelled = false;
    const subscriptions: Unsubscribe[] = [];
    const subscribe = (unsubscribe: Unsubscribe) => {
      if (cancelled) unsubscribe();
      else subscriptions.push(unsubscribe);
    };
    void (async () => {
      const transport = await getTransport();
      if (cancelled) return;
      transportRef.current = transport;

      subscribe(await transport.onScanProgress((progress) => {
        if (!cancelled) dispatch({ type: 'scan-progress', progress });
      }));
      subscribe(await transport.onDuplicateProgress((progress) => {
        if (!cancelled) dispatch({ type: 'duplicates-progress', progress });
      }));
      if (cancelled) return;

      const [drives, settings, version, privacy, history, pro] = await Promise.all([
        transport.listDrives().catch(() => [] as DriveInfo[]),
        transport.settingsGet().catch(error => {
          if (!cancelled) toast({ kind: 'warning', title: 'Preferences are unavailable', detail: describe(error) });
          return null;
        }),
        transport.appVersion().catch(() => null),
        transport.privacyReport().catch(() => null),
        transport.scanList().catch(() => [] as ScanSummary[]),
        transport.proStatus().catch(() => null),
      ]);
      if (cancelled) return;
      if (pro) dispatch({ type: 'pro-status', status: pro });
      dispatch({ type: 'drives', drives });
      if (settings) dispatch({ type: 'settings', settings });
      if (version) dispatch({ type: 'app-version', version });
      if (privacy) dispatch({ type: 'privacy', report: privacy });
      dispatch({ type: 'scan-history', history });
      const system = drives.find((drive) => drive.isSystemDrive) ?? drives[0];
      if (system) dispatch({ type: 'select-drive', driveId: system.id });
      dispatch({ type: 'ready', kind: transport.kind, canMutate: transport.canMutate });
    })().catch((error: unknown) => {
      if (!cancelled) dispatch({ type: 'drives-error', message: describe(error) });
      subscriptions.splice(0).forEach((unsubscribe) => unsubscribe());
    });
    return () => {
      cancelled = true;
      subscriptions.splice(0).forEach((unsubscribe) => unsubscribe());
    };
  }, [toast]);

  /* -------------------------------- theme -------------------------------- */
  useEffect(() => {
    const theme = state.settings?.theme ?? 'system';
    const root = document.documentElement;
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const apply = () => {
      const dark = theme === 'dark' || (theme === 'system' && media.matches);
      root.classList.toggle('dark', dark);
      root.style.colorScheme = dark ? 'dark' : 'light';
    };
    apply();
    media.addEventListener('change', apply);
    return () => media.removeEventListener('change', apply);
  }, [state.settings?.theme]);

  /* ------------------------------- actions ------------------------------- */
  const actions = useMemo<AppActions>(() => {
    const requireTransport = (): BackendTransport => {
      const transport = transportRef.current;
      if (!transport) throw new Error('The backend is still starting up.');
      return transport;
    };

    return {
      navigate: (route) => dispatch({ type: 'route', route }),

      refreshDrives: async () => {
        try {
          dispatch({ type: 'drives', drives: await requireTransport().listDrives() });
        } catch (error) {
          dispatch({ type: 'drives-error', message: describe(error) });
        }
      },

      selectDrive: (driveId) => dispatch({ type: 'select-drive', driveId }),

      scanDrive: async (driveId?: string) => {
        const id = driveId ?? stateRef.current.selectedDriveId;
        const drive = stateRef.current.drives.find((d) => d.id === id) ?? stateRef.current.drives[0];
        if (!drive) {
          toast({ kind: 'warning', title: 'No drive selected', detail: 'Pick a drive first.' });
          return;
        }
        await actions.scanPath(drive.path);
      },

      pickAndScanFolder: async () => {
        try {
          const path = await requireTransport().pickFolder();
          if (path) await actions.scanPath(path);
        } catch (error) {
          toast({ kind: 'error', title: 'Could not select a folder', detail: describe(error) });
        }
      },

      scanPath: async (path: string) => {
        const settings = stateRef.current.settings;
        try {
          const scanId = await requireTransport().scanStart({
            rootPath: path,
            followLinks: settings?.followLinks ?? false,
            maxChildrenPerNode: settings?.maxChildrenPerNode ?? 100,
          });
          dispatch({ type: 'scan-start', scanId });
          dispatch({ type: 'route', route: 'analyze' });
        } catch (error) {
          dispatch({ type: 'scan-error', message: describe(error) });
          toast({ kind: 'error', title: 'Could not start the scan', detail: describe(error) });
        }
      },

      cancelScan: async () => {
        const scanId = stateRef.current.scan.activeScanId;
        if (!scanId) return;
        dispatch({ type: 'scan-cancelling' });
        try {
          await requireTransport().scanCancel(scanId);
        } catch (error) {
          toast({ kind: 'error', title: 'Could not cancel the scan', detail: describe(error) });
        }
      },

      clearScan: () => dispatch({ type: 'scan-reset' }),

      setViewMode: (mode) => dispatch({ type: 'viz-mode', mode }),
      focusPath: (path) => dispatch({ type: 'viz-focus', path }),
      selectNode: (path) => dispatch({ type: 'viz-select', path }),
      setSort: (sort, dir) =>
        dispatch({
          type: 'viz-sort',
          sort,
          dir:
            dir ??
            (stateRef.current.viz.sort === sort && stateRef.current.viz.sortDir === 'desc'
              ? 'asc'
              : 'desc'),
        }),
      setGroupBy: (groupBy) => dispatch({ type: 'viz-group', groupBy }),
      setSearch: (search) => dispatch({ type: 'viz-search', search }),
      openListPath: (path) => dispatch({ type: 'viz-list-path', path }),

      runLargeFileQuery: async (patch) => {
        const scanId = stateRef.current.scan.result?.scanId;
        if (!scanId) return;
        const query: LargeFileQuery = {
          scanId,
          minBytes: 100 * 1024 * 1024,
          sort: 'size-desc',
          limit: 250,
          offset: 0,
          ...(stateRef.current.largeFiles.query ?? {}),
          ...patch,
        };
        dispatch({ type: 'large-files-query', query });
        dispatch({ type: 'large-files-loading' });
        try {
          dispatch({ type: 'large-files-page', page: await requireTransport().listLargeFiles(query) });
        } catch (error) {
          dispatch({ type: 'large-files-error', message: describe(error) });
        }
      },

      selectDetails: async (path) => {
        if (!path) {
          dispatch({ type: 'details-path', path: null });
          return;
        }
        dispatch({ type: 'details-path', path });
        // Synthetic aggregate nodes have no filesystem entry to describe.
        if (path.endsWith('[Other]')) return;
        dispatch({ type: 'details-loading' });
        try {
          dispatch({ type: 'details-data', data: await requireTransport().fileDetails(path) });
        } catch {
          dispatch({ type: 'details-error', message: 'Details are unavailable for this item.' });
        }
      },

      reveal: async (path) => {
        try {
          await requireTransport().revealInExplorer(path);
        } catch (error) {
          toast({ kind: 'error', title: 'Could not open File Explorer', detail: describe(error) });
        }
      },

      open: async (path) => {
        try {
          await requireTransport().openPath(path);
        } catch (error) {
          toast({ kind: 'error', title: 'Could not open the item', detail: describe(error) });
        }
      },

      copyPath: async (path) => {
        // Full-path viewing and copying is a Pro feature; the gate is
        // enforced here as well as in the UI so no surface can bypass it.
        if (!stateRef.current.pro?.enabled) {
          actions.promptUpgrade('Full path copying');
          return;
        }
        try {
          await navigator.clipboard.writeText(path);
          toast({ kind: 'success', title: 'Path copied', detail: path, timeoutMs: 3000 });
        } catch {
          toast({ kind: 'warning', title: 'Could not copy the path', detail: path });
        }
      },

      runDuplicateScan: async (patch) => {
        const scanId = stateRef.current.scan.result?.scanId;
        if (!scanId) {
          toast({
            kind: 'info',
            title: 'Scan a drive first',
            detail: 'Duplicate detection works on scan results.',
          });
          return;
        }
        const options: DuplicateOptions = {
          scanId,
          minBytes: 1024,
          maxBytes: 5 * 1024 * 1024 * 1024,
          algorithm: 'sha256',
          excludePaths: stateRef.current.duplicates.excludePaths,
          ...(stateRef.current.duplicates.options ?? {}),
          ...patch,
        };
        dispatch({ type: 'duplicates-options', options });
        dispatch({ type: 'duplicates-loading' });
        try {
          const reportId = await requireTransport().findDuplicates(options);
          const transport = requireTransport();
          const poll = async () => {
            const report = await transport.duplicatesGet(reportId);
            if (report && report.status !== 'running') {
              dispatch({ type: 'duplicates-report', report });
              if (report.totalGroups === 0) {
                toast({
                  kind: 'info',
                  title: 'No duplicates found',
                  detail: 'Nothing matched at this size threshold.',
                });
              }
              return;
            }
            window.setTimeout(() => void poll().catch(error => {
              dispatch({ type: 'duplicates-error', message: describe(error) });
            }), 250);
          };
          await poll();
        } catch (error) {
          dispatch({ type: 'duplicates-error', message: describe(error) });
        }
      },

      toggleDuplicatePath: (path) => dispatch({ type: 'duplicates-toggle-path', path }),
      clearDuplicateSelection: () => dispatch({ type: 'duplicates-clear-selection' }),
      excludeDuplicatePath: (path) => dispatch({ type: 'duplicates-exclude-add', path }),
      removeDuplicateExclusion: (path) => dispatch({ type: 'duplicates-exclude-remove', path }),

      updateSettings: async (patch) => {
        const previous = stateRef.current.settings;
        dispatch({
          type: 'settings',
          settings: { ...(previous ?? ({} as AppSettings)), ...patch } as AppSettings,
        });
        try {
          dispatch({ type: 'settings', settings: await requireTransport().settingsUpdate(patch) });
        } catch (error) {
          if (previous) dispatch({ type: 'settings', settings: previous });
          toast({ kind: 'error', title: 'Could not save settings', detail: describe(error) });
        }
      },

      /* ------------------------------- Diskora Pro ------------------------------- */

      refreshProStatus: async () => {
        try {
          dispatch({ type: 'pro-status', status: await requireTransport().proStatus() });
        } catch (error) {
          toast({ kind: 'warning', title: 'Could not check Diskora Pro status', detail: describe(error) });
        }
      },

      verifyPro: async () => {
        try {
          dispatch({ type: 'pro-status', status: await requireTransport().proVerify() });
        } catch (error) {
          toast({ kind: 'error', title: 'Could not verify the purchase', detail: describe(error) });
          throw error;
        }
      },

      openProCheckout: async () => {
        try {
          const { url } = await requireTransport().proCheckout();
          toast({
            kind: 'info',
            title: 'Checkout opened in your browser',
            detail: url,
            timeoutMs: 8000,
          });
          await actions.refreshProStatus();
        } catch (error) {
          toast({ kind: 'error', title: 'Could not open the checkout', detail: describe(error) });
        }
      },

      activateLicense: async (licenseKey) => {
        try {
          const status = await requireTransport().proActivate(licenseKey.trim());
          dispatch({ type: 'pro-status', status });
          if (status.enabled) {
            toast({ kind: 'success', title: 'Diskora Pro is active', detail: 'Both Pro features are unlocked on this device.' });
          } else {
            toast({ kind: 'error', title: 'Activation failed', detail: status.message });
          }
        } catch (error) {
          toast({ kind: 'error', title: 'Activation failed', detail: describe(error) });
          throw error;
        }
      },

      restorePurchases: async () => {
        try {
          const status = await requireTransport().proRestore();
          dispatch({ type: 'pro-status', status });
          if (status.enabled) {
            toast({ kind: 'success', title: 'Purchase restored', detail: status.message });
          } else {
            toast({
              kind: 'info',
              title: 'No active purchase found',
              detail: 'Dodo Payments reports no valid Diskora Pro licence for this device. Activate it with your license key instead.',
            });
          }
        } catch (error) {
          toast({ kind: 'error', title: 'Could not restore purchases', detail: describe(error) });
          throw error;
        }
      },

      promptUpgrade: (feature) => {
        toast({
          kind: 'info',
          title: `${feature} is a Diskora Pro feature`,
          detail: 'Upgrade once to unlock it — $12.99, no subscription.',
          timeoutMs: 7000,
          action: { label: 'Upgrade to Pro', run: () => actions.navigate('pro') },
        });
      },

      /* ------------------------------ safe deletion ------------------------------ */

      requestDeletion: (paths) => {
        if (paths.length === 0) return;
        if (!stateRef.current.pro?.enabled) {
          actions.promptUpgrade('Safe file deletion');
          return;
        }
        dispatch({ type: 'deletion-request', paths });
      },

      cancelDeletion: () => dispatch({ type: 'deletion-done' }),

      confirmDeletion: async () => {
        const paths = stateRef.current.deletion.paths;
        const scanId = stateRef.current.scan.activeScanId;
        if (!paths || paths.length === 0 || !scanId) {
          dispatch({ type: 'deletion-done' });
          return;
        }
        dispatch({ type: 'deletion-working', working: true });
        try {
          const transport = requireTransport();
          const request: DeleteItemsRequest = { scanId, paths };
          const outcome: DeleteOutcome = await transport.deleteItems(request);

          if (outcome.deleted.length > 0) {
            const reclaimed = outcome.deleted.reduce((sum, item) => sum + item.sizeBytes, 0);
            toast({
              kind: 'success',
              title:
                outcome.deleted.length === 1
                  ? `Moved “${outcome.deleted[0].name}” to the Recycle Bin`
                  : `Moved ${outcome.deleted.length} items to the Recycle Bin`,
              detail: `${formatBytes(reclaimed)} reclaimed — restore them from the Windows Recycle Bin if you change your mind.`,
              timeoutMs: 7000,
            });

            // Refresh the scan results from the updated index.
            const refreshed = await transport.scanGet(scanId);
            if (refreshed) dispatch({ type: 'scan-refreshed', result: refreshed });

            // If the selection was deleted, move it to the nearest surviving parent.
            const { selectedPath } = stateRef.current.viz;
            if (selectedPath && paths.some((path) => selectedPath === path || selectedPath.startsWith(`${path}\\`))) {
              const surviving = paths
              .map((path) => parentPath(path))
              .find(
                (parent): parent is string =>
                  parent !== null && parent.length >= 3 && !paths.includes(parent),
              );
              actions.selectNode(surviving ?? null);
            }
          }

          if (outcome.failed.length > 0) {
            const first = outcome.failed[0];
            toast({
              kind: 'error',
              title:
                outcome.failed.length === 1
                  ? `Could not move “${first.path.split('\\').pop()}” to the Recycle Bin`
                  : `${outcome.failed.length} items could not be moved`,
              detail: `${first.reason}${outcome.failed.length > 1 ? ` (and ${outcome.failed.length - 1} more)` : ''}`,
              timeoutMs: 9000,
            });
          }
        } catch (error) {
          toast({ kind: 'error', title: 'Deletion failed', detail: describe(error) });
        } finally {
          dispatch({ type: 'deletion-done' });
        }
      },

      toast,
      dismissToast: (id) => dispatch({ type: 'toast-dismiss', id }),
    };
  }, [toast]);

  // Poll results as well as receiving progress. Very short scans can finish before
  // scanStart's IPC response arrives; relying solely on a final event loses that race.
  useEffect(() => {
    const { scanning, activeScanId } = state.scan;
    const transport = transportRef.current;
    if (!scanning || !activeScanId || !transport) return;
    let cancelled = false;
    let busy = false;
    const poll = async () => {
      if (busy || cancelled) return;
      busy = true;
      try {
        const result = await transport.scanGet(activeScanId);
        if (cancelled || !result) return;
        dispatch({ type: 'scan-result', result });
        const history = await transport.scanList();
        if (!cancelled) dispatch({ type: 'scan-history', history });
      } catch (error) {
        if (!cancelled) dispatch({ type: 'scan-error', message: describe(error) });
      } finally { busy = false; }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 250);
    return () => { cancelled = true; window.clearInterval(timer); };
  }, [state.scan.scanning, state.scan.activeScanId]);

  const value = useMemo<AppContextValue>(
    () => ({ state, actions, transport: transportRef.current }),
    [state, actions],
  );

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const context = useContext(AppContext);
  if (!context) throw new Error('useApp must be used inside <AppStateProvider>.');
  return context;
}

/** Convenience selector for the node currently focused in the visualization. */
export function useFocusNode(): TreeNode | null {
  const { state } = useApp();
  return useMemo(() => {
    const tree = state.scan.result?.tree;
    if (!tree) return null;
    return findNode(tree, state.viz.focusPath ?? tree.path) ?? tree;
  }, [state.scan.result, state.viz.focusPath]);
}

export function describe(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  try {
    return JSON.stringify(error);
  } catch {
    return 'Unknown error';
  }
}
