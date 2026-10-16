/**
 * Left navigation sidebar (~240px).
 *
 * Sections, top to bottom:
 *   1. Identity + primary actions (Scan Drive / Scan a folder…)
 *   2. Navigation
 *   3. Selected drive: capacity, used, free, storage ring
 *   4. Live scan progress with cancellation
 *   5. Quick Wins — real measurements taken from the current scan
 */

import { useMemo } from 'react';
import {
  Clock,
  Copy,
  Crown,
  FileSearch,
  FolderSearch,
  FolderTree,
  HardDrive,
  History,
  Home,
  Play,
  Settings,
  Sparkles,
  Square,
  X,
} from 'lucide-react';
import { Button, Divider, Panel, ProBadge, ProgressBar, SectionTitle, cx } from '@/components/ui/primitives';
import { StorageRing } from '@/components/ui/StorageRing';
import { formatBytes, formatCount, formatDuration, formatPercent } from '@/lib/format';
import { categoryColor } from '@/lib/categories';
import { computeQuickWins, totalQuickWinBytes } from '@/features/scanner/quickWins';
import { useApp, type Route } from './state';

const NAV: Array<{ route: Route; label: string; icon: typeof Home; pro?: boolean }> = [
  { route: 'home', label: 'Home', icon: Home },
  { route: 'analyze', label: 'Drives and folders', icon: FolderTree },
  { route: 'large-files', label: 'Large Files', icon: FileSearch },
  { route: 'duplicates', label: 'Duplicate Finder', icon: Copy },
  { route: 'history', label: 'Scan History', icon: History },
  { route: 'pro', label: 'Diskora Pro', icon: Crown, pro: true },
  { route: 'settings', label: 'Settings', icon: Settings },
];

