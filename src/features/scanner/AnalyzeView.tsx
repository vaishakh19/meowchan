/**
 * Main visualization area: header, breadcrumb, controls and the active view.
 *
 * Every number in the header comes from the scan result — there is no placeholder data. The four
 * states (no scan, scanning, results, partial/failed) are all explicit so the user is never left
 * guessing what the app is doing.
 */

import type { TreeNode } from '@/types/contracts';
import { useMemo, useState } from 'react';
import { ChevronRight, Clock, Crosshair, Download, FolderTree, LayoutGrid, List, PieChart, RotateCcw, Search, ShieldAlert, Users, ZoomIn, Lock } from 'lucide-react';
import { useItemMenu } from '@/app/itemMenu';
import { useApp, useFocusNode } from '@/app/state';
import { Sunburst } from '@/features/visualization/Sunburst';
import { Treemap } from '@/features/visualization/Treemap';
import { FileList } from '@/features/visualization/FileList';
import { UserStorageView } from '@/features/scanner/UserStorageView';
import { Badge, Button, EmptyState, ErrorState, Panel, ProgressBar, SegmentedControl, Spinner, cx, ProBadge } from '@/components/ui/primitives';
import { formatBytes, formatCount, formatDateTime, formatDuration, formatPercent } from '@/lib/format';
import { categoryBreakdown } from '@/features/visualization/layout';
import { categoryColor, categoryLabel } from '@/lib/categories';
import { breadcrumbs, isWithin, shownPath } from '@/lib/path';
import { ancestorChain } from '@/features/visualization/layout';
import type { ViewMode } from '@/app/state';

