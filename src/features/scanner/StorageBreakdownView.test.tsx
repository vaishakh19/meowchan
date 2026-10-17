import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '@/app/App';
import { __setTransport } from '@/lib/transport';
import { createTauriTransport } from '@/lib/backend/tauriTransport';
import type { StorageBreakdownReport } from '@/types/contracts';

const { invoke, open, listen } = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

const MOCK_BREAKDOWN: StorageBreakdownReport = {
  driveId: 'C:',
  drivePath: 'C:\\',
  measuredAt: 1729150000000,
  totalDriveBytes: 512_000_000_000,
  freeDriveBytes: 200_000_000_000,
  items: [
    {
      id: 'recycle-bin',
      name: 'Recycle Bin',
      category: 'other',
      sizeBytes: 15_000_000_000,
      itemCount: 42,
      status: 'measured',
      path: 'C:\\$Recycle.Bin',
      description: 'User files and folders moved to the Windows Recycle Bin.',
      actionableExplanation: 'Can be emptied from Desktop Recycle Bin.',
      isSystemProtected: false,
    },
    {
      id: 'system-restore-vss',
      name: 'System Restore & Shadow Copies',
      category: 'system',
      sizeBytes: null,
      itemCount: null,
      status: 'inaccessible',
      path: 'C:\\System Volume Information',
      description: 'System Restore points and Volume Shadow Copies.',
      actionableExplanation: 'Requires elevated Administrator/SYSTEM permissions.',
      isSystemProtected: true,
    },
    {
      id: 'hibernation-file',
      name: 'Hibernation File (hiberfil.sys)',
      category: 'system',
      sizeBytes: 12_884_901_888,
      itemCount: 1,
      status: 'measured',
      path: 'C:\\hiberfil.sys',
      description: 'Reserved memory dump for Fast Startup.',
      actionableExplanation: 'Configured via powercfg.exe.',
      isSystemProtected: true,
    },
    {
      id: 'virtual-memory',
      name: 'Virtual Memory (pagefile.sys)',
      category: 'system',
      sizeBytes: 8_589_934_592,
      itemCount: 1,
      status: 'measured',
      path: 'C:\\pagefile.sys',
      description: 'Windows virtual memory paging file.',
      actionableExplanation: 'Managed by Windows memory manager.',
      isSystemProtected: true,
    },
  ],
  notes: ['Read-only inspection.'],
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
    if (command === 'storage_breakdown') return MOCK_BREAKDOWN;
    if (command === 'scan_list') return [];
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

describe('Windows Storage Breakdown UI', () => {
  it('renders measured Recycle Bin and distinguishes inaccessible system shadow copies', async () => {
    render(<App />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('storage_breakdown', { driveId: 'C:' });
    });

    expect(await screen.findByText('Windows Storage & System Allocation')).toBeDefined();
    expect(screen.getByText('Recycle Bin')).toBeDefined();
    expect(screen.getByText('System Restore & Shadow Copies')).toBeDefined();
    expect(screen.getByText('Hibernation File (hiberfil.sys)')).toBeDefined();

    // Recycle bin measured size
    expect(screen.getByTestId('breakdown-card-recycle-bin')).toBeDefined();
    expect(screen.getByText('42 items')).toBeDefined();

    // Inaccessible status for System Volume Information
    expect(screen.getByText('Inaccessible (Admin Required)')).toBeDefined();
    expect(screen.getByText('Inaccessible without admin')).toBeDefined();

    // System protected badges
    expect(screen.getAllByText('System Protected').length).toBeGreaterThan(0);
  });

  it('allows manual refresh of storage breakdown', async () => {
    render(<App />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /refresh breakdown/i })).toBeDefined();
    });

    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: /refresh breakdown/i }));

    await waitFor(() => {
      expect(invoke.mock.calls.filter(([cmd]) => cmd === 'storage_breakdown').length).toBeGreaterThanOrEqual(2);
    });
  });
});