export function Sidebar(): React.JSX.Element {
  const { state, actions } = useApp();
  const { drives, selectedDriveId, scan, viz } = state;

  const drive = drives.find((d) => d.id === selectedDriveId) ?? drives[0] ?? null;
  const scanning = scan.scanning;
  const progress = scan.progress;

  const quickWins = useMemo(
    () => (scan.result ? computeQuickWins(scan.result.tree) : []),
    [scan.result],
  );
  const quickWinTotal = useMemo(() => totalQuickWinBytes(quickWins), [quickWins]);

  return (
    <aside
      className="flex w-[240px] shrink-0 flex-col gap-2 overflow-y-auto border-r border-line bg-canvas p-2"
      aria-label="Primary navigation"
    >
      {/* ---------------- primary actions ---------------- */}
      <div className="flex flex-col gap-1.5">
        <Button
          variant="primary"
          className="w-full justify-start"
          onClick={() => void actions.scanDrive()}
          disabled={!state.ready || scanning || drives.length === 0}
          data-testid="scan-drive"
        >
          {scanning ? <Square aria-hidden className="size-3.5" /> : <Play aria-hidden className="size-3.5" />}
          {scanning ? 'Scanning…' : 'Scan Drive'}
        </Button>
        <Button
          variant="secondary"
          className="w-full justify-start"
          onClick={() => void actions.pickAndScanFolder()}
          disabled={!state.ready || scanning}
        >
          <FolderSearch aria-hidden className="size-3.5" />
          Scan a folder…
        </Button>
      </div>

      {/* ---------------- navigation ---------------- */}
      <nav className="flex flex-col gap-0.5" aria-label="Sections">
        {NAV.map((item) => {
          const active = state.route === item.route;
          const Icon = item.icon;
          return (
            <button
              key={item.route}
              type="button"
              onClick={() => actions.navigate(item.route)}
              aria-current={active ? 'page' : undefined}
              className={cx(
                'flex items-center gap-2 rounded-control px-2 py-1.5 text-left text-[13px] transition-colors',
                active
                  ? 'bg-surface font-medium text-ink shadow-panel'
                  : 'text-ink-muted hover:bg-surface hover:text-ink',
              )}
            >
              <Icon aria-hidden className="size-3.5 shrink-0" />
              <span className="flex-1 truncate">{item.label}</span>
              {item.pro && !state.pro?.enabled ? <ProBadge /> : null}
            </button>
          );
        })}
      </nav>

      <Divider />

      {/* ---------------- drive summary ---------------- */}
      {drive ? (
        <Panel className="p-2.5">
          <div className="flex items-start gap-2.5">
            <StorageRing
              usedBytes={drive.usedBytes}
              totalBytes={drive.totalBytes}
              size={72}
              thickness={7}
              label={`${drive.name} usage`}
              sublabel={drive.fileSystem}
            />
            <div className="min-w-0 flex-1">
              <p className="flex items-center gap-1 truncate text-[13px] font-medium text-ink">
                <HardDrive aria-hidden className="size-3.5 shrink-0 text-ink-faint" />
                {drive.name}
              </p>
              <dl className="dw-tnum mt-1 space-y-0.5 text-[11px]">
                <div className="flex justify-between gap-2">
                  <dt className="text-ink-faint">Capacity</dt>
                  <dd className="text-ink">{formatBytes(drive.totalBytes)}</dd>
                </div>
                <div className="flex justify-between gap-2">
                  <dt className="text-ink-faint">Used</dt>
                  <dd className="text-ink">{formatBytes(drive.usedBytes)}</dd>
                </div>
                <div className="flex justify-between gap-2">
                  <dt className="text-ink-faint">Free</dt>
                  <dd className="text-ok">{formatBytes(drive.freeBytes)}</dd>
                </div>
              </dl>
            </div>
          </div>
          <p className="mt-1.5 text-[10px] leading-relaxed text-ink-faint">
            {formatPercent(drive.totalBytes > 0 ? drive.usedBytes / drive.totalBytes : 0)} of{' '}
            {drive.fileSystem} volume {drive.id}
          </p>
        </Panel>
      ) : (
        <Panel className="p-3">
          <p className="text-xs text-ink-muted">Detecting drives…</p>
        </Panel>
      )}

      {/* ---------------- live scan progress ---------------- */}
      {scanning && progress ? (
        <Panel className="p-2.5" data-testid="scan-progress">
          <SectionTitle className="px-0 pt-0 pb-1">Scanning</SectionTitle>
          <ProgressBar
            value={progress.percent === null ? 0 : progress.percent / 100}
            label="Scan progress"
            className="mb-2"
          />
          <dl className="dw-tnum space-y-0.5 text-[11px]">
            <Row label="Files" value={formatCount(progress.filesScanned)} />
            <Row label="Folders" value={formatCount(progress.foldersScanned)} />
            <Row label="Analyzed" value={formatBytes(progress.bytesAnalyzed)} />
            <Row label="Elapsed" value={formatDuration(progress.elapsedMs)} />
            <Row
              label="Remaining"
              value={progress.etaMs !== null ? formatDuration(progress.etaMs) : 'estimating…'}
            />
            {progress.errors > 0 ? <Row label="Skipped" value={formatCount(progress.errors)} /> : null}
          </dl>
          <p className="mt-1.5 truncate text-[10px] text-ink-faint" title={progress.currentPath}>
            {progress.currentPath}
          </p>
          <Button
            size="sm"
            variant="secondary"
            className="mt-2 w-full justify-center"
            onClick={() => void actions.cancelScan()}
            disabled={scan.cancelling}
            data-testid="cancel-scan"
          >
            <X aria-hidden className="size-3" />
            {scan.cancelling ? 'Cancelling…' : 'Cancel scan'}
          </Button>
        </Panel>
      ) : null}

      {/* ---------------- scan error ---------------- */}
      {scan.error ? (
        <Panel className="border-danger/30 p-2.5">
          <SectionTitle className="px-0 pt-0 pb-1">Scan failed</SectionTitle>
          <p className="break-words text-[11px] leading-relaxed text-danger">{scan.error}</p>
        </Panel>
      ) : null}

      {/* ---------------- quick wins ---------------- */}
      <div className="mt-auto">
        <SectionTitle
          action={
            quickWinTotal > 0 ? (
              <span className="dw-tnum text-[10px] font-medium text-ink-muted">
                {formatBytes(quickWinTotal)}
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
          <Panel className="p-2.5">
            <p className="text-[11px] leading-relaxed text-ink-muted">
              {scan.result
                ? 'Nothing obvious to reclaim on this volume.'
                : 'Scan a drive to see where your space is going.'}
            </p>
          </Panel>
        ) : (
          <ul className="flex flex-col gap-1" data-testid="quick-wins">
            {quickWins.map((win) => (
              <li key={win.id}>
                <button
                  type="button"
                  onClick={() => {
                    if (win.id === 'large-media') {
                      actions.navigate('large-files');
                    } else if (win.path) {
                      actions.navigate('analyze');
                      actions.focusPath(win.path);
                    } else {
                      actions.navigate('large-files');
                    }
                  }}
                  className="group flex w-full items-center gap-2 rounded-control border border-line bg-surface px-2 py-1.5 text-left shadow-panel transition-colors hover:border-line-strong"
                  title={win.hint}
                >
                  <span
                    aria-hidden
                    className="size-2 shrink-0 rounded-sm ring-1 ring-black/5"
                    style={{ background: categoryColor(win.category) }}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[12px] text-ink">{win.label}</span>
                    <span className="dw-tnum block text-[10px] text-ink-faint">
                      {formatCount(win.fileCount)} files
                    </span>
                  </span>
                  <span className="dw-tnum shrink-0 text-[11px] font-medium text-ink">
                    {formatBytes(win.bytes)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}

        {viz.selectedPath ? (
          <p className="mt-1.5 flex items-center gap-1 px-1 text-[10px] text-ink-faint">
            <Clock aria-hidden className="size-3" />
            Selection updates the details panel
          </p>
        ) : null}
      </div>
    </aside>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-2">
      <dt className="text-ink-faint">{label}</dt>
      <dd className="text-ink">{value}</dd>
    </div>
  );
}

export { NAV };
