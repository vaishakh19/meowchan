/**
 * Per-User Storage Attribution.
 *
 * Breaks down storage utilization across user profiles (C:\Users\<user> or /home/<user>).
 * Accurately aggregates document folders, AppData/caches, downloads, and developer environments.
 */

import { useCallback, useEffect, useState } from 'react';
import { Download, Folder, RefreshCw, User, Users } from 'lucide-react';
import { useApp } from '@/app/state';
import { Button, EmptyState, Panel, SectionTitle, Spinner } from '@/components/ui/primitives';
import { formatBytes, formatCount } from '@/lib/format';
import { categoryColor, categoryLabel } from '@/lib/categories';
import type { StorageCategory, UserStorageReport } from '@/types/contracts';

export function UserStorageView({ onClose }: { onClose?: () => void }): React.JSX.Element {
  const { state, actions } = useApp();
  const { scan } = state;
  const [report, setReport] = useState<UserStorageReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);

  const scanId = scan.result?.scanId;

  const loadReport = useCallback(async () => {
    if (!scanId) return;
    setLoading(true);
    setError(null);
    try {
      const transport = await import('@/lib/transport').then((m) => m.getTransport());
      const res = await transport.userStorageReport(scanId);
      setReport(res);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [scanId]);

  useEffect(() => {
    void loadReport();
  }, [loadReport]);

  const handleExportCsv = async () => {
    if (!scanId) return;
    setExporting(true);
    try {
      const transport = await import('@/lib/transport').then((m) => m.getTransport());
      const csv = await transport.exportScanCsv({ scanId, exportType: 'user-attribution' });
      const blob = new Blob([csv], { type: 'text/csv;charset=utf-8;' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `diskora-user-attribution-${scanId}.csv`;
      a.click();
      URL.revokeObjectURL(url);
      actions.toast({ kind: 'success', title: 'Exported CSV', detail: 'User attribution report saved.' });
    } catch (err) {
      actions.toast({ kind: 'error', title: 'Export failed', detail: String(err) });
    } finally {
      setExporting(false);
    }
  };

  if (!scan.result) {
    return (
      <Panel className="m-4 p-6">
        <EmptyState
          icon={<Users aria-hidden className="size-6" />}
          title="No scan active"
          detail="Scan a volume to view per-user storage attribution."
        />
      </Panel>
    );
  }

  return (
    <div className="flex flex-col h-full overflow-y-auto p-4 space-y-4" data-testid="user-storage-view">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-line pb-3">
        <div>
          <h2 className="text-base font-semibold text-ink flex items-center gap-2">
            <Users aria-hidden className="size-4 text-accent" />
            Per-User Storage Attribution
          </h2>
          <p className="text-xs text-ink-muted mt-0.5">
            Space allocated across user profiles and account directories on {scan.result.rootName}.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" onClick={() => void handleExportCsv()} disabled={exporting || !report}>
            <Download aria-hidden className="size-3" />
            {exporting ? 'Exporting…' : 'Export CSV'}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => void loadReport()} disabled={loading}>
            <RefreshCw aria-hidden className="size-3" />
            Refresh
          </Button>
          {onClose ? (
            <Button size="sm" variant="secondary" onClick={onClose}>
              Back to scan
            </Button>
          ) : null}
        </div>
      </div>

      {loading ? (
        <Panel className="flex items-center justify-center p-8">
          <Spinner label="Attributing storage across user profiles…" />
        </Panel>
      ) : error ? (
        <Panel className="p-4 text-xs text-danger">
          <p className="font-semibold">Failed to analyze user storage:</p>
          <p className="mt-1 font-mono">{error}</p>
        </Panel>
      ) : !report || report.users.length === 0 ? (
        <Panel className="p-6">
          <EmptyState
            icon={<User aria-hidden className="size-6" />}
            title="No separate user profiles found"
            detail="The scanned root does not contain standard User Profile directories (e.g. C:\Users\<Username>)."
          />
        </Panel>
      ) : (
        <div className="space-y-4">
          <div className="grid grid-cols-[repeat(auto-fit,minmax(320px,1fr))] gap-3">
            {report.users.map((user) => (
              <Panel key={user.username} className="p-4 flex flex-col justify-between" data-testid={`user-card-${user.username}`}>
                <div>
                  <div className="flex items-start justify-between gap-2 border-b border-line pb-2">
                    <div className="flex items-center gap-2 min-w-0">
                      <div className="flex size-7 shrink-0 items-center justify-center rounded-full bg-accent-soft text-accent">
                        <User aria-hidden className="size-4" />
                      </div>
                      <div className="min-w-0">
                        <h3 className="font-semibold text-sm text-ink truncate">{user.username}</h3>
                        <p className="font-mono text-[10px] text-ink-muted truncate" title={user.userPath}>
                          {user.userPath}
                        </p>
                      </div>
                    </div>
                    <div className="text-right shrink-0">
                      <p className="dw-tnum text-sm font-bold text-ink">{formatBytes(user.totalBytes)}</p>
                      <p className="dw-tnum text-[10px] text-ink-muted">{formatCount(user.fileCount)} files</p>
                    </div>
                  </div>

                  {/* Category breakdown */}
                  <div className="mt-3">
                    <SectionTitle className="px-0 pt-0 pb-1 text-[10px]">Categories</SectionTitle>
                    <div className="space-y-1">
                      {Object.entries(user.categories)
                        .sort((a, b) => b[1] - a[1])
                        .slice(0, 5)
                        .map(([cat, bytes]) => (
                          <div key={cat} className="flex items-center justify-between text-[11px]">
                            <span className="flex items-center gap-1.5 text-ink-muted">
                              <span
                                className="size-2 rounded-sm"
                                style={{ background: categoryColor(cat as StorageCategory) }}
                              />
                              {categoryLabel(cat as StorageCategory)}
                            </span>
                            <span className="dw-tnum font-medium text-ink">{formatBytes(bytes)}</span>
                          </div>
                        ))}
                    </div>
                  </div>

                  {/* Top user folders */}
                  {user.topFolders.length > 0 ? (
                    <div className="mt-3">
                      <SectionTitle className="px-0 pt-0 pb-1 text-[10px]">Top folders</SectionTitle>
                      <div className="space-y-1">
                        {user.topFolders.slice(0, 5).map((folder) => (
                          <button
                            key={folder.path}
                            type="button"
                            onClick={() => {
                              actions.navigate('analyze');
                              actions.focusPath(folder.path);
                            }}
                            className="flex w-full items-center justify-between rounded px-1.5 py-1 text-left text-[11px] hover:bg-surface-sunken transition-colors"
                          >
                            <span className="flex items-center gap-1.5 min-w-0 truncate text-ink">
                              <Folder aria-hidden className="size-3 shrink-0 text-ink-muted" />
                              <span className="truncate">{folder.name}</span>
                            </span>
                            <span className="dw-tnum shrink-0 font-medium text-ink-muted">
                              {formatBytes(folder.sizeBytes)}
                            </span>
                          </button>
                        ))}
                      </div>
                    </div>
                  ) : null}
                </div>
              </Panel>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
