/**
 * Paged, windowed file list — the third visualization mode.
 *
 * The Rust index owns the full set. The webview requests bounded pages (500 records per IPC call)
 * and mounts only visible rows, so a large folder is never serialized or rendered all at once.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowDown, ArrowUp, ChevronRight, Search } from 'lucide-react';
import type { TreeNode } from '@/types/contracts';
import { VirtualList } from '@/components/ui/VirtualList';
import { EmptyState, TextField, cx } from '@/components/ui/primitives';
import { FileIcon } from '@/lib/icons';
import { formatBytes, formatCount, formatDateTime } from '@/lib/format';
import { getTransport } from '@/lib/transport';
import { baseName, parentPath } from '@/lib/path';
import type { SortKey } from '@/app/state';

const ROW_HEIGHT = 26;
const PAGE_SIZE = 500;

export interface FileListProps {
  scanId: string;
  root: TreeNode;
  rootPath: string;
  scanRootPath: string;
  selectedPath: string | null;
  sort: SortKey;
  sortDir: 'asc' | 'desc';
  search: string;
  onSelect: (path: string | null) => void;
  onOpen: (path: string) => void;
  onSortChange: (sort: SortKey, dir: 'asc' | 'desc') => void;
  onSearchChange: (search: string) => void;
  onContextMenu: (path: string, x: number, y: number, kind: 'file' | 'folder') => void;
  className?: string;
}

export function FileList({
  scanId,
  root,
  rootPath,
  scanRootPath,
  selectedPath,
  sort,
  sortDir,
  search,
  onSelect,
  onOpen,
  onSortChange,
  onSearchChange,
  onContextMenu,
  className,
}: FileListProps): React.JSX.Element {
  const [items, setItems] = useState<TreeNode[]>([]);
  const [total, setTotal] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  const loadingRef = useRef(false);
  const nextOffset = useRef(0);
  const totalRef = useRef<number | null>(null);
  const queryRef = useRef({ scanId, path: rootPath, sort, sortDir, search });
  const queryKey = `${scanId}\u0000${rootPath}\u0000${sort}\u0000${sortDir}\u0000${search.trim()}`;
  queryRef.current = { scanId, path: rootPath, sort, sortDir, search: search.trim() };
  const rootName = rootPath === root.path ? root.name : baseName(rootPath) || root.name;
  const upPath = parentPath(rootPath);
  const canGoUp = rootPath.toLowerCase() !== scanRootPath.toLowerCase() && upPath !== null;
  totalRef.current = total;

  useEffect(() => {
    const requestGeneration = ++generation.current;
    let active = true;
    const request = queryRef.current;
    nextOffset.current = 0;
    totalRef.current = null;
    loadingRef.current = true;
    setItems([]);
    setTotal(null);
    setError(null);
    setLoading(true);

    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const transport = await getTransport();
          const page = await transport.scanChildren({ ...request, offset: 0, limit: PAGE_SIZE });
          if (!active || generation.current !== requestGeneration) return;
          setItems(page.items);
          nextOffset.current = page.items.length;
          totalRef.current = page.total;
          setTotal(page.total);
        } catch (cause) {
          if (active && generation.current === requestGeneration) {
            setError(cause instanceof Error ? cause.message : String(cause));
          }
        } finally {
          if (active && generation.current === requestGeneration) {
            loadingRef.current = false;
            setLoading(false);
          }
        }
      })();
    }, request.search ? 160 : 0);

    return () => {
      active = false;
      window.clearTimeout(timer);
      if (generation.current === requestGeneration) {
        generation.current += 1;
        loadingRef.current = false;
      }
    };
  }, [queryKey]);

  const loadMore = useCallback(async () => {
    const expectedTotal = totalRef.current;
    const offset = nextOffset.current;
    if (loadingRef.current || expectedTotal === null || offset >= expectedTotal) return;

    loadingRef.current = true;
    setLoading(true);
    const requestGeneration = generation.current;
    try {
      const transport = await getTransport();
      const page = await transport.scanChildren({
        ...queryRef.current,
        offset,
        limit: PAGE_SIZE,
      });
      if (generation.current !== requestGeneration) return;
      setItems((current) => [...current, ...page.items]);
      nextOffset.current = offset + page.items.length;
      totalRef.current = page.total;
      setTotal(page.total);
      setError(null);
    } catch (cause) {
      if (generation.current === requestGeneration) {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    } finally {
      if (generation.current === requestGeneration) {
        loadingRef.current = false;
        setLoading(false);
      }
    }
  }, []);

  const loadedBytes = useMemo(() => items.reduce((sum, item) => sum + item.sizeBytes, 0), [items]);
  const hasMore = total !== null && items.length < total;

  return (
    <div className={cx('flex min-h-0 flex-col', className)} data-testid="file-list">
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-2 py-1.5">
        {canGoUp && upPath ? (
          <button
            type="button"
            onClick={() => onOpen(upPath)}
            className="rounded border border-line px-2 py-1 text-[10px] text-ink-muted hover:bg-surface-sunken hover:text-ink"
            aria-label="Open parent folder"
          >
            Up
          </button>
        ) : null}
        <span className="relative">
          <Search aria-hidden className="pointer-events-none absolute left-2 top-1/2 size-3 -translate-y-1/2 text-ink-faint" />
          <TextField
            value={search}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder={`Search in ${rootName}`}
            aria-label="Search within results"
            className="h-7 w-52 pl-7 text-xs"
          />
        </span>
        <div className="ml-auto dw-tnum flex items-center gap-2 text-[10px] text-ink-faint">
          <span>
            {formatCount(items.length)} of {total === null ? '…' : formatCount(total)} items
          </span>
          <span>{formatBytes(loadedBytes)} loaded</span>
        </div>
      </div>

      <div className="grid grid-cols-[1fr_88px_72px_140px_24px] items-center gap-2 border-b border-line bg-surface-sunken px-2 py-1 text-[10px] font-semibold uppercase tracking-wide text-ink-faint">
        <SortHeader label="Name" active={sort === 'name'} dir={sortDir} onClick={() => onSortChange('name', sort === 'name' && sortDir === 'desc' ? 'asc' : 'desc')} />
        <SortHeader label="Size" active={sort === 'size'} dir={sortDir} onClick={() => onSortChange('size', sort === 'size' && sortDir === 'desc' ? 'asc' : 'desc')} align="right" />
        <SortHeader
          label="Files"
          active={sort === 'count'}
          dir={sortDir}
          onClick={() => onSortChange('count', sort === 'count' && sortDir === 'desc' ? 'asc' : 'desc')}
          align="right"
        />
        <SortHeader
          label="Modified"
          active={sort === 'modified'}
          dir={sortDir}
          onClick={() => onSortChange('modified', sort === 'modified' && sortDir === 'desc' ? 'asc' : 'desc')}
        />
        <span />
      </div>

      {error ? (
        <p role="alert" className="border-b border-line bg-danger-soft px-2 py-1 text-[11px] text-danger">
          Could not load this page: {error}
        </p>
      ) : null}
      <VirtualList
        items={items}
        rowHeight={ROW_HEIGHT}
        height={420}
        className="min-h-0 flex-1"
        keyOf={(node) => node.path}
        resetKey={queryKey}
        ariaLabel="Scan results"
        onScrollBottom={() => void loadMore()}
        empty={
          <EmptyState
            title={loading ? 'Loading entries…' : search ? 'No matches' : 'This folder is empty'}
            detail={error ?? (search ? `Nothing in ${rootName} matches “${search}”.` : undefined)}
          />
        }
        renderRow={(node) => (
          <button
            type="button"
            onClick={() => onSelect(node.path)}
            onDoubleClick={() => {
              if (node.kind === 'folder' && !node.isSymlink && !node.isJunction) onOpen(node.path);
            }}
            onContextMenu={(event) => {
              event.preventDefault();
              onContextMenu(node.path, event.clientX, event.clientY, node.kind);
            }}
            className={cx(
              'flex h-full w-full items-center gap-2 px-2 text-left transition-colors',
              selectedPath === node.path ? 'bg-info-soft' : 'hover:bg-surface-sunken',
            )}
            data-testid="file-row"
          >
            <FileIcon
              name={node.name}
              extension={node.extension}
              kind={node.kind}
              category={node.category}
              className="size-3.5 shrink-0"
            />
            <span className="min-w-0 flex-1 truncate text-[12px] text-ink">{node.name}</span>
            <span className="dw-tnum w-[88px] shrink-0 text-right text-[11px] text-ink">
              {formatBytes(node.sizeBytes)}
            </span>
            <span className="dw-tnum w-[72px] shrink-0 text-right text-[11px] text-ink-muted">
              {node.kind === 'folder' ? formatCount(node.fileCount) : '—'}
            </span>
            <span className="dw-tnum w-[140px] shrink-0 truncate text-[11px] text-ink-muted">
              {formatDateTime(node.modifiedAt)}
            </span>
            {node.kind === 'folder' && !node.isSymlink && !node.isJunction ? (
              <ChevronRight aria-hidden className="size-3 shrink-0 text-ink-faint" />
            ) : (
              <span />
            )}
          </button>
        )}
      />
      {loading && items.length > 0 ? (
        <p className="border-t border-line px-2 py-1 text-[10px] text-ink-faint" role="status">
          Loading more entries…
        </p>
      ) : null}
      {!loading && hasMore && items.length > 0 ? (
        <button
          type="button"
          className="border-t border-line px-2 py-1 text-[10px] text-ink-faint hover:text-ink"
          onClick={() => void loadMore()}
        >
          Load next page
        </button>
      ) : null}
    </div>
  );
}

function SortHeader({
  label,
  active,
  dir,
  onClick,
  align = 'left',
}: {
  label: string;
  active: boolean;
  dir: 'asc' | 'desc';
  onClick: () => void;
  align?: 'left' | 'right';
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cx(
        'flex items-center gap-0.5 rounded transition-colors hover:text-ink',
        align === 'right' && 'justify-end',
        active && 'text-ink',
      )}
      aria-sort={active ? (dir === 'asc' ? 'ascending' : 'descending') : 'none'}
    >
      {label}
      {active ? dir === 'asc' ? (
        <ArrowUp aria-hidden className="size-2.5" />
      ) : (
        <ArrowDown aria-hidden className="size-2.5" />
      ) : null}
    </button>
  );
}
