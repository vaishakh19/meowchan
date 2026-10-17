import { describe, it, expect, vi } from 'vitest';
const { listen, unlisten, invoke } = vi.hoisted(() => ({
  listen: vi.fn(), unlisten: vi.fn(), invoke: vi.fn(),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
import { createTauriTransport } from './tauriTransport';

describe('native transport', () => {
  it('forwards event payloads and actually releases native listeners', async () => {
    listen.mockResolvedValue(unlisten);
    const handler = vi.fn();
    const unsubscribe = await createTauriTransport().onScanProgress(handler);
    const callback = listen.mock.calls[0]![1] as (event: { payload: unknown }) => void;
    callback({ payload: { phase: 'done' } });
    expect(handler).toHaveBeenCalledWith({ phase: 'done' });
    unsubscribe();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });
  it('passes scan options to the native command unchanged', async () => {
    invoke.mockResolvedValue('scan-1');
    const options = { rootPath: 'C:\\test', followLinks: false, maxChildrenPerNode: 100 };
    expect(await createTauriTransport().scanStart(options)).toBe('scan-1');
    expect(invoke).toHaveBeenCalledWith('scan_start', { options });
  });

  it('forwards duplicate cancellation to the native command', async () => {
    await createTauriTransport().cancelDuplicates('dup-1');
    expect(invoke).toHaveBeenCalledWith('duplicates_cancel', { reportId: 'dup-1' });
  });

  it('requests bounded scan-index pages with the selected sort and search', async () => {
    invoke.mockResolvedValue({ path: 'C:/data', offset: 0, limit: 500, total: 1200, items: [] });
    const query = {
      scanId: 'scan-1', path: 'C:/data', offset: 0, limit: 500,
      sort: 'name' as const, sortDir: 'asc' as const, search: 'report',
    };
    await createTauriTransport().scanChildren(query);
    expect(invoke).toHaveBeenCalledWith('scan_children', { query });
  });
});
