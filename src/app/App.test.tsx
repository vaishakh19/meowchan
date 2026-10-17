import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from './App';
import { __setTransport } from '@/lib/transport';
import { createTauriTransport } from '@/lib/backend/tauriTransport';
const { invoke, open, listen } = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

const LOCKED_PRO_STATUS = {
  enabled: false,
  configured: true,
  licenseKey: null,
  activatedAt: null,
  lastValidatedAt: null,
  offline: false,
  message: 'Diskora Pro is not active on this device.',
};

beforeEach(() => {
  invoke.mockReset(); open.mockReset(); listen.mockReset();
  listen.mockResolvedValue(vi.fn());
  invoke.mockImplementation(async (command: string) => {
    if (command === 'settings_get') return { theme: 'system', followLinks: false, maxChildrenPerNode: 100 };
    if (command === 'list_drives') return [];
    if (command === 'scan_list') return [];
    if (command === 'pro_status') return LOCKED_PRO_STATUS;
    return null;
  });
  __setTransport(createTauriTransport());
});
afterEach(() => { cleanup(); __setTransport(null); });

describe('desktop shell with mocked IPC (not a real filesystem test)', () => {
  it('shows a desktop-only message in a plain browser instead of inventing scan data', async () => {
    __setTransport(null);
    render(<App />);
    expect(await screen.findByRole('heading', { name: 'Run inside the desktop app' })).toBeDefined();
    expect(invoke.mock.calls.some(([command]) => command === 'list_drives')).toBe(false);
  });

  it('renders the analyzer with free features intact and an upgrade path', async () => {
    render(<App />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('list_drives', undefined));
    // Free capabilities stay reachable without payment.
    expect(screen.getByRole('button', { name: /duplicate finder/i })).toBeDefined();
    expect(screen.getByRole('button', { name: /scan drive/i })).toBeDefined();
    // The upgrade path is visible; nothing is unlocked by its presence.
    expect(screen.getByRole('button', { name: /upgrade to pro/i })).toBeDefined();
    expect(screen.getByRole('button', { name: /diskora pro/i })).toBeDefined();
  });

  it('fetches the licence state at startup and keeps Pro locked without a purchase', async () => {
    render(<App />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('pro_status', undefined));
    // The locked licence state never enables Pro-only behaviour.
    await waitFor(() => expect(screen.getByTestId('upgrade-to-pro')).toBeDefined());
    expect(screen.queryByTestId('pro-badge')).toBeNull();
  });

  it('opens a folder picker, then sends the chosen path to Rust', async () => {
    open.mockResolvedValue('C:\\\\chosen');
    render(<App />);
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: /scan a folder/i }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('scan_start', { options: {
      rootPath: 'C:\\\\chosen', followLinks: false, maxChildrenPerNode: 100,
    } }));
  });

  it('does not scan when the folder dialog is cancelled', async () => {
    open.mockResolvedValue(null);
    render(<App />);
    await userEvent.setup().click(screen.getByRole('button', { name: /scan a folder/i }));
    expect(invoke.mock.calls.filter(([command]) => command === 'scan_start')).toHaveLength(0);
  });

  it('shows a native error rather than claiming a scan succeeded', async () => {
    open.mockRejectedValue(new Error('Access denied'));
    render(<App />);
    await userEvent.setup().click(screen.getByRole('button', { name: /scan a folder/i }));
    await waitFor(() => expect(screen.getByText('Could not select a folder')).toBeDefined());
    expect(screen.getByText('Access denied')).toBeDefined();
  });
});

it('fetches a completed short scan even when no progress event was received', async () => {
  open.mockResolvedValue('C:\\\\chosen');
  const root = { id: 'root', path: 'C:\\\\chosen', name: 'chosen', kind: 'folder',
    sizeBytes: 10, diskSizeBytes: null, fileCount: 1, folderCount: 0, modifiedAt: null,
    createdAt: null, extension: null, category: 'other', accessDenied: false, children: [], synthetic: false };
  const fallback = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command, args) => {
    if (command === 'scan_start') return 'instant';
    if (command === 'scan_get') return { scanId: 'instant', rootPath: root.path, rootName: root.name,
      startedAt: 0, finishedAt: 1, status: 'complete', tree: root, totalBytes: 10,
      diskTotalBytes: null, fileCount: 1, folderCount: 0, errors: [], truncated: false,
      prunedNodes: 0, durationMs: 1, expectedBytes: null };
    return fallback(command, args);
  });
  render(<App />);
  await userEvent.setup().click(screen.getByRole('button', { name: /scan a folder/i }));
  await waitFor(() => expect(screen.getByTestId('analyzed-size').textContent).toMatch(/10\s*B/));
});

