/**
 * Home — the landing view.
 *
 * Shows what Diskora measured about this machine: every volume, the last scan's breakdown, and
 * the biggest reclaimable categories. If there is no scan yet it explains what will happen next
 * instead of showing a fake chart.
 */

import { useMemo } from 'react';
import {
  ArrowRight,
  Clock,
  FolderTree,
  HardDrive,
  Play,
  ShieldCheck,
  Sparkles,
  Trash2,
} from 'lucide-react';
import { useApp } from '@/app/state';
import { StorageRing } from '@/components/ui/StorageRing';
import { Badge, Button, EmptyState, Panel, SectionTitle, cx } from '@/components/ui/primitives';
import { formatBytes, formatCount, formatDateTime, formatDuration, formatPercent } from '@/lib/format';
import { categoryColor, categoryLabel } from '@/lib/categories';
import { categoryBreakdown } from '@/features/visualization/layout';
import { computeQuickWins, totalQuickWinBytes } from './quickWins';
import { StorageBreakdownSection } from './StorageBreakdownView';

export function HomeView(): React.JSX.Element {
  const { state, actions } = useApp();
  const { drives, scan } = state;
  const result = scan.result;

  const breakdown = useMemo(() => (result ? categoryBreakdown(result.tree) : []), [result]);
  const quickWins = useMemo(() => (result ? computeQuickWins(result.tree) : []), [result]);
  const quickWinTotal = useMemo(() => totalQuickWinBytes(quickWins), [quickWins]);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto" data-testid="home-view">
      {/* ------------------------------- hero ------------------------------- */}
      <section className="border-b border-line bg-surface px-5 py-4">
        <h1 className="text-[17px] font-semibold tracking-tight text-ink">
          {result ? 'Your storage at a glance' : 'Understand your disk in seconds'}
        </h1>
        <p className="mt-1 max-w-2xl text-xs leading-relaxed text-ink-muted">
          {result
            ? `Last scan of ${result.rootName} finished ${formatDateTime(result.finishedAt ?? result.startedAt)} in ${formatDuration(result.durationMs ?? 0)}.`
            : 'Diskora scans a drive, maps every folder by real size, and helps you find what is using space. Everything runs locally on this PC — nothing is uploaded. Only Diskora Pro license checks ever leave the machine, and only if you choose to buy Pro.'}
        </p>
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <Button
            variant="primary"
            onClick={() => void actions.scanDrive()}
            disabled={scan.scanning || drives.length === 0}
          >
            <Play aria-hidden className="size-3.5" />
            {result ? 'Scan again' : 'Scan the selected drive'}
          </Button>
          {result ? (
            <Button onClick={() => actions.navigate('analyze')}>
              <FolderTree aria-hidden className="size-3.5" />
              Open the sunburst
              <ArrowRight aria-hidden className="size-3" />
            </Button>
          ) : null}
          <Button variant="ghost" onClick={() => actions.navigate('privacy')}>
            <ShieldCheck aria-hidden className="size-3.5" />
            What Diskora stores
          </Button>
        </div>
      </section>

      {/* ------------------------------ volumes ------------------------------ */}
      <section className="px-5 py-4">
        <SectionTitle className="px-0">Volumes</SectionTitle>
        {drives.length === 0 ? (
          <Panel className="p-4">
            <p className="text-xs text-ink-muted">Detecting drives…</p>
          </Panel>
        ) : (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(230px,1fr))] gap-2">
            {drives.map((drive) => {
              const active = drive.id === state.selectedDriveId;
              return (
                <button
                  key={drive.id}
                  type="button"
                  onClick={() => {
                    actions.selectDrive(drive.id);
                    void actions.scanDrive(drive.id);
                  }}
                  className={cx(
                    'flex items-start gap-3 rounded-panel border bg-surface p-3 text-left shadow-panel transition-colors',
                    active ? 'border-accent' : 'border-line hover:border-line-strong',
                  )}
                  data-testid="drive-card"
                >
                  <StorageRing
                    usedBytes={drive.usedBytes}
                    totalBytes={drive.totalBytes}
                    size={64}
                    thickness={6}
                    label={`${drive.name} usage`}
                  />
                  <div className="min-w-0 flex-1">
                    <p className="flex items-center gap-1.5 truncate text-[13px] font-medium text-ink">
                      <HardDrive aria-hidden className="size-3.5 shrink-0 text-ink-faint" />
                      {drive.name}
                    </p>
                    <p className="dw-tnum mt-1 text-[11px] text-ink-muted">
                      {formatBytes(drive.freeBytes)} free of {formatBytes(drive.totalBytes)}
                    </p>
                    <p className="mt-1 flex flex-wrap items-center gap-1">
                      <Badge tone="neutral">{drive.fileSystem}</Badge>
                      <Badge tone="neutral">{drive.kind}</Badge>
                      {drive.isSystemDrive ? <Badge tone="info">System</Badge> : null}
                      {drive.isDisconnected ? <Badge tone="warn">Disconnected</Badge> : null}
                    </p>
                  </div>
                </button>
              );
            })}
          </div>
        )}
      </section>

      {/* ------------------- storage breakdown ------------------- */}
      <StorageBreakdownSection />

      {/* --------------------------- last scan --------------------------- */}
      <section className="px-5 pb-4">
        <SectionTitle className="px-0">Last scan</SectionTitle>
        {!result ? (
          <Panel>
            <EmptyState
              icon={<FolderTree aria-hidden className="size-6" />}
              title="No scan results yet"
              detail="Scan a volume to see its category breakdown, the largest folders and your Quick Wins."
            />
          </Panel>
        ) : (
          <div className="grid grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-2">
            <Panel className="p-3">
              <SectionTitle className="px-0 pt-0">Where the space goes</SectionTitle>
              <ul className="flex flex-col gap-1.5">
                {breakdown.slice(0, 7).map((slice) => (
                  <li key={slice.category} className="flex items-center gap-2">
                    <span
                      aria-hidden
                      className="size-2.5 shrink-0 rounded-sm ring-1 ring-black/5"
                      style={{ background: categoryColor(slice.category) }}
                    />
                    <span className="min-w-0 flex-1 truncate text-[12px] text-ink">
                      {categoryLabel(slice.category)}
                    </span>
                    <span className="dw-tnum shrink-0 text-[11px] text-ink-muted">
                      {formatCount(slice.fileCount)} files
                    </span>
                    <span className="dw-tnum w-16 shrink-0 text-right text-[11px] font-medium text-ink">
                      {formatBytes(slice.bytes)}
                    </span>
                    <span className="dw-tnum w-10 shrink-0 text-right text-[10px] text-ink-faint">
                      {formatPercent(slice.fraction, 1)}
                    </span>
                  </li>
                ))}
              </ul>
              <div className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-surface-sunken">
                <div className="flex h-full">
                  {breakdown.map((slice) => (
                    <span
                      key={slice.category}
                      style={{ width: `${slice.fraction * 100}%`, background: categoryColor(slice.category) }}
                    />
                  ))}
                </div>
              </div>
            </Panel>

            <Panel className="p-3">
              <SectionTitle
                className="px-0 pt-0"
                action={
                  quickWinTotal > 0 ? (
                    <span className="dw-tnum text-[10px] font-medium text-ink-muted">
                      {formatBytes(quickWinTotal)} total
                    </span>
                  ) : null
                }
              >
                <span className="flex items-center gap-1">
                  <Sparkles aria-hidden className="size-3" />
                  Quick Wins
                </span>
              </SectionTitle>
              {quickWins.length === 0 ? (
                <p className="text-[11px] leading-relaxed text-ink-muted">
                  Nothing obvious to reclaim on this volume.
                </p>
              ) : (
                <ul className="flex flex-col gap-1">
                  {quickWins.slice(0, 6).map((win) => (
                    <li key={win.id}>
                      <button
                        type="button"
                        onClick={() => {
                          if (win.id === 'large-media') actions.navigate('large-files');
                          else if (win.path) {
                            actions.navigate('analyze');
                            actions.focusPath(win.path);
                          } else actions.navigate('large-files');
                        }}
                        className="flex w-full items-center gap-2 rounded-control px-1.5 py-1 text-left transition-colors hover:bg-surface-sunken"
                      >
                        <span
                          aria-hidden
                          className="size-2 shrink-0 rounded-sm ring-1 ring-black/5"
                          style={{ background: categoryColor(win.category) }}
                        />
                        <span className="min-w-0 flex-1 truncate text-[12px] text-ink">{win.label}</span>
                        <span className="dw-tnum shrink-0 text-[11px] font-medium text-ink">
                          {formatBytes(win.bytes)}
                        </span>
                        <ArrowRight aria-hidden className="size-3 shrink-0 text-ink-faint" />
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <Button
                size="sm"
                className="mt-2 w-full justify-center"
                onClick={() => actions.navigate('large-files')}
              >
                <Trash2 aria-hidden className="size-3" />
                Review large files
              </Button>
            </Panel>

            <Panel className="p-3">
              <SectionTitle className="px-0 pt-0">Scan facts</SectionTitle>
              <dl className="dw-tnum space-y-1 text-[11px]">
                <Fact label="Root" value={result.rootName} />
                <Fact label="Analyzed" value={formatBytes(result.totalBytes)} />
                <Fact label="Files" value={formatCount(result.fileCount)} />
                <Fact label="Folders" value={formatCount(result.folderCount)} />
                <Fact label="Duration" value={formatDuration(result.durationMs ?? 0)} />
                <Fact label="Unreadable locations" value={formatCount(result.errors.length)} />
                <Fact label="Folded items" value={formatCount(result.prunedNodes)} />
              </dl>
              {result.errors.length > 0 ? (
                <div className="mt-2 max-h-24 overflow-y-auto rounded-control bg-surface-sunken p-2">
                  <p className="mb-1 text-[10px] font-medium text-ink-muted">Diskora could not read:</p>
                  <ul className="space-y-0.5">
                    {result.errors.slice(0, 8).map((error) => (
                      <li key={error.path} className="truncate font-mono text-[9px] text-ink-faint" title={error.path}>
                        {error.path} — {error.kind}
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
            </Panel>
          </div>
        )}
      </section>

      {/* ------------------------------ privacy ------------------------------ */}
      <section className="px-5 pb-6">
        <Panel className="flex items-start gap-2.5 p-3">
          <ShieldCheck aria-hidden className="mt-px size-4 shrink-0 text-ok" />
          <div>
            <p className="text-[12px] font-medium text-ink">Local-first by default</p>
            <p className="mt-0.5 text-[11px] leading-relaxed text-ink-muted">
              No account, no cloud upload, no background scanning. Diskora reads your filesystem,
              keeps results in memory, and only writes preferences and cached scan history to a
              local SQLite database.{' '}
              <button
                type="button"
                className="underline hover:text-ink"
                onClick={() => actions.navigate('privacy')}
              >
                Read the privacy details
              </button>
              .
            </p>
          </div>
        </Panel>
        <p className="mt-2 flex items-center gap-1 text-[10px] text-ink-faint">
          <Clock aria-hidden className="size-3" />
          Press F5 anywhere to rescan the selected drive.
        </p>
      </section>
    </div>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <dt className="text-ink-faint">{label}</dt>
      <dd className="min-w-0 truncate text-ink" title={value}>
        {value}
      </dd>
    </div>
  );
}
