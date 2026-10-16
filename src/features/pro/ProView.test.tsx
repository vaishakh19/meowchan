/**
 * Pro pricing/licence screen tests — with the IPC layer mocked, the
 * same seam every Diskora test uses. These assert the locked,
 * purchased, unconfigured and failing states, never a real payment.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AppStateProvider } from '@/app/state';
import { ProView } from './ProView';
import { __setTransport } from '@/lib/transport';
import { createTauriTransport } from '@/lib/backend/tauriTransport';

const { invoke, listen } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

function proStatus(overrides: Record<string, unknown> = {}) {
  return {
    enabled: false,
    configured: true,
    licenseKey: null,
    activatedAt: null,
    lastValidatedAt: null,
    offline: false,
    message: 'Diskora Pro is not active on this device.',
    ...overrides,
  };
}

function renderPro() {
  return render(
    <AppStateProvider>
      <ProView />
    </AppStateProvider>,
  );
}

beforeEach(() => {
  invoke.mockReset();
  listen.mockReset();
  listen.mockResolvedValue(vi.fn());
  invoke.mockImplementation(async (command: string) => {
    if (command === 'pro_status') return proStatus();
    if (command === 'pro_verify') return proStatus();
    return null;
  });
  __setTransport(createTauriTransport());
});
afterEach(() => {
  cleanup();
  __setTransport(null);
});

describe('Diskora Pro pricing screen', () => {
  it('shows the one-time price and both Pro features while locked', async () => {
    renderPro();
    await waitFor(() => expect(screen.getByTestId('pro-pricing')).toBeDefined());

    expect(screen.getByText('$12.99')).toBeDefined();
    expect(screen.getByText('Pay once, use permanently.')).toBeDefined();
    expect(screen.getByText('One-time purchase · No subscription · No account · No renewals')).toBeDefined();

    expect(screen.getByText('Safe file deletion')).toBeDefined();
    expect(screen.getByText('Full file paths')).toBeDefined();
    // Free features are explicitly stated to stay free.
    expect(screen.getByText(/Scanning, filtering, size calculations and duplicate detection stay free/)).toBeDefined();

    // Locked state: an upgrade path, no success state.
    expect(screen.getByTestId('upgrade-to-pro')).toBeDefined();
    expect(screen.queryByTestId('pro-active')).toBeNull();
    expect(screen.getByTestId('license-key')).toBeDefined();
    expect(screen.getByTestId('restore-purchases')).toBeDefined();
  });

  it('shows the purchased state with the masked key after verification', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'pro_status' || command === 'pro_verify') {
        return proStatus({
          enabled: true,
          licenseKey: 'ABCD…5678',
          activatedAt: 1_770_000_000_000,
          lastValidatedAt: 1_770_000_000_000,
          message: 'Diskora Pro is active. One-time purchase; no subscription.',
        });
      }
      return null;
    });
    renderPro();
    await waitFor(() => expect(screen.getByTestId('pro-active')).toBeDefined());
    expect(screen.getByText('Active — both Pro features are unlocked')).toBeDefined();
    expect(screen.getByText('ABCD…5678')).toBeDefined();
    // No checkout button once purchased.
    expect(screen.queryByTestId('upgrade-to-pro')).toBeNull();
  });

  it('names exactly what is missing when the build is unconfigured', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'pro_status' || command === 'pro_verify') {
        return proStatus({ configured: false });
      }
      return null;
    });
    renderPro();
    await waitFor(() =>
      expect(screen.getByText(/This build has no Dodo Payments product or payment link/)).toBeDefined(),
    );
    expect(screen.getByText(/DISKORA_DODO_PRODUCT_ID/)).toBeDefined();
    // Nothing can be purchased from an unconfigured build.
    expect(screen.queryByTestId('upgrade-to-pro')).toBeNull();
  });

  it('sends a pasted key to the backend for activation', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'pro_activate') return proStatus({ enabled: true });
      if (command === 'pro_status' || command === 'pro_verify') return proStatus();
      return null;
    });
    renderPro();
    await waitFor(() => expect(screen.getByTestId('license-key')).toBeDefined());

    const user = userEvent.setup();
    await user.type(screen.getByTestId('license-key'), '  ABCD-1234-EFGH-5678  ');
    await user.click(screen.getByTestId('activate-license'));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('pro_activate', { licenseKey: 'ABCD-1234-EFGH-5678' }),
    );
  });

  it('restores purchases by re-validating with the provider', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'pro_restore') return proStatus({ enabled: true });
      if (command === 'pro_status' || command === 'pro_verify') return proStatus();
      return null;
    });
    renderPro();
    await waitFor(() => expect(screen.getByTestId('restore-purchases')).toBeDefined());
    await userEvent.setup().click(screen.getAllByTestId('restore-purchases')[0]);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('pro_restore', undefined));
  });

  it('surfaces activation failures instead of unlocking', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'pro_activate') {
        throw new Error('Dodo Payments does not know this license key.');
      }
      if (command === 'pro_status' || command === 'pro_verify') return proStatus();
      return null;
    });
    renderPro();
    await waitFor(() => expect(screen.getByTestId('license-key')).toBeDefined());

    await userEvent.setup().type(screen.getByTestId('license-key'), 'NOPE-1234');
    await userEvent.setup().click(screen.getByTestId('activate-license'));

    await waitFor(() =>
      expect(screen.getByRole('alert').textContent).toMatch(/does not know this license key/),
    );
    // Still locked: nothing was unlocked by the failed attempt.
    expect(screen.queryByTestId('pro-active')).toBeNull();
  });
});