it('offers byte-for-byte verification for every reported duplicate copy', async () => {
  open.mockResolvedValue('C:\\chosen');
  const root = {
    id: 'root', path: 'C:\\chosen', name: 'chosen', kind: 'folder', sizeBytes: 10,
    diskSizeBytes: null, fileCount: 2, folderCount: 0, modifiedAt: null,
    createdAt: null, extension: null, category: 'other', accessDenied: false,
    children: [], synthetic: false,
  };
  const scanResult = {
    scanId: 'scan-verify', rootPath: root.path, rootName: root.name,
    startedAt: 0, finishedAt: 1, status: 'complete', tree: root, totalBytes: 10,
    diskTotalBytes: null, fileCount: 2, folderCount: 0, errors: [], truncated: false,
    prunedNodes: 0, durationMs: 1, expectedBytes: null,
  };
  const duplicateReport = {
    reportId: 'report-verify', scanId: 'scan-verify', startedAt: 0, finishedAt: 1,
    status: 'complete', algorithm: 'sha256',
    groups: [{
      groupId: 'group-1', sizeBytes: 10, reclaimableBytes: 10, hash: 'sha256:match',
      copies: [
        { path: 'C:\\chosen\\a.txt', name: 'a.txt', sizeBytes: 10, modifiedAt: 0, isSuggestedOriginal: true },
        { path: 'C:\\chosen\\b.txt', name: 'b.txt', sizeBytes: 10, modifiedAt: 1, isSuggestedOriginal: false }
      ],
    }],
    candidateGroups: 1, totalGroups: 1, totalCopies: 2, totalReclaimableBytes: 10,
    hashedBytes: 20, filesHashed: 2, skippedLargeFiles: 0, skippedLargeBytes: 0, errors: [],
  };
  const fallback = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args?: { scanId?: string }) => {
    if (command === 'scan_start') return 'scan-verify';
    if (command === 'scan_get' && args?.scanId === 'scan-verify') return scanResult;
    if (command === 'find_duplicates') return 'report-verify';
    if (command === 'duplicates_get') return duplicateReport;
    if (command === 'verify_duplicates') return { equal: true, bytesCompared: 10, firstDifferenceOffset: null };
    return fallback(command, args);
  });

  render(<App />);
  const user = userEvent.setup();
  await user.click(screen.getByRole('button', { name: /scan a folder/i }));
  await screen.findByTestId('analyzed-size');
  await user.click(screen.getByRole('button', { name: /duplicate finder/i }));
  await user.click(screen.getByRole('button', { name: 'Find duplicates' }));
  await screen.findByText('2 copies · 10 B each');
  await user.click(screen.getByRole('button', { name: 'Verify bytes' }));

  expect(await screen.findByText('Every copy was compared byte-for-byte with the suggested original.')).toBeDefined();
  expect(invoke).toHaveBeenCalledWith('verify_duplicates', {
    pathA: 'C:\\chosen\\a.txt', pathB: 'C:\\chosen\\b.txt',
  });
});

it('disposes a native subscription that finishes registering after unmount', async () => {
  let resolveListener!: (callback: () => void) => void;
  listen.mockImplementationOnce(() => new Promise(resolve => { resolveListener = resolve; }));
  const view = render(<App />);
  await waitFor(() => expect(listen).toHaveBeenCalled());
  view.unmount();
  const unsubscribe = vi.fn();
  resolveListener(unsubscribe);
  await waitFor(() => expect(unsubscribe).toHaveBeenCalledTimes(1));
});

it('keeps scanning disabled until native initialization is complete', async () => {
  let resolveListener!: (callback: () => void) => void;
  listen.mockImplementationOnce(() => new Promise(resolve => { resolveListener = resolve; }));
  render(<App />);
  const button = screen.getByRole('button', { name: /scan a folder/i }) as HTMLButtonElement;
  await waitFor(() => expect(listen).toHaveBeenCalled());
  expect(button.disabled).toBe(true);
  resolveListener(vi.fn());
  await waitFor(() => expect(button.disabled).toBe(false));
});
