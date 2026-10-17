/**
 * Free vs Pro, end to end through the real state, details panel and upgrade dialog.
 *
 * The IPC is mocked with exactly what the Rust backend sends: on the free tier `path` is an opaque
 * identity token and only `displayPath` carries a masked string; with Pro both carry real paths.
 * These tests assert what reaches the DOM and which commands are invoked.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AppStateProvider, useApp } from './state';
import { DetailsPanel } from './DetailsPanel';
import { UpgradeDialog } from '@/features/pro/UpgradeDialog';
import { DeleteConfirmDialog } from './DeleteConfirmDialog';
import { ToastHost } from '@/components/ui/feedback';
import { __setTransport } from '@/lib/transport';
import { createTauriTransport } from '@/lib/backend/tauriTransport';

const { invoke, listen } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

const SCAN_ID = 'scan-free-1';
const REAL_ROOT = 'C:\\fixture';
const REAL_FILE = 'C:\\fixture\\a.bin';

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

/** The tree the backend sends to a free user: tokens for identity, masked text for display. */
function freeTree() {
  return {
    id: 'root',
    path: 'ref:1111111111111111',
    displayPath: REAL_ROOT,
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
        path: 'ref:2222222222222222',
        displayPath: 'C:\\...\\a.bin',
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
}

/** The same scan as Pro receives it: real paths, no display siblings. */
function proTree() {
  const tree = freeTree();
  tree.path = REAL_ROOT;
  delete (tree as { displayPath?: string }).displayPath;
  tree.children[0].path = REAL_FILE;
  delete (tree.children[0] as { displayPath?: string }).displayPath;
  return tree;
}

function scanResult(tree: ReturnType<typeof freeTree>, free: boolean) {
  return {
    scanId: SCAN_ID,
    rootPath: free ? tree.path : REAL_ROOT,
    rootPathDisplay: free ? REAL_ROOT : undefined,
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
}

function Harness({ free }: { free: boolean }) {
  const { actions } = useApp();
  return (
    <>
      <button type="button" onClick={() => void actions.scanPath(REAL_ROOT)}>
        start-scan
      </button>
      <button type="button" onClick={() => actions.selectNode(free ? 'ref:2222222222222222' : REAL_FILE)}>
        select-file
      </button>
      <DetailsPanel />
      <UpgradeDialog />
      <DeleteConfirmDialog />
      <ToastWire />
    </>
  );
}

function ToastWire() {
  const { state, actions } = useApp();
  return <ToastHost toasts={state.toasts} onDismiss={actions.dismissToast} />;
}

function renderFor(free: boolean) {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'settings_get') return { theme: 'system', followLinks: false, maxChildrenPerNode: 100 };
    if (command === 'list_drives') return [];
    if (command === 'scan_list') return [];
    if (command === 'pro_status') return proStatus(!free);
    if (command === 'pro_verify') return proStatus(!free);
    if (command === 'scan_start') return SCAN_ID;
    if (command === 'scan_get') return scanResult(free ? freeTree() : proTree(), free);
    if (command === 'delete_items') return { deleted: [], failed: [] };
    if (command === 'reveal_in_explorer') return null;
    return null;
  });
  return render(
    <AppStateProvider>
      <Harness free={free} />
    </AppStateProvider>,
  );
}

async function openSelectedFile(free: boolean) {
  renderFor(free);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('pro_status', undefined));
  // Wait for the licence state to land, so the first click is judged against the real tier.
  await waitFor(() => {
    const label = screen.queryByRole('button', { name: 'start-scan' });
    expect(label).not.toBeNull();
  });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('pro_status', undefined));
  await new Promise((resolve) => setTimeout(resolve, 0));
  const user = userEvent.setup();
  await user.click(screen.getByRole('button', { name: 'start-scan' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('scan_get', { scanId: SCAN_ID }));
  await user.click(screen.getByRole('button', { name: 'select-file' }));
  await waitFor(() => expect(screen.getByTestId('details-path')).toBeDefined());
  return user;
}

const writeText = vi.fn(async () => undefined);

beforeEach(() => {
  invoke.mockReset();
  listen.mockReset();
  listen.mockResolvedValue(vi.fn());
  writeText.mockClear();
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  __setTransport(createTauriTransport());
});

afterEach(() => {
  cleanup();
  __setTransport(null);
});

describe('free tier never shows or hands out a real path', () => {
  it('shows the masked path and no real path anywhere in the details panel', async () => {
    await openSelectedFile(true);
    expect(screen.getByTestId('details-path').textContent).toBe('C:\\...\\a.bin');
    expect(document.body.textContent ?? '').not.toContain('C:\\fixture\\a.bin');
    expect(document.body.textContent ?? '').not.toContain('ref:2222');
  });

  it('keeps delete, reveal and copy visible, locked and labelled Pro', async () => {
    await openSelectedFile(true);
    const deleteButton = screen.getByTestId('delete-item');
    const copyButton = screen.getByTestId('copy-path');
    expect(deleteButton.textContent).toContain('Pro');
    expect(copyButton.textContent).toContain('Pro');
    expect(copyButton.getAttribute('title')).toBe('Diskora Pro feature');
    expect(screen.getByRole('button', { name: /reveal/i }).textContent).toContain('Pro');
  });

  it('turns a locked delete into the upgrade dialog, never a delete call', async () => {
    const user = await openSelectedFile(true);
    await user.click(screen.getByTestId('delete-item'));
    expect(await screen.findByRole('dialog', { name: 'Upgrade to Diskora Pro' })).toBeDefined();
    expect(screen.getByText(/Safe file deletion is a Diskora Pro feature/)).toBeDefined();
    expect(invoke.mock.calls.some(([command]) => command === 'delete_items')).toBe(false);
  });

  it('turns a locked copy into the upgrade dialog and copies nothing', async () => {
    const user = await openSelectedFile(true);
    await user.click(screen.getByTestId('copy-path'));
    expect(await screen.findByRole('dialog', { name: 'Upgrade to Diskora Pro' })).toBeDefined();
    expect(writeText).not.toHaveBeenCalled();
  });

  it('turns a locked reveal into the upgrade dialog and invokes nothing', async () => {
    const user = await openSelectedFile(true);
    await user.click(screen.getByRole('button', { name: /reveal/i }));
    expect(await screen.findByRole('dialog', { name: 'Upgrade to Diskora Pro' })).toBeDefined();
    expect(invoke.mock.calls.some(([command]) => command === 'reveal_in_explorer')).toBe(false);
  });
});

describe('Pro unlocks the same surfaces', () => {
  it('shows the real path and opens the delete confirmation instead of the upgrade dialog', async () => {
    const user = await openSelectedFile(false);
    expect(screen.getByTestId('details-path').textContent).toBe(REAL_FILE);
    await user.click(screen.getByTestId('delete-item'));
    expect(await screen.findByRole('dialog', { name: 'Move to the Recycle Bin?' })).toBeDefined();
    expect(screen.queryByRole('dialog', { name: 'Upgrade to Diskora Pro' })).toBeNull();
  });

  it('copies the complete path on request', async () => {
    const user = await openSelectedFile(false);
    // userEvent.setup() installs its own clipboard; spy on the one the app actually calls.
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    await user.click(screen.getByTestId('copy-path'));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(REAL_FILE));
  });
});
