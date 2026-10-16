/**
 * Large file finder.
 *
 * Filters (minimum size, extension, modification window) run against the scan index in Rust, so
 * the results are exact and the browser never holds the whole file list. Opening a result reveals
 * its real location in File Explorer.
 */

import { useEffect, useMemo, useState } from 'react';
import { FileSearch, FolderOpen, Search } from 'lucide-react';
import { useApp } from '@/app/state';
import { useItemMenu } from '@/app/itemMenu';
import { Button, EmptyState, ErrorState, Panel, Select, TextField, cx } from '@/components/ui/primitives';
import { VirtualList } from '@/components/ui/VirtualList';
import { FileIcon } from '@/lib/icons';
import { formatBytes, formatBytesCompact, formatCount, formatDateTime, formatRelativeTime } from '@/lib/format';
import { parseBytes } from '@/lib/format';
import { categoryLabel } from '@/lib/categories';
import type { LargeFileSort } from '@/types/contracts';

const MB = 1024 * 1024;

const SIZE_PRESETS = [
  { label: '> 100 MB', bytes: 100 * MB },
  { label: '> 500 MB', bytes: 500 * MB },
  { label: '> 1 GB', bytes: 1024 * MB },
  { label: '> 5 GB', bytes: 5 * 1024 * MB },
];

const ROW_HEIGHT = 30;
const PAGE = 250;