export function AnalyzeView(): React.JSX.Element {
  const { state, actions } = useApp();
  const { scan, viz } = state;
  const focusNode = useFocusNode();
  const result = scan.result;
  const [showUserStorage, setShowUserStorage] = useState(false);
  const [exporting, setExporting] = useState(false);

  const itemMenu = useItemMenu();
  const openMenu = (path: string, x: number, y: number, kind?: 'file' | 'folder') =>
    itemMenu.open(path, x, y, kind ?? focusNode?.children.find((child) => child.path === path)?.kind ?? 'folder');

  const activePath = result
    ? viz.mode === 'list'
      ? viz.focusPath ?? result.rootPath
      : focusNode?.path ?? result.rootPath
    : null;
  const proEnabled = state.pro?.enabled === true;
  const crumbs = useMemo(() => {
    if (!activePath || !result) return [];
    // Real paths (Pro) keep the full drive trail. Free-tier paths are opaque tokens, so the trail
    // is read from the tree instead.
    if (proEnabled) return breadcrumbs(activePath);
    return ancestorChain(result.tree, activePath).map((node) => ({ label: node.name, path: node.path }));
  }, [activePath, result, proEnabled]);
  const activeDisplay = focusNode ? shownPath(focusNode) : (activePath ?? '');

  const handleExportCsv = async () => {
    if (!result) return;
    // The CSV holds real paths, so it is a Pro feature. Locked, it explains itself instead.
    if (!proEnabled) {
      actions.promptUpgrade('CSV export with full paths');
      return;
    }
    setExporting(true);
    try {
      const transport = await import('@/lib/transport').then((m) => m.getTransport());
      const csv = await transport.exportScanCsv({ scanId: result.scanId, exportType: 'all-files' });
      const blob = new Blob([csv], { type: 'text/csv;charset=utf-8;' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `diskora-scan-${result.scanId}.csv`;
      a.click();
      URL.revokeObjectURL(url);
      actions.toast({ kind: 'success', title: 'Exported CSV', detail: 'Complete scan results exported to CSV.' });
    } catch (err) {
      actions.toast({ kind: 'error', title: 'Export failed', detail: String(err) });
    } finally {
      setExporting(false);
    }
  };

  if (scan.scanning) {
    return (
      <div className="flex h-full flex-col" data-testid="analyze-scanning">
        <ScanHeader />
        <Panel className="m-3 flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-6">
          <Spinner label={scan.cancelling ? 'Cancelling…' : 'Scanning…'} />
          <p className="max-w-md text-center text-xs leading-relaxed text-ink-muted">
            Diskora walks the volume with bounded concurrency and never blocks the interface. You
            can keep using the app — or cancel at any time.
          </p>
          {scan.progress ? (
            <div className="w-full max-w-sm">
              <ProgressBar
                value={scan.progress.percent === null ? 0 : scan.progress.percent / 100}
                indeterminate={scan.progress.percent === null}
                label="Scan progress"
              />
              <dl className="dw-tnum mt-2 grid grid-cols-2 gap-x-4 gap-y-0.5 text-[11px]">
                <Stat label="Entries" value={formatCount(scan.progress.entriesScanned)} />
                <Stat label="Files" value={formatCount(scan.progress.filesScanned)} />
                <Stat label="Folders" value={formatCount(scan.progress.foldersScanned)} />
                <Stat label="Analyzed" value={formatBytes(scan.progress.bytesAnalyzed)} />
                <Stat label="Elapsed" value={formatDuration(scan.progress.elapsedMs)} />
              </dl>
              <p className="mt-2 truncate font-mono text-[10px] text-ink-faint" title={scan.progress.currentPathDisplay ?? scan.progress.currentPath}>
                {scan.progress.currentPathDisplay ?? scan.progress.currentPath}
              </p>
            </div>
          ) : null}
        </Panel>
      </div>
    );
  }

  if (scan.error) {
    return (
      <div className="flex h-full flex-col">
        <ScanHeader />
        <ErrorState
          title="The scan could not be completed"
          detail={scan.error}
          onRetry={() => void actions.scanDrive()}
        />
      </div>
    );
  }

  if (!result || !focusNode) {
    return (
      <div className="flex h-full flex-col">
        <ScanHeader />
        <EmptyState
          icon={<FolderTree aria-hidden className="size-6" />}
          title="No scan yet"
          detail="Pick a drive in the sidebar and press Scan Drive, or scan a single folder. Everything you see afterwards is measured from your own filesystem."
          action={
            <Button variant="primary" onClick={() => void actions.scanDrive()} disabled={state.drives.length === 0}>
              Scan the selected drive
            </Button>
          }
        />
      </div>
    );
  }

  const breakdown = focusNode.path === activePath ? categoryBreakdown(focusNode).slice(0, 6) : [];

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="analyze-view">
      {/* ------------------------------ header ------------------------------ */}
      <header className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line bg-surface px-3 py-2">
        <div className="min-w-0 flex-1">
          <nav aria-label="Breadcrumb" className="flex flex-wrap items-center gap-0.5 text-[12px]">
            {crumbs.map((crumb, index) => {
              const last = index === crumbs.length - 1;
              const isScannedPath = proEnabled ? isWithin(crumb.path, result.rootPath) : true;
              const disabled = last || !isScannedPath;
              return (
                <span key={crumb.path} className="flex items-center gap-0.5">
                  {index > 0 ? <ChevronRight aria-hidden className="size-3 text-ink-faint" /> : null}
                  <button
                    type="button"
                    disabled={disabled}
                    onClick={() => actions.focusPath(crumb.path)}
                    className={cx(
                      'rounded px-1 py-px transition-colors',
                      disabled ? 'font-medium text-ink' : 'text-ink-muted hover:bg-surface-sunken hover:text-ink',
                    )}
                    aria-current={last ? 'location' : undefined}
                  >
                    {crumb.label}
                  </button>
                </span>
              );
            })}
          </nav>
          <p className="mt-0.5 truncate font-mono text-[10px] text-ink-faint" title={activeDisplay}>
            {activeDisplay}
          </p>
        </div>

        <dl className="dw-tnum flex items-center gap-3 text-[11px]">
          <div className="text-right">
            <dt className="text-ink-faint">Analyzed</dt>
            <dd className="text-[13px] font-semibold text-ink" data-testid="analyzed-size">
              {formatBytes(result.totalBytes)}
            </dd>
          </div>
          <div className="text-right">
            <dt className="text-ink-faint">Files</dt>
            <dd className="text-ink">{formatCount(result.fileCount)}</dd>
          </div>
          <div className="text-right">
            <dt className="text-ink-faint">Folders</dt>
            <dd className="text-ink">{formatCount(result.folderCount)}</dd>
          </div>
          <div className="text-right">
            <dt className="text-ink-faint">Last scan</dt>
            <dd className="text-ink">{formatDateTime(result.finishedAt ?? result.startedAt)}</dd>
          </div>
        </dl>
      </header>

      {/* ----------------------------- controls ----------------------------- */}
      <div className="flex flex-wrap items-center gap-2 border-b border-line bg-canvas px-3 py-1.5">
        <SegmentedControl<ViewMode>
          label="Visualization"
          value={viz.mode}
          onChange={actions.setViewMode}
          options={[
            { value: 'sunburst', label: 'Sunburst', icon: <PieChart aria-hidden className="size-3" /> },
            { value: 'treemap', label: 'Treemap', icon: <LayoutGrid aria-hidden className="size-3" /> },
            { value: 'list', label: 'File list', icon: <List aria-hidden className="size-3" /> },
          ]}
        />

        {viz.mode !== 'list' ? (
          <span className="relative">
            <Search aria-hidden className="pointer-events-none absolute left-2 top-1/2 size-3 -translate-y-1/2 text-ink-faint" />
            <input
              value={viz.search}
              onChange={(event) => actions.setSearch(event.target.value)}
              placeholder="Search within results"
              aria-label="Search within results"
              className="h-7 w-44 rounded-control border border-line bg-surface pl-7 pr-2 text-xs text-ink placeholder:text-ink-faint focus:border-info focus:outline-none"
            />
          </span>
        ) : null}

        <div className="ml-auto flex items-center gap-1">
          <Button
            size="sm"
            variant={showUserStorage ? 'primary' : 'secondary'}
            onClick={() => setShowUserStorage(!showUserStorage)}
            title="View storage attribution per user profile"
          >
            <Users aria-hidden className="size-3" />
            User Profiles
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => void handleExportCsv()}
            disabled={exporting}
            title={proEnabled ? 'Export scan results to CSV' : 'CSV export with full paths is a Diskora Pro feature'}
          >
            <Download aria-hidden className="size-3" />
            {exporting ? 'Exporting…' : 'Export CSV'}
            {!proEnabled ? <Lock aria-hidden className="size-3" /> : null}
            {!proEnabled ? <ProBadge /> : null}
          </Button>
          {result.prunedNodes > 0 ? (
            <Badge tone="warn" className="mr-1">
              {formatCount(result.prunedNodes)} tree items folded into “Other”
            </Badge>
          ) : null}
          {result.truncated ? (
            <Badge tone="warn" className="mr-1">
              Entry safety limit reached
            </Badge>
          ) : null}
          {result.status === 'partial' ? (
            <Badge tone="warn" className="mr-1">
              Partial results
            </Badge>
          ) : null}
          <Button
            size="sm"
            variant="ghost"
            onClick={() => actions.focusPath(result.rootPath)}
            disabled={viz.focusPath === result.rootPath}
            title="Reset the view to the scan root"
          >
            <RotateCcw aria-hidden className="size-3" />
            Reset
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => actions.focusPath(result.rootPath)}
            title="Zoom out to the scan root"
          >
            <ZoomIn aria-hidden className="size-3" />
            Zoom out
          </Button>
        </div>
      </div>

      {/* --------------------------- visualization --------------------------- */}
      {showUserStorage ? (
        <div className="relative min-h-0 flex-1 bg-surface">
          <UserStorageView onClose={() => setShowUserStorage(false)} />
        </div>
      ) : (
      <div className="relative min-h-0 flex-1">
        {viz.mode === 'sunburst' ? (
          <Sunburst
            key={result.scanId}
            root={focusNode}
            selectedPath={viz.selectedPath}
            onSelect={actions.selectNode}
            onNavigate={actions.focusPath}
            onNavigateUp={() => {
              const chain = ancestorChain(result.tree, focusNode.path);
              actions.focusPath(chain.length >= 2 ? chain[chain.length - 2].path : result.rootPath);
            }}
            canNavigateUp={viz.focusPath !== result.rootPath}
            onContextMenu={openMenu}
            className="absolute inset-0"
          />
        ) : viz.mode === 'treemap' ? (
          <Treemap
            root={focusNode}
            selectedPath={viz.selectedPath}
            onSelect={actions.selectNode}
            onNavigate={actions.focusPath}
            onContextMenu={openMenu}
            className="absolute inset-0"
          />
        ) : (
          <FileList
            root={focusNode}
            selectedPath={viz.selectedPath}
            scanId={result.scanId}
            rootPath={viz.focusPath ?? result.rootPath}
            scanRootPath={result.rootPath}
            upPath={parentIdentity(result.tree, viz.focusPath ?? result.rootPath)}
            sort={viz.sort}
            sortDir={viz.sortDir}
            search={viz.search}
            onSelect={actions.selectNode}
            onOpen={actions.focusPath}
            onSortChange={actions.setSort}
            onSearchChange={actions.setSearch}
            onContextMenu={openMenu}
            className="absolute inset-0 bg-surface"
          />
        )}
      </div>
      )}

      {/* ------------------------------ footer ------------------------------ */}
      <footer className="flex flex-wrap items-center gap-x-4 gap-y-1 border-t border-line bg-surface px-3 py-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-3">
          {breakdown.map((slice) => (
            <span key={slice.category} className="flex items-center gap-1.5 text-[10px] text-ink-muted">
              <span
                aria-hidden
                className="size-2 rounded-sm ring-1 ring-black/5"
                style={{ background: categoryColor(slice.category) }}
              />
              {categoryLabel(slice.category)}
              <span className="dw-tnum text-ink">{formatPercent(slice.fraction, 1)}</span>
            </span>
          ))}
        </div>
        <div className="flex items-center gap-3 text-[10px] text-ink-faint">
          {result.errors.length > 0 ? (
            <span className="flex items-center gap-1 text-warn">
              <ShieldAlert aria-hidden className="size-3" />
              {formatCount(result.errors.length)} unreadable locations
            </span>
          ) : null}
          {result.durationMs !== null ? (
            <span className="flex items-center gap-1">
              <Clock aria-hidden className="size-3" />
              {formatDuration(result.durationMs)}
            </span>
          ) : null}
          <span className="flex items-center gap-1">
            <Crosshair aria-hidden className="size-3" />
            {viz.selectedPath ? 'Item selected' : 'Click a segment for details'}
          </span>
        </div>
      </footer>
    </div>
  );
}

function ScanHeader(): React.JSX.Element {
  const { state, actions } = useApp();
  return (
    <header className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line bg-surface px-3 py-2">
      <div className="min-w-0 flex-1">
        <h1 className="text-[13px] font-semibold text-ink">
          {state.scan.scanning ? 'Scanning…' : 'Disk analysis'}
        </h1>
        <p className="mt-0.5 truncate text-[10px] text-ink-faint">
          {state.scan.progress?.currentPathDisplay ?? state.scan.progress?.currentPath ?? 'Preparing'}
        </p>
      </div>
      {state.scan.scanning ? (
        <Button size="sm" onClick={() => void actions.cancelScan()} disabled={state.scan.cancelling}>
          Cancel
        </Button>
      ) : null}
    </header>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <dt className="text-ink-faint">{label}</dt>
      <dd className="text-ink">{value}</dd>
    </div>
  );
}

/** Identity of the folder above `path`, read from the tree (works for free-tier tokens too). */
function parentIdentity(tree: TreeNode, path: string): string | null {
  const chain = ancestorChain(tree, path);
  return chain.length >= 2 ? chain[chain.length - 2].path : null;
}
