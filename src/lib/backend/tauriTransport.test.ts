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
});
