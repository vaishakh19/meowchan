import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '@/app/App';
import { __setTransport } from '@/lib/transport';
import { createTauriTransport } from '@/lib/backend/tauriTransport';
import type { ScanGrowthComparison, ScanSummary } from '@/types/contracts';

const { invoke, open, listen } = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

const MOCK_SCANS: ScanSummary[] = [
  {
    scanId: 'scan-2',
    rootPath: 'C:\\',
    rootName: 'Local Disk (C:)',
    startedAt: 1729160000000,
    finishedAt: 1729160100000,
    durationMs: 100000,
    status: 'complete',
    totalBytes: 120_000_000_000,
    fileCount: 6000,
    folderCount: 500,
    errorCount: 0,
    truncated: false,
    driveId: 'C:',
    persisted: true,
  },
  {
    scanId: 'scan-1',
    rootPath: 'C:\\',
    rootName: 'Local Disk (C:)',
    startedAt: 1729150000000,
    finishedAt: 1729150100000,
    durationMs: 100000,
    status: 'complete',
    totalBytes: 100_000_000_000,
    fileCount: 5000,
    folderCount: 450,
    errorCount: 0,
    truncated: false,
    driveId: 'C:',
    persisted: true,
  },
];

const MOCK_GROWTH: ScanGrowthComparison = {
  baselineScanId: 'scan-1',
  currentScanId: 'scan-2',
  baselineStartedAt: 1729150000000,
  currentStartedAt: 1729160000000,
  baselineRoot: 'C:\\',
  currentRoot: 'C:\\',
  bytesDelta: 20_000_000_000,
  filesDelta: 1000,
  foldersDelta: 50,
  categoryDeltas: {
    media: 15_000_000_000,
    documents: 5_000_000_000,
  },
  growingFolders: [
    {
      path: 'C:\\Users\\Alice\\Videos',
      name: 'Videos',
      baselineBytes: 10_000_000_000,
      currentBytes: 25_000_000_000,
      deltaBytes: 15_000_000_000,
    },
  ],
  reclaimedFolders: [],
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
    if (command === 'scan_list') return MOCK_SCANS;
    if (command === 'compare_scans') return MOCK_GROWTH;
    if (command === 'pro_status') {
      return {
        enabled: false,
        configured: true,
        licenseKey: null,
        activatedAt: null,
        lastValidatedAt: null,
        offline: false,
        message: 'Free edition',
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

describe('Storage Growth Comparison', () => {
  it('navigates to History and displays Compare Scans view', async () => {
    render(<App />);

    const user = userEvent.setup();
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /scan history/i })).toBeDefined();
    });

    await user.click(screen.getByRole('button', { name: /scan history/i }));

    expect(await screen.findByText('Compare Scans')).toBeDefined();
    await user.click(screen.getByRole('button', { name: /compare scans/i }));

    expect(await screen.findByText('Storage Growth & Scan Comparison')).toBeDefined();
    expect(await screen.findByText('Net Storage Delta')).toBeDefined();
    expect(screen.getByText('Top Growing Folders')).toBeDefined();
    expect(screen.getByText('Videos')).toBeDefined();
  });
});
