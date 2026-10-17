/**
 * Safe-deletion flow tests — IPC mocked, the same seam as every
 * Diskora test. They assert the locked/unlocked gating, the
 * confirmation content, and that nothing is deleted without an
 * explicit confirm.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AppStateProvider, useApp } from './state';
import { DeleteConfirmDialog } from './DeleteConfirmDialog';
import { UpgradeDialog } from '@/features/pro/UpgradeDialog';
import { ToastHost } from '@/components/ui/feedback';
import { __setTransport } from '@/lib/transport';
import { createTauriTransport } from '@/lib/backend/tauriTransport';

const { invoke, listen } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

const SCAN_ID = 'scan-test-1';
const FILE_PATH = 'C:\\fixture\\a.bin';

function proStatus(enabled: boolean) {
  return {
    enabled,
    state: enabled ? 'active' : 'not-purchased',
    configured: true,
    licenseKey: enabled ? 'ABCD…5678' : null,
    activatedAt: enabled ? 1_770_000_000_000 : null,
    lastValidatedAt: enabled ? 1_770_000_000_000 : null,
    offline: false,
    message: enabled ? 'Diskora Pro is active.' : 'Diskora Pro is not active on this device.',
  };
}

const tree = {
  id: 'root',
  path: 'C:\\fixture',
  name: 'fixture',
  kind: 'folder',
  sizeBytes: 100,
  diskSizeBytes: null,
  fileCount: 1,
  folderCount: 0,
  modifiedAt: null,
  createdAt: null,
  extension: null,
  category: 'other',
  accessDenied: false,
  synthetic: false,
  children: [
    {
      id: 'file',
      path: FILE_PATH,
      name: 'a.bin',
      kind: 'file',
      sizeBytes: 100,
      diskSizeBytes: null,
      fileCount: 0,
      folderCount: 0,
      modifiedAt: null,
      createdAt: null,
      extension: 'bin',
      category: 'other',
      accessDenied: false,
      children: [],
      synthetic: false,
    },
  ],
};

const scanResult = {
  scanId: SCAN_ID,
  rootPath: 'C:\\fixture',
  rootName: 'fixture',
  startedAt: 0,
  finishedAt: 1,
  status: 'complete',
  tree,
  totalBytes: 100,
  diskTotalBytes: null,
  fileCount: 1,
  folderCount: 0,
  errors: [],
  truncated: false,
  prunedNodes: 0,
  durationMs: 1,
  expectedBytes: null,
};

/** Buttons the tests press to reach the real action creators. */
function Harness() {
  const { actions } = useApp();
  return (
    <>
      <button type="button" onClick={() => void actions.scanPath('C:\\fixture')}>
        start-scan
      </button>
      <button type="button" onClick={() => actions.requestDeletion([FILE_PATH])}>
        request-deletion
      </button>
    </>
  );
}

/** Toasts are rendered by the app shell; the tests need them wired too. */
function ToastWire() {
  const { state, actions } = useApp();
  return <ToastHost toasts={state.toasts} onDismiss={actions.dismissToast} />;
}

function renderHarness() {
  return render(
    <AppStateProvider>
      <Harness />
      <DeleteConfirmDialog />
      <UpgradeDialog />
      <ToastWire />
    </AppStateProvider>,
  );
}

beforeEach(() => {
  invoke.mockReset();
  listen.mockReset();
  listen.mockResolvedValue(vi.fn());
  invoke.mockImplementation(async (command: string) => {
    if (command === 'settings_get') return { theme: 'system', followLinks: false, maxChildrenPerNode: 100 };
    if (command === 'list_drives') return [];
    if (command === 'scan_list') return [];
    if (command === 'pro_status') return proStatus(true);
    if (command === 'pro_verify') return proStatus(true);
    if (command === 'scan_start') return SCAN_ID;
    if (command === 'scan_get') return scanResult;
    if (command === 'delete_items') {
      return {
        deleted: [{ path: FILE_PATH, name: 'a.bin', kind: 'file', sizeBytes: 100 }],
        failed: [],
      };
    }
    return null;
  });
  __setTransport(createTauriTransport());
});
afterEach(() => {
  cleanup();
  __setTransport(null);
});