export function LargeFilesView(): React.JSX.Element {
  const { state, actions } = useApp();
  const itemMenu = useItemMenu();
  const { largeFiles, scan } = state;

  const [minBytes, setMinBytes] = useState(100 * MB);
  const [customSize, setCustomSize] = useState('');
  const [extension, setExtension] = useState('');
  const [ageDays, setAgeDays] = useState('');
  const [sort, setSort] = useState<LargeFileSort>('size-desc');
  const [offset, setOffset] = useState(0);

  const hasScan = scan.result !== null;

  // Run the query whenever a filter changes (debounced by the input's own cadence).
  useEffect(() => {
    if (!hasScan) return;
    const handle = window.setTimeout(() => {
      setOffset(0);
      void actions.runLargeFileQuery({
        minBytes,
        extensions: extension.trim() ? [extension.trim().replace(/^\./, '').toLowerCase()] : [],
        modifiedBefore: ageDays.trim() ? Date.now() - Number(ageDays) * 86_400_000 : null,
        modifiedAfter: null,
        sort,
        offset: 0,
        limit: PAGE,
      });
    }, 180);
    return () => window.clearTimeout(handle);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [minBytes, extension, ageDays, sort, hasScan]);

  const items = largeFiles.page?.items ?? [];

  const extensionOptions = useMemo(() => {
    const counts = new Map<string, number>();
    for (const item of items) {
      if (item.extension) counts.set(item.extension, (counts.get(item.extension) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 12);
  }, [items]);

  if (!hasScan) {
    return (
      <div className="flex h-full flex-col" data-testid="large-files-view">
        <Header />
        <EmptyState
          icon={<FileSearch aria-hidden className="size-6" />}
          title="Scan a drive first"
          detail="The large file finder works on scan results, so it can filter by exact size and modification date without walking the disk twice."
          action={
            <Button variant="primary" onClick={() => void actions.scanDrive()} disabled={state.drives.length === 0}>
              Scan the selected drive
            </Button>
          }
        />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="large-files-view">
      <Header />

      {/* ------------------------------ filters ------------------------------ */}
      <div className="flex flex-wrap items-end gap-2 border-b border-line bg-canvas px-3 py-2">
        <div>
          <span className="mb-1 block text-[11px] font-medium text-ink-muted">Minimum size</span>
          <div className="flex flex-wrap items-center gap-1">
            {SIZE_PRESETS.map((preset) => (
              <Button
                key={preset.bytes}
                size="sm"
                variant={minBytes === preset.bytes ? 'primary' : 'secondary'}
                onClick={() => setMinBytes(preset.bytes)}
              >
                {preset.label}
              </Button>
            ))}
          </div>
        </div>

        <TextField
          label="Custom minimum"
          placeholder="e.g. 750 MB"
          value={customSize}
          onChange={(event) => {
            setCustomSize(event.target.value);
            const parsed = parseBytes(event.target.value);
            if (parsed !== null && parsed > 0) setMinBytes(parsed);
          }}
          className="h-7 w-28 text-xs"
        />

        <TextField
          label="Extension"
          placeholder="mp4"
          value={extension}
          onChange={(event) => setExtension(event.target.value)}
          className="h-7 w-24 text-xs"
        />

        <TextField
          label="Not modified in (days)"
          placeholder="180"
          value={ageDays}
          onChange={(event) => setAgeDays(event.target.value.replace(/[^0-9]/g, ''))}
          className="h-7 w-28 text-xs"
        />

        <Select
          label="Sort"
          value={sort}
          onChange={(event) => setSort(event.target.value as LargeFileSort)}
          className="h-7 w-40 text-xs"
        >
          <option value="size-desc">Largest first</option>
          <option value="size-asc">Smallest first</option>
          <option value="modified-desc">Recently modified</option>
          <option value="modified-asc">Oldest first</option>
          <option value="name-asc">Name (A–Z)</option>
          <option value="name-desc">Name (Z–A)</option>
        </Select>
      </div>

      {/* ------------------------------ summary ------------------------------ */}
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-line bg-surface px-3 py-1.5 text-[11px]">
        <span className="dw-tnum text-ink">
          <strong data-testid="large-files-count">{formatCount(largeFiles.page?.total ?? 0)}</strong> files at
          least {formatBytes(minBytes)}
        </span>
        <span className="dw-tnum text-ink-muted">
          totalling {formatBytes(largeFiles.page?.totalBytes ?? 0)}
        </span>
        {extensionOptions.length > 1 ? (
          <span className="flex flex-wrap items-center gap-1.5">
            {extensionOptions.map(([ext, count]) => (
              <button
                key={ext}
                type="button"
                onClick={() => setExtension(extension === ext ? '' : ext)}
                className={cx(
                  'rounded-full border px-1.5 py-px text-[10px] transition-colors',
                  extension === ext
                    ? 'border-accent bg-accent text-accent-ink'
                    : 'border-line bg-surface text-ink-muted hover:text-ink',
                )}
              >
                {ext} · {formatCount(count)}
              </button>
            ))}
          </span>
        ) : null}
      </div>

      {/* ------------------------------- table ------------------------------- */}
      {largeFiles.loading ? (
        <Panel className="m-3 p-4">
          <p className="text-xs text-ink-muted">Filtering scan results…</p>
        </Panel>
      ) : largeFiles.error ? (
        <ErrorState title="Could not read the large file list" detail={largeFiles.error} onRetry={() => void actions.runLargeFileQuery()} />
      ) : items.length === 0 ? (
        <EmptyState
          icon={<Search aria-hidden className="size-6" />}
          title="No files match these filters"
          detail="Try a smaller minimum size, or clear the extension and age filters."
        />
      ) : (
        <VirtualList
          items={items}
          rowHeight={ROW_HEIGHT}
          height={560}
          className="min-h-0 flex-1"
          keyOf={(item) => item.path}
          ariaLabel="Large files"
          onScrollBottom={() => {
            const next = offset + PAGE;
            const total = largeFiles.page?.total ?? 0;
            if (next < total) {
              setOffset(next);
              void actions.runLargeFileQuery({ offset: next });
            }
          }}
          renderRow={(item) => (
            <div
              className={cx(
                'flex h-full items-center gap-2 border-b border-line/60 px-3 transition-colors hover:bg-surface-sunken',
                state.viz.selectedPath === item.path && 'bg-info-soft',
              )}
              onContextMenu={(event) => {
                event.preventDefault();
                itemMenu.open(item.path, event.clientX, event.clientY, 'file');
              }}
            >
              <FileIcon
                name={item.name}
                extension={item.extension}
                kind="file"
                category={item.category}
                className="size-4 shrink-0"
              />
              <div className="min-w-0 flex-1">
                <p className="truncate text-[12px] text-ink">{item.name}</p>
                <p className="truncate font-mono text-[9px] text-ink-faint" title={item.path}>
                  {item.path}
                </p>
              </div>
              <span className="dw-tnum w-20 shrink-0 text-right text-[11px] text-ink-muted">
                {categoryLabel(item.category)}
              </span>
              <span className="dw-tnum w-24 shrink-0 text-right text-[11px] text-ink-muted" title={formatDateTime(item.modifiedAt)}>
                {formatRelativeTime(item.modifiedAt)}
              </span>
              <span className="dw-tnum w-20 shrink-0 text-right text-[13px] font-semibold text-ink">
                {formatBytesCompact(item.sizeBytes)}
              </span>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void actions.reveal(item.path)}
                title="Show this file in File Explorer"
              >
                <FolderOpen aria-hidden className="size-3" />
                Reveal
              </Button>
            </div>
          )}
        />
      )}
    </div>
  );
}

function Header(): React.JSX.Element {
  return (
    <header className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line bg-surface px-3 py-2">
      <div className="min-w-0 flex-1">
        <h1 className="text-[13px] font-semibold text-ink">Large files</h1>
        <p className="mt-0.5 text-[10px] text-ink-faint">
          Biggest files on the scanned volume. Reveal opens the real location in File Explorer.
        </p>
      </div>
    </header>
  );
}
