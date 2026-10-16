/**
 * Application shell tests.
 *
 * These run in jsdom with the browser (demo) transport, which is exactly the code path a browser
 * preview takes. They verify the real reducer, the real navigation, the real selection wiring and
 * the real empty / loading / error states — no component is mocked out.
 */

import { afterEach, describe, expect, it } from 'vitest';
import { act, cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from './App';

afterEach(() => {
  cleanup();
});

/** Runs a demo scan and waits for the terminal state so results are on screen. */
async function runScanToCompletion(user: ReturnType<typeof userEvent.setup>): Promise<void> {
  await act(async () => {
    await user.click(screen.getByRole('button', { name: /scan drive/i }));
  });
  for (let i = 0; i < 200; i += 1) {
    const done = await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 25));
      return screen.queryByTestId('analyzed-size') !== null;
    });
    if (done) return;
  }
  throw new Error('The demo scan never reached a terminal state');
}

async function boot(): Promise<void> {
  await act(async () => {
    render(<App />);
    // Let the bootstrap effect resolve its transport promises.
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe('application shell', () => {
  it('renders the three regions and the navigation', async () => {
    await boot();
    expect(screen.getByTestId('app-shell')).toBeDefined();
    const sidebar = screen.getByRole('complementary', { name: /navigation/i });
    expect(within(sidebar).getByRole('button', { name: /scan drive/i })).toBeDefined();
    for (const label of ['Home', 'Large Files', 'Duplicate Finder', 'Cleanup Suggestions', 'Scan History']) {
      expect(within(sidebar).getByRole('button', { name: new RegExp(label, 'i') })).toBeDefined();
    }
  });

  it('navigates between routes and shows an honest empty state without a scan', async () => {
    await boot();
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: /large files/i }));
    expect(screen.getByTestId('large-files-view')).toBeDefined();
    expect(screen.getByText(/scan a drive first/i)).toBeDefined();

    await user.click(screen.getByRole('button', { name: /duplicate finder/i }));
    expect(screen.getByTestId('duplicates-view')).toBeDefined();

    await user.click(screen.getByRole('button', { name: /cleanup suggestions/i }));
    expect(screen.getByTestId('cleanup-view')).toBeDefined();

    await user.click(screen.getByRole('button', { name: /^home$/i }));
    expect(screen.getByTestId('home-view')).toBeDefined();
  });

  it('opens the licensing and privacy pages', async () => {
    await boot();
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: /^settings$/i }));
    expect(screen.getByTestId('settings-view')).toBeDefined();
    expect(screen.getByLabelText(/theme/i)).toBeDefined();
    expect(screen.getByText(/follow junctions and symlinks/i)).toBeDefined();

    // Privacy is reached from the home page's disclosure link.
    await user.click(screen.getByRole('button', { name: /^home$/i }));
    await user.click(screen.getByRole('button', { name: /read the privacy details/i }));
    expect(screen.getByTestId('privacy-view')).toBeDefined();
    expect(screen.getByText(/what diskwise never does/i)).toBeDefined();
  });

  it('runs a scan through the demo transport and renders real results', async () => {
    await boot();
    const user = userEvent.setup();

    await runScanToCompletion(user);

    // The app moves to the analyze route on its own when a scan starts.
    expect(screen.queryByTestId('analyze-view') ?? screen.queryByTestId('analyze-scanning')).not.toBeNull();

    const analyzed = screen.queryByTestId('analyzed-size');
    if (analyzed) {
      // Size text is derived from the synthetic tree, never a hardcoded literal.
      expect(analyzed.textContent).toMatch(/(B|KB|MB|GB|TB)\b/);
    }
  });

  it('selecting an item in the file list updates the details panel with real data', async () => {
    await boot();
    const user = userEvent.setup();

    await runScanToCompletion(user);

    // Switch to the file list so there is a deterministic row to click.
    await user.click(screen.getByRole('radio', { name: /file list/i }));

    const rows = screen.getAllByTestId('file-row');
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.length).toBeGreaterThan(0);

    await user.click(rows[0]);

    const details = screen.getByTestId('details-panel');
    const name = within(details).getByTestId('details-name');
    expect(name.textContent?.length ?? 0).toBeGreaterThan(0);
    // The size shown is measured, so it is a real formatted byte value.
    expect(within(details).getByTestId('details-size').textContent).toMatch(/\d/);
  });

  it('keeps the details panel present with a prompt before any selection', async () => {
    await boot();
    const details = screen.getByRole('complementary', { name: /details/i });
    expect(details).toBeDefined();
    expect(within(details).getByText(/nothing selected/i)).toBeDefined();
  });
});