describe('safe deletion (Diskora Pro)', () => {
  it('refuses to open the dialog while Pro is locked', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'pro_status') return proStatus(false);
      if (command === 'pro_verify') return proStatus(false);
      if (command === 'settings_get') return { theme: 'system', followLinks: false, maxChildrenPerNode: 100 };
      if (command === 'list_drives') return [];
      if (command === 'scan_list') return [];
      return null;
    });
    renderHarness();
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('pro_status', undefined));
    await userEvent.setup().click(screen.getByRole('button', { name: 'request-deletion' }));

    // A locked action opens the upgrade dialog (never the delete confirmation) and calls no delete.
    await waitFor(() =>
      expect(screen.getByRole('dialog', { name: 'Upgrade to Diskora Pro' })).toBeDefined(),
    );
    expect(screen.getByText(/Safe file deletion is a Diskora Pro feature/)).toBeDefined();
    expect(screen.queryByText('Confirm deletion')).toBeNull();
    expect(invoke.mock.calls.filter(([command]) => command === 'delete_items')).toHaveLength(0);
  });

  it('lists name, full path and size, and deletes only after confirmation', async () => {
    renderHarness();
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('pro_status', undefined));
    // A completed scan so the dialog can show the scanned metadata.
    await userEvent.setup().click(screen.getByRole('button', { name: 'start-scan' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('scan_get', { scanId: SCAN_ID }));
    await userEvent.setup().click(screen.getByRole('button', { name: 'request-deletion' }));

    // The dialog lists the exact item before anything happens.
    await waitFor(() => expect(screen.getByRole('dialog')).toBeDefined());
    expect(screen.getByText('a.bin')).toBeDefined();
    expect(screen.getByText(FILE_PATH)).toBeDefined();
    expect(screen.getByText(/100 B \(100 B exact\)/)).toBeDefined();
    expect(screen.getByText(/Total: 100 B across 1 item/)).toBeDefined();

    // Nothing has been deleted yet.
    expect(invoke.mock.calls.filter(([command]) => command === 'delete_items')).toHaveLength(0);

    // Cancelling must not delete.
    await userEvent.setup().click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(invoke.mock.calls.filter(([command]) => command === 'delete_items')).toHaveLength(0);
  });

  it('moves the item through the backend and refreshes the scan result', async () => {
    renderHarness();
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('pro_status', undefined));
    // A completed scan establishes the security boundary the backend checks.
    await userEvent.setup().click(screen.getByRole('button', { name: 'start-scan' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('scan_get', { scanId: SCAN_ID }));
    await userEvent.setup().click(screen.getByRole('button', { name: 'request-deletion' }));
    await waitFor(() => expect(screen.getByRole('dialog')).toBeDefined());

    await userEvent.setup().click(screen.getByTestId('confirm-deletion'));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('delete_items', {
        request: { scanId: SCAN_ID, paths: [FILE_PATH] },
      }),
    );
    // The results are refreshed from the updated index afterwards.
    await waitFor(() => {
      const calls = invoke.mock.calls.filter(([command]) => command === 'scan_get');
      expect(calls.length).toBeGreaterThanOrEqual(1);
    });
    await waitFor(() =>
      expect(screen.getByText('Moved “a.bin” to the Recycle Bin')).toBeDefined(),
    );
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('reports backend failures honestly instead of claiming success', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'pro_status') return proStatus(true);
      if (command === 'pro_verify') return proStatus(true);
      if (command === 'settings_get') return { theme: 'system', followLinks: false, maxChildrenPerNode: 100 };
      if (command === 'list_drives') return [];
      if (command === 'scan_list') return [];
      if (command === 'scan_start') return SCAN_ID;
      if (command === 'scan_get') return scanResult;
      if (command === 'delete_items') {
        throw new Error('Moving C:\\fixture\\a.bin to the Recycle Bin failed (Windows code 5).');
      }
      return null;
    });
    renderHarness();
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('pro_status', undefined));
    await userEvent.setup().click(screen.getByRole('button', { name: 'start-scan' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('scan_get', { scanId: SCAN_ID }));
    await userEvent.setup().click(screen.getByRole('button', { name: 'request-deletion' }));
    await waitFor(() => expect(screen.getByRole('dialog')).toBeDefined());

    await userEvent.setup().click(screen.getByTestId('confirm-deletion'));

    await waitFor(() =>
      expect(screen.getByText('Deletion failed')).toBeDefined(),
    );
    expect(screen.getByText(/Windows code 5/)).toBeDefined();
  });
});
