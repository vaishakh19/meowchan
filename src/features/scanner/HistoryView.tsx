/**
 * Scan history.
 *
 * Diskora keeps a bounded local record of past scans in SQLite so the user can compare volumes
 * over time. The list is informational: nothing here deletes anything.
 */

import { useMemo, useState } from 'react';
import { Database, Download, GitCompare, History, Play } from 'lucide-react';
import { useApp } from '@/app/state';
import { Button, EmptyState, Panel, SectionTitle, cx } from '@/components/ui/primitives';
import { formatBytes, formatCount, formatDateTime, formatDuration, formatRelativeTime } from '@/lib/format';
import { CompareScansView } from './CompareScansView';
import type { ScanSummary } from '@/types/contracts';

export function HistoryView(): React.JSX.Element {
  const { state } = useApp();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showCompare, setShowCompare] = useState(false);
  const history = state.scan.history;
  const record = useMemo(
    () => history.find((item) => item.scanId === selectedId) ?? null,
    [history, selectedId],
  );

  if (showCompare) {
    return <CompareScansView onClose={() => setShowCompare(false)} />;
  }

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="history-view">
      <header className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-surface px-3 py-2">
        <div>
          <h1 className="text-[13px] font-semibold text-ink">Scan history</h1>
          <p className="mt-0.5 text-[10px] text-ink-faint">
            Stored locally in this PC's SQLite database. Diskora never uploads scan data.
          </p>
        </div>
        {history.length >= 2 ? (
          <Button size="sm" variant="secondary" onClick={() => setShowCompare(true)}>
            <GitCompare aria-hidden className="size-3" />
            Compare Scans
          </Button>
        ) : null}
      </header>

      <div className="flex min-h-0 flex-1">
        <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2">
          {history.length === 0 ? (
            <Panel>
              <EmptyState
                icon={<History aria-hidden className="size-6" />}
                title="No saved scans yet"
                detail="Scans you run appear here with their totals, duration and whether the result was complete."
              />
            </Panel>
          ) : (
            <table className="w-full text-left">
              <thead>
                <tr className="border-b border-line text-[10px] uppercase tracking-wide text-ink-faint">
                  <th className="py-1.5 pr-2 font-semibold">Volume</th>
                  <th className="py-1.5 pr-2 font-semibold">Finished</th>
                  <th className="py-1.5 pr-2 text-right font-semibold">Size</th>
                  <th className="py-1.5 pr-2 text-right font-semibold">Files</th>
                  <th className="py-1.5 pr-2 text-right font-semibold">Duration</th>
                  <th className="py-1.5 font-semibold">Status</th>
                </tr>
              </thead>
              <tbody>
                {history.map((item) => (
                  <tr
                    key={item.scanId}
                    onClick={() => setSelectedId(item.scanId)}
                    className={cx(
                      'cursor-pointer border-b border-line/60 text-[11px] transition-colors hover:bg-surface-sunken',
                      selectedId === item.scanId && 'bg-info-soft',
                    )}
                  >
                    <td className="py-1.5 pr-2 text-ink">{item.rootName}</td>
                    <td className="py-1.5 pr-2 text-ink-muted" title={item.finishedAt === null ? '' : formatDateTime(item.finishedAt)}>
                      {formatRelativeTime(item.finishedAt)}
                    </td>
                    <td className="dw-tnum py-1.5 pr-2 text-right text-ink">{formatBytes(item.totalBytes)}</td>
                    <td className="dw-tnum py-1.5 pr-2 text-right text-ink-muted">{formatCount(item.fileCount)}</td>
                    <td className="dw-tnum py-1.5 pr-2 text-right text-ink-muted">
                      {item.durationMs === null ? '—' : formatDuration(item.durationMs)}
                    </td>
                    <td className="py-1.5">
                      <span className={cx('text-[10px]', item.status === 'complete' ? 'text-ok' : 'text-warn')}>
                        {item.status === 'complete'
                          ? 'complete'
                          : `${formatCount(item.errorCount)} unreadable`}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>

        {record ? <RecordDetail record={record} /> : null}
      </div>
    </div>
  );
}

function RecordDetail({ record }: { record: ScanSummary }): React.JSX.Element {
  const { actions } = useApp();
  return (
    <aside className="w-64 shrink-0 overflow-y-auto border-l border-line bg-surface p-3" aria-label="Scan details">
      <SectionTitle className="px-0 pt-0">Scan details</SectionTitle>
      <dl className="dw-tnum space-y-1 text-[11px]">
        <Row label="Volume" value={record.rootName} />
        <Row label="Path" value={record.rootPath} mono />
        <Row label="Started" value={formatDateTime(record.startedAt)} />
        <Row label="Finished" value={record.finishedAt === null ? '—' : formatDateTime(record.finishedAt)} />
        <Row label="Duration" value={record.durationMs === null ? '—' : formatDuration(record.durationMs)} />
        <Row label="Analyzed" value={formatBytes(record.totalBytes)} />
        <Row label="Files" value={formatCount(record.fileCount)} />
        <Row label="Folders" value={formatCount(record.folderCount)} />
        <Row label="Unreadable" value={formatCount(record.errorCount)} />
        <Row label="Status" value={record.status} />
        <Row label="Persisted" value={record.persisted ? 'yes' : 'in memory only'} />
      </dl>
      <Button
        size="sm"
        className="mt-3 w-full justify-center"
        onClick={() => {
          void actions.scanPath(record.rootPath);
        }}
      >
        <Play aria-hidden className="size-3" />
        Rescan this path
      </Button>
      <Button
        size="sm"
        variant="secondary"
        className="mt-1.5 w-full justify-center"
        onClick={async () => {
          try {
            const transport = await import('@/lib/transport').then((m) => m.getTransport());
            const csv = await transport.exportScanCsv({ scanId: record.scanId, exportType: 'all-files' });
            const blob = new Blob([csv], { type: 'text/csv;charset=utf-8;' });
            const url = URL.createObjectURL(blob);
            const a = document.createElement('a');
            a.href = url;
            a.download = `diskora-scan-${record.scanId}.csv`;
            a.click();
            URL.revokeObjectURL(url);
            actions.toast({ kind: 'success', title: 'Exported CSV', detail: 'Scan CSV exported.' });
          } catch (err) {
            actions.toast({ kind: 'error', title: 'Export failed', detail: String(err) });
          }
        }}
      >
        <Download aria-hidden className="size-3" />
        Export CSV
      </Button>
      <p className="mt-2 flex items-start gap-1.5 text-[10px] leading-relaxed text-ink-faint">
        <Database aria-hidden className="mt-px size-3 shrink-0" />
        Historical comparison shows how a volume changed between scans.
      </p>
    </aside>
  );
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <dt className="shrink-0 text-ink-faint">{label}</dt>
      <dd
        className={cx('min-w-0 truncate text-right text-ink', mono && 'font-mono text-[10px]')}
        title={value}
      >
        {value}
      </dd>
    </div>
  );
}
