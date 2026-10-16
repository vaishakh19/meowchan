/**
 * Application shell.
 *
 * Three regions, as specified:
 *   A. Left sidebar   (~240px) — navigation, drive summary, live scan progress, Quick Wins
 *   B. Main area      (flex)   — header, breadcrumb, controls and the active visualization
 *   C. Details panel  (~280px) — the selected item
 *
 * The window is resizable and all three regions scroll independently, which is what keeps the
 * layout usable from 960px wide upwards and at 100/125/150% display scaling.
 */

import { useEffect } from 'react';
import { AppStateProvider, useApp } from './state';
import { TitleBar } from './TitleBar';
import { Sidebar } from './Sidebar';
import { DetailsPanel } from './DetailsPanel';
import { DeleteConfirmDialog } from './DeleteConfirmDialog';
import { ToastHost } from '@/components/ui/feedback';
import { ItemMenuProvider } from './itemMenu';
import { AnalyzeView } from '@/features/scanner/AnalyzeView';
import { LargeFilesView } from '@/features/scanner/LargeFilesView';
import { DuplicatesView } from '@/features/duplicates/DuplicatesView';
import { HistoryView } from '@/features/scanner/HistoryView';
import { ProView } from '@/features/pro/ProView';
import { SettingsView } from '@/features/settings/SettingsView';
import { PrivacyView } from '@/features/settings/PrivacyView';
import { HomeView } from '@/features/scanner/HomeView';

export function App(): React.JSX.Element {
  return (
    <AppStateProvider>
      <ItemMenuProvider>
        <Shell />
      </ItemMenuProvider>
    </AppStateProvider>
  );
}

function Shell(): React.JSX.Element {
  const { state, actions } = useApp();

  // Global shortcuts. Deliberately minimal and always visible in the UI too.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
      if (event.key === 'F5') {
        event.preventDefault();
        void actions.scanDrive();
      }
      if (event.key === 'Escape') actions.selectNode(null);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [actions]);

  return (
    <div className="flex h-full flex-col overflow-hidden bg-canvas text-ink" data-testid="app-shell">
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <main className="flex min-w-0 flex-1 flex-col bg-canvas" aria-label="Workspace">
          {state.drivesError ? <div role="alert" className="border-b border-line p-3 text-danger">{state.drivesError}</div> : null}
          {state.route === 'home' ? <HomeView /> : null}
          {state.route === 'analyze' ? <AnalyzeView /> : null}
          {state.route === 'large-files' ? <LargeFilesView /> : null}
          {state.route === 'duplicates' ? <DuplicatesView /> : null}
          {state.route === 'history' ? <HistoryView /> : null}
          {state.route === 'pro' ? <ProView /> : null}
          {state.route === 'settings' ? <SettingsView /> : null}
          {state.route === 'privacy' ? <PrivacyView /> : null}
        </main>
        <DetailsPanel />
      </div>
      <DeleteConfirmDialog />
      <ToastHost toasts={state.toasts} onDismiss={actions.dismissToast} />
    </div>
  );
}
