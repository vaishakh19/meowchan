/**
 * Privacy.
 *
 * This page is deliberately literal: it states exactly what is stored locally, what can leave the
 * device, and what never happens. The numbers come from the backend's privacy report, not from
 * marketing copy.
 */

import { useMemo } from 'react';
import { Check, Database, EyeOff, ShieldCheck, WifiOff } from 'lucide-react';
import { useApp } from '@/app/state';
import { Button, EmptyState, Notice, Panel, SectionTitle } from '@/components/ui/primitives';

const NEVER = [
  'No account is required and none is created.',
  'No files, file names or paths are uploaded — ever.',
  'No full-path analytics or telemetry.',
  'No ads, no sponsored results, no bundled toolbars.',
  'No hidden or background scanning: a scan only runs while you are looking at it, and it stops when you cancel.',
  'Diskora never executes a file it has analysed.',
  'Diskora never elevates privileges or asks for administrator rights to scan a folder you can already read.',
];

export function PrivacyView(): React.JSX.Element {
  const { state, actions } = useApp();
  const report = state.privacy;

  const rows = useMemo(() => {
    if (!report) return [];
    return [
      { title: 'Stored on this PC', items: report.localStorage, icon: <Database aria-hidden className="size-3.5" /> },
      {
        title: 'Leaves this device',
        items:
          report.networkActivity.length === 0
            ? ['Nothing. Crash reporting is off unless you turn it on in Settings.']
            : report.networkActivity,
        icon: <WifiOff aria-hidden className="size-3.5" />,
      },
    ];
  }, [report]);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto" data-testid="privacy-view">
      <header className="border-b border-line bg-surface px-4 py-3">
        <h1 className="text-[13px] font-semibold text-ink">Privacy</h1>
        <p className="mt-0.5 text-[10px] text-ink-faint">
          Local-first disk analysis. Below is exactly what Diskora keeps and what it sends.
        </p>
      </header>

      <div className="max-w-3xl px-4 py-3">
        {!report ? (
          <Panel>
            <EmptyState
              icon={<ShieldCheck aria-hidden className="size-6" />}
              title="The native backend is not running"
              detail="In the desktop app this page lists the real database and data directories for your installation. In a browser preview there is no filesystem access at all."
              action={
                <Button variant="primary" onClick={() => actions.navigate('home')}>
                  Back to home
                </Button>
              }
            />
          </Panel>
        ) : (
          <>
            <Panel className="mb-3 p-3">
              <SectionTitle className="px-0 pt-0">Where Diskora keeps its data</SectionTitle>
              <dl className="space-y-1 text-[11px]">
                <Row label="Application data" value={report.dataDir} mono />
                <Row label="Local database" value={report.databasePath} mono />
              </dl>
              <p className="mt-2 text-[10px] leading-relaxed text-ink-faint">
                The database holds preferences and cached scan history. Delete it and Diskora starts
                from a clean slate; scanning works exactly the same.
              </p>
            </Panel>

            <div className="mb-3 grid gap-3 sm:grid-cols-2">
              {rows.map((row) => (
                <Panel key={row.title} className="p-3">
                  <SectionTitle
                    className="px-0 pt-0"
                    action={<span className="text-ink-faint">{row.icon}</span>}
                  >
                    {row.title}
                  </SectionTitle>
                  <ul className="space-y-1">
                    {row.items.map((item) => (
                      <li key={item} className="flex items-start gap-1.5 text-[11px] leading-relaxed text-ink-muted">
                        <Check aria-hidden className="mt-0.5 size-3 shrink-0 text-ok" />
                        <span>{item}</span>
                      </li>
                    ))}
                  </ul>
                </Panel>
              ))}
            </div>
          </>
        )}

        <Panel className="mb-3 p-3">
          <SectionTitle className="px-0 pt-0">What Diskora never does</SectionTitle>
          <ul className="space-y-1.5">
            {NEVER.map((item) => (
              <li key={item} className="flex items-start gap-1.5 text-[11px] leading-relaxed text-ink-muted">
                <EyeOff aria-hidden className="mt-0.5 size-3 shrink-0 text-ink-faint" />
                <span>{item}</span>
              </li>
            ))}
          </ul>
        </Panel>

        <Notice tone="info" className="mb-3">
          Crash reporting is opt-in and off by default. When you enable it, Diskora sends a stack
          trace and the app version — no paths, no file names, no scan results.
        </Notice>

        <Panel className="p-3">
          <SectionTitle className="px-0 pt-0">Cleanup safety</SectionTitle>
          <p className="text-[11px] leading-relaxed text-ink-muted">
            Diskora deletes nothing because a file was selected or analysed. Cleanup actions only
            apply to folders you explicitly list, always show the exact paths and byte totals, always
            require a confirmation click, and prefer the Recycle Bin. Permanent deletion is a
            separate, clearly-labelled action. At least one copy of every duplicate group is always
            preserved.
          </p>
          <Button size="sm" className="mt-2" variant="secondary" onClick={() => actions.navigate('settings')}>
            Open settings
          </Button>
        </Panel>
      </div>
    </div>
  );
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex flex-wrap items-baseline gap-x-2">
      <dt className="shrink-0 text-[10px] text-ink-faint">{label}</dt>
      <dd className={`min-w-0 flex-1 break-all text-ink ${mono ? 'font-mono text-[10px]' : 'text-[11px]'}`}>{value}</dd>
    </div>
  );
}
