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
beforeEach(() => {
  invoke.mockReset(); open.mockReset(); listen.mockReset();
  listen.mockResolvedValue(vi.fn());
  invoke.mockImplementation(async (command: string) => {
    if (command === 'settings_get') return { theme: 'system', followLinks: false, maxChildrenPerNode: 100 };
    if (command === 'list_drives') return [];
    if (command === 'scan_list') return [];
    return null;
  });
  __setTransport(createTauriTransport());
});
afterEach(() => { cleanup(); __setTransport(null); });
describe('desktop shell with mocked IPC (not a real filesystem test)', () => {
  it('renders the analyzer without cleanup or billing controls', async () => {
    render(<App />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('list_drives', undefined));
    expect(screen.getByRole('button', { name: /duplicate finder/i })).toBeDefined();
    expect(screen.queryByRole('button', { name: /cleanup|upgrade|buy/i })).toBeNull();
  });
  it('opens a folder picker, then sends the chosen path to Rust', async () => {
    open.mockResolvedValue('C:\\chosen');
    render(<App />);
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: /scan a folder/i }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('scan_start', { options: {
      rootPath: 'C:\\chosen', followLinks: false, maxChildrenPerNode: 100,
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
  open.mockResolvedValue('C:\\chosen');
  const root = { id: 'root', path: 'C:\\chosen', name: 'chosen', kind: 'folder',
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
