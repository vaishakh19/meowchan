/**
 * The upgrade dialog: price, purchase, activation and the honest result of each attempt.
 * The native backend is mocked; it is the only place a licence is ever verified.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AppStateProvider, useApp } from '@/app/state';
import { UpgradeDialog } from './UpgradeDialog';
import { __setTransport } from '@/lib/transport';
import { createTauriTransport } from '@/lib/backend/tauriTransport';

const { invoke, listen } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

const LOCKED = {
  enabled: false,
  state: 'not-purchased',
  configured: true,
  licenseKey: null,
  activatedAt: null,
  lastValidatedAt: null,
  offline: false,
  message: 'Diskora Pro is not active on this device.',
};

function Harness() {
  const { actions } = useApp();
  return (
    <>
      <button type="button" onClick={() => actions.promptUpgrade('Reveal in File Explorer')}>
        open-upgrade
      </button>
      <UpgradeDialog />
    </>
  );
}

function renderDialog(activate: unknown) {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'settings_get') return { theme: 'system', followLinks: false, maxChildrenPerNode: 100 };
    if (command === 'list_drives') return [];
    if (command === 'scan_list') return [];
    if (command === 'pro_status') return LOCKED;
    if (command === 'pro_activate') return activate;
    return null;
  });
  render(
    <AppStateProvider>
      <Harness />
    </AppStateProvider>,
  );
}

beforeEach(() => {
  invoke.mockReset();
  listen.mockReset();
  listen.mockResolvedValue(vi.fn());
  __setTransport(createTauriTransport());
});

afterEach(() => {
  cleanup();
  __setTransport(null);
});

describe('upgrade dialog', () => {
  it('explains the one-time $12.99 purchase for the feature that was locked', async () => {
    renderDialog(LOCKED);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('pro_status', undefined));
    await userEvent.setup().click(screen.getByRole('button', { name: 'open-upgrade' }));

    const dialog = await screen.findByRole('dialog', { name: 'Upgrade to Diskora Pro' });
    expect(dialog.textContent).toContain('$12.99');
    expect(dialog.textContent).toContain('One-time payment');
    expect(dialog.textContent).toContain('Reveal in File Explorer is a Diskora Pro feature');
    expect(screen.getByRole('button', { name: /buy pro/i })).toBeDefined();
    expect(screen.getByLabelText('Already have a license key?')).toBeDefined();
  });

  it('shows the backend reason when a key is rejected and stays open', async () => {
    renderDialog({
      ...LOCKED,
      state: 'invalid',
      message: 'This Diskora Pro licence is no longer valid. It may have been revoked, refunded, expired or used on too many devices.',
    });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('pro_status', undefined));
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'open-upgrade' }));
    await user.type(await screen.findByLabelText('Already have a license key?'), 'ABCD-1234-EFGH-5678');
    await user.click(screen.getByRole('button', { name: 'Activate' }));

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('no longer valid');
    expect(screen.getByRole('dialog', { name: 'Upgrade to Diskora Pro' })).toBeDefined();
  });

  it('closes once a licence is verified and the feature is unlocked', async () => {
    renderDialog({ ...LOCKED, enabled: true, state: 'active', licenseKey: 'ABCD…5678', message: 'Diskora Pro is active.' });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('pro_status', undefined));
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'open-upgrade' }));
    await user.type(await screen.findByLabelText('Already have a license key?'), 'ABCD-1234-EFGH-5678');
    await user.click(screen.getByRole('button', { name: 'Activate' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: /upgrade to diskora pro/i })).toBeNull());
    expect(invoke).toHaveBeenCalledWith('pro_activate', { licenseKey: 'ABCD-1234-EFGH-5678' });
  });
});
