import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '@/app/App';
import { __setTransport } from '@/lib/transport';
import { createTauriTransport } from '@/lib/backend/tauriTransport';
import type { UserStorageReport } from '@/types/contracts';

const { invoke, open, listen } = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

const MOCK_USER_REPORT: UserStorageReport = {
  driveId: 'Local Disk (C:)',
  totalAttributedBytes: 45_000_000_000,
  users: [
    {
      username: 'Alice',
      userPath: 'C:\\Users\\Alice',
      totalBytes: 30_000_000_000,
      diskSizeBytes: 30_000_000_000,
      fileCount: 1500,
      folderCount: 120,
      categories: {
        documents: 10_000_000_000,
        media: 15_000_000_000,
        cache: 5_000_000_000,
      },
      topFolders: [
        { name: 'Documents', path: 'C:\\Users\\Alice\\Documents', sizeBytes: 10_000_000_000 },
        { name: 'Videos', path: 'C:\\Users\\Alice\\Videos', sizeBytes: 15_000_000_000 },
      ],
    },
    {
      username: 'Bob',
      userPath: 'C:\\Users\\Bob',
      totalBytes: 15_000_000_000,
      diskSizeBytes: 15_000_000_000,
      fileCount: 800,
      folderCount: 50,
      categories: {
        development: 12_000_000_000,
        downloads: 3_000_000_000,
      },
      topFolders: [
        { name: 'Projects', path: 'C:\\Users\\Bob\\Projects', sizeBytes: 12_000_000_000 },
      ],
    },
  ],
};

const MOCK_SCAN = {
  scanId: 'scan-1',
  rootPath: 'C:\\',
  rootName: 'Local Disk (C:)',
  startedAt: Date.now() - 60000,
  finishedAt: Date.now(),
  status: 'complete',
  tree: {
    id: 'root',
    path: 'C:\\',
    name: 'Local Disk (C:)',
    kind: 'folder',
    sizeBytes: 100_000_000_000,
    diskSizeBytes: 100_000_000_000,
    fileCount: 5000,
    folderCount: 400,
    modifiedAt: null,
    createdAt: null,
    extension: null,
    category: 'other',
    accessDenied: false,
    children: [],
    synthetic: false,
  },
  totalBytes: 100_000_000_000,
  diskTotalBytes: 100_000_000_000,
  fileCount: 5000,
  folderCount: 400,
  errors: [],
  truncated: false,
  prunedNodes: 0,
  durationMs: 1200,
  expectedBytes: null,
};

beforeEach(() => {
  invoke.mockReset();
  open.mockReset();
  listen.mockReset();
  listen.mockResolvedValue(vi.fn());
  invoke.mockImplementation(async (command: string) => {
    if (command === 'settings_get') return { theme: 'system', followLinks: false, maxChildrenPerNode: 100 };
    if (command === 'list_drives') {
      return [
        {
          id: 'C:',
          name: 'Local Disk (C:)',
          path: 'C:\\',
          kind: 'fixed',
          totalBytes: 512_000_000_000,
          freeBytes: 200_000_000_000,
          usedBytes: 312_000_000_000,
          fileSystem: 'NTFS',
          isSystemDrive: true,
          isReadOnly: false,
          isDisconnected: false,
        },
      ];
    }
    if (command === 'scan_list') return [MOCK_SCAN];
    if (command === 'scan_get') return MOCK_SCAN;
    if (command === 'user_storage_report') return MOCK_USER_REPORT;
    if (command === 'export_scan_csv') return '"Username","User Path"\n"Alice","C:\\Users\\Alice"\n';
    if (command === 'pro_status') {
      return {
        enabled: true,
        configured: true,
        licenseKey: 'PRO-1234',
        activatedAt: Date.now(),
        lastValidatedAt: Date.now(),
        offline: false,
        message: 'Active',
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

describe('User Storage Attribution & CSV Export', () => {
  it('displays user profile breakdown and top folders', async () => {
    render(<App />);

    // Open Analyze view with mock scan
    const fallback = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (cmd, args) => {
      if (cmd === 'scan_start') return 'scan-1';
      return fallback(cmd, args);
    });

    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: /scan drive/i }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /user profiles/i })).toBeDefined();
    });

    await user.click(screen.getByRole('button', { name: /user profiles/i }));

    expect(await screen.findByText('Per-User Storage Attribution')).toBeDefined();
    expect(screen.getByText('Alice')).toBeDefined();
    expect(screen.getByText('Bob')).toBeDefined();
    expect(screen.getByText('C:\\Users\\Alice')).toBeDefined();
  });
});
