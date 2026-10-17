/**
 * Scan Growth Comparison.
 *
 * Compares two persistent scans of a volume to visualize disk space growth or reduction.
 * Shows net byte changes, category-level growth deltas, and the top growing / freed directories.
 */

import { shownPath } from '@/lib/path';
import { useCallback, useEffect, useState } from 'react';
import { ArrowDown, ArrowUp, GitCompare, TrendingDown, TrendingUp } from 'lucide-react';
import { useApp } from '@/app/state';
import { Button, EmptyState, Panel, SectionTitle, Select, Spinner, cx } from '@/components/ui/primitives';
import { formatBytes, formatCount, formatDateTime } from '@/lib/format';
import { categoryColor, categoryLabel } from '@/lib/categories';
import type { ScanGrowthComparison, StorageCategory } from '@/types/contracts';

export function CompareScansView({ onClose }: { onClose?: () => void }): React.JSX.Element {
  const { state } = useApp();
  const { history } = state.scan;

  const [baselineId, setBaselineId] = useState<string>(history[1]?.scanId ?? history[0]?.scanId ?? '');
  const [currentId, setCurrentId] = useState<string>(history[0]?.scanId ?? '');
  const [comparison, setComparison] = useState<ScanGrowthComparison | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const runComparison = useCallback(async () => {
    if (!baselineId || !currentId || baselineId === currentId) return;
    setLoading(true);
    setError(null);
    try {
      const transport = await import('@/lib/transport').then((m) => m.getTransport());
      const res = await transport.compareScans(baselineId, currentId);
      setComparison(res);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [baselineId, currentId]);

  useEffect(() => {
    void runComparison();
  }, [runComparison]);

  if (history.length < 2) {
    return (
      <Panel className="m-4 p-6">
        <EmptyState
          icon={<GitCompare aria-hidden className="size-6" />}
          title="Need at least two scans to compare"
          detail="Run additional scans on this volume over time to track disk space growth and changes."
        />
      </Panel>
    );
  }

  return (
    <div className="flex flex-col h-full overflow-y-auto p-4 space-y-4" data-testid="compare-scans-view">
      {/* Header & scan selectors */}
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-line pb-3">
        <div>
          <h2 className="text-base font-semibold text-ink flex items-center gap-2">
            <GitCompare aria-hidden className="size-4 text-accent" />
            Storage Growth & Scan Comparison
          </h2>
          <p className="text-xs text-ink-muted mt-0.5">
            Compare two scans to see what consumed or freed storage over time.
          </p>
        </div>
        {onClose ? (
          <Button size="sm" variant="secondary" onClick={onClose}>
            Back to history
          </Button>
        ) : null}
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        <Panel className="p-3">
          <SectionTitle className="px-0 pt-0 pb-1 text-[11px]">Baseline scan (Older)</SectionTitle>
          <Select
            value={baselineId}
            onChange={(e) => setBaselineId(e.target.value)}
            className="w-full text-xs"
          >
            {history.map((s) => (
              <option key={s.scanId} value={s.scanId}>
                {s.rootName} — {formatDateTime(s.startedAt)} ({formatBytes(s.totalBytes)})
              </option>
            ))}
          </Select>
        </Panel>

        <Panel className="p-3">
          <SectionTitle className="px-0 pt-0 pb-1 text-[11px]">Current scan (Newer)</SectionTitle>
          <Select
            value={currentId}
            onChange={(e) => setCurrentId(e.target.value)}
            className="w-full text-xs"
          >
            {history.map((s) => (
              <option key={s.scanId} value={s.scanId}>
                {s.rootName} — {formatDateTime(s.startedAt)} ({formatBytes(s.totalBytes)})
              </option>
            ))}
          </Select>
        </Panel>
      </div>

      {loading ? (
        <Panel className="flex items-center justify-center p-8">
          <Spinner label="Calculating storage delta between scans…" />
        </Panel>
      ) : error ? (
        <Panel className="p-4 text-xs text-danger">
          <p className="font-semibold">Could not compare scans:</p>
          <p className="mt-1 font-mono">{error}</p>
        </Panel>
      ) : !comparison ? null : (
        <div className="space-y-4">
          {/* Summary Delta cards */}
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
            <Panel className="p-3.5">
              <span className="text-[11px] font-medium text-ink-muted">Net Storage Delta</span>
              <div className="mt-1 flex items-baseline gap-2">
                <span
                  className={cx(
                    'text-xl font-bold tracking-tight dw-tnum',
                    comparison.bytesDelta > 0 ? 'text-warn' : comparison.bytesDelta < 0 ? 'text-ok' : 'text-ink',
                  )}
                >
                  {comparison.bytesDelta > 0 ? '+' : ''}
                  {formatBytes(comparison.bytesDelta)}
                </span>
                {comparison.bytesDelta > 0 ? (
                  <TrendingUp className="size-4 text-warn" />
                ) : comparison.bytesDelta < 0 ? (
                  <TrendingDown className="size-4 text-ok" />
                ) : null}
              </div>
            </Panel>

            <Panel className="p-3.5">
              <span className="text-[11px] font-medium text-ink-muted">Files Changed</span>
              <p className="mt-1 text-xl font-bold tracking-tight text-ink dw-tnum">
                {comparison.filesDelta > 0 ? '+' : ''}
                {formatCount(comparison.filesDelta)}
              </p>
            </Panel>

            <Panel className="p-3.5">
              <span className="text-[11px] font-medium text-ink-muted">Folders Changed</span>
              <p className="mt-1 text-xl font-bold tracking-tight text-ink dw-tnum">
                {comparison.foldersDelta > 0 ? '+' : ''}
                {formatCount(comparison.foldersDelta)}
              </p>
            </Panel>
          </div>

          {/* Category changes */}
          <Panel className="p-4">
            <SectionTitle className="px-0 pt-0 pb-2">Category Growth Deltas</SectionTitle>
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-2">
              {Object.entries(comparison.categoryDeltas).map(([cat, delta]) => (
                <div key={cat} className="rounded-control bg-surface-sunken p-2.5 text-xs">
                  <div className="flex items-center gap-1.5 text-ink-muted">
                    <span
                      className="size-2 rounded-sm"
                      style={{ background: categoryColor(cat as StorageCategory) }}
                    />
                    <span className="truncate">{categoryLabel(cat as StorageCategory)}</span>
                  </div>
                  <p
                    className={cx(
                      'mt-1 font-semibold dw-tnum',
                      delta > 0 ? 'text-warn' : delta < 0 ? 'text-ok' : 'text-ink-muted',
                    )}
                  >
                    {delta > 0 ? '+' : ''}
                    {formatBytes(delta)}
                  </p>
                </div>
              ))}
            </div>
          </Panel>

          {/* Growing vs Reclaimed */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            <Panel className="p-4">
              <SectionTitle className="px-0 pt-0 pb-2 text-warn flex items-center gap-1.5">
                <ArrowUp className="size-3.5" />
                Top Growing Folders
              </SectionTitle>
              {comparison.growingFolders.length === 0 ? (
                <p className="text-xs text-ink-muted">No folders with significant growth.</p>
              ) : (
                <div className="space-y-1.5">
                  {comparison.growingFolders.map((f) => (
                    <div
                      key={f.path}
                      className="flex items-center justify-between rounded p-1.5 hover:bg-surface-sunken text-xs"
                    >
                      <div className="min-w-0 flex-1 truncate pr-2">
                        <p className="font-medium text-ink truncate">{f.name}</p>
                        <p className="font-mono text-[9px] text-ink-faint truncate">{shownPath(f)}</p>
                      </div>
                      <span className="dw-tnum font-semibold text-warn shrink-0">
                        +{formatBytes(f.deltaBytes)}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </Panel>

            <Panel className="p-4">
              <SectionTitle className="px-0 pt-0 pb-2 text-ok flex items-center gap-1.5">
                <ArrowDown className="size-3.5" />
                Top Freed / Reclaimed Folders
              </SectionTitle>
              {comparison.reclaimedFolders.length === 0 ? (
                <p className="text-xs text-ink-muted">No folders with significant space freed.</p>
              ) : (
                <div className="space-y-1.5">
                  {comparison.reclaimedFolders.map((f) => (
                    <div
                      key={f.path}
                      className="flex items-center justify-between rounded p-1.5 hover:bg-surface-sunken text-xs"
                    >
                      <div className="min-w-0 flex-1 truncate pr-2">
                        <p className="font-medium text-ink truncate">{f.name}</p>
                        <p className="font-mono text-[9px] text-ink-faint truncate">{shownPath(f)}</p>
                      </div>
                      <span className="dw-tnum font-semibold text-ok shrink-0">
                        {formatBytes(f.deltaBytes)}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </Panel>
          </div>
        </div>
      )}
    </div>
  );
}
