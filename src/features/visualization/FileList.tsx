/**
 * File list — the third visualization mode.
 *
 * A windowed table: only the visible rows exist in the DOM, so a folder with 500,000 entries
 * scrolls as smoothly as one with 50. Supports sorting, grouping by file type or folder, and
 * free-text search within the current focus.
 */

import { useMemo, useState } from 'react';
import { ArrowDown, ArrowUp, ChevronRight, Search } from 'lucide-react';
import type { TreeNode } from '@/types/contracts';
import { VirtualList } from '@/components/ui/VirtualList';
import { EmptyState, Select, TextField, cx } from '@/components/ui/primitives';
import { FileIcon } from '@/lib/icons';
import { formatBytes, formatCount, formatDateTime } from '@/lib/format';
import { sortNodes } from './layout';
import type { GroupBy, SortKey } from '@/app/state';

const ROW_HEIGHT = 26;

export interface FileListProps {
  root: TreeNode;
  selectedPath: string | null;
  sort: SortKey;
  sortDir: 'asc' | 'desc';
  groupBy: GroupBy;
  search: string;
  onSelect: (path: string | null) => void;
  onOpen: (path: string) => void;
  onSortChange: (sort: SortKey, dir: 'asc' | 'desc') => void;
  onGroupChange: (groupBy: GroupBy) => void;
  onSearchChange: (search: string) => void;
  onContextMenu: (path: string, x: number, y: number) => void;
  className?: string;
}

interface Row {
  key: string;
  kind: 'group' | 'item';
  label: string;
  bytes?: number;
  count?: number;
  node?: TreeNode;
}

export function FileList({
  root,
  selectedPath,
  sort,
  sortDir,
  groupBy,
  search,
  onSelect,
  onOpen,
  onSortChange,
  onGroupChange,
  onSearchChange,
  onContextMenu,
  className,
}: FileListProps): React.JSX.Element {
  const [visibleCount, setVisibleCount] = useState(200);

  const rows = useMemo<Row[]>(() => {
    const needle = search.trim().toLowerCase();
    const filtered = needle
      ? root.children.filter((child) => child.name.toLowerCase().includes(needle))
      : root.children;
    const sorted = sortNodes(filtered, sort, sortDir);

    if (groupBy === 'none') {
      return sorted.map((node) => ({ key: node.path, kind: 'item', label: node.name, node }));
    }

    const keyOf = (node: TreeNode): string =>
      groupBy === 'type' ? node.extension ?? 'no extension' : node.kind === 'folder' ? 'Folders' : 'Files';

    const groups = new Map<string, TreeNode[]>();
    for (const node of sorted) {
      const key = keyOf(node);
      const bucket = groups.get(key);
      if (bucket) bucket.push(node);
      else groups.set(key, [node]);
    }

    const out: Row[] = [];
    const groupBytes = (nodes: TreeNode[]): number => nodes.reduce((sum, node) => sum + node.sizeBytes, 0);
    const entries: Array<[string, TreeNode[]]> = [...groups.entries()];
    entries.sort((a, b) => groupBytes(b[1]) - groupBytes(a[1]));
    for (const [key, nodes] of entries) {
      out.push({
        key: `group:${key}`,
        kind: 'group',
        label: key,
        bytes: nodes.reduce((sum, node) => sum + node.sizeBytes, 0),
        count: nodes.length,
      });
      for (const node of nodes) out.push({ key: node.path, kind: 'item', label: node.name, node });
    }
    return out;
  }, [root.children, search, sort, sortDir, groupBy]);

  const shown = useMemo(() => rows.slice(0, visibleCount), [rows, visibleCount]);
  const totalBytes = useMemo(
    () => rows.reduce((sum, row) => sum + (row.kind === 'item' ? row.node?.sizeBytes ?? 0 : 0), 0),
    [rows],
  );

  return (
    <div className={cx('flex min-h-0 flex-col', className)} data-testid="file-list">
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-2 py-1.5">
        <span className="relative">
          <Search aria-hidden className="pointer-events-none absolute left-2 top-1/2 size-3 -translate-y-1/2 text-ink-faint" />
          <TextField
            value={search}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder={`Search in ${root.name}`}
            aria-label="Search within results"
            className="h-7 w-52 pl-7 text-xs"
          />
        </span>
        <Select
          value={groupBy}
          onChange={(event) => onGroupChange(event.target.value as GroupBy)}
          aria-label="Group by"
          className="h-7 w-32 text-xs"
        >
          <option value="none">No grouping</option>
          <option value="type">Group by type</option>
          <option value="folder">Group by kind</option>
        </Select>
        <div className="ml-auto dw-tnum flex items-center gap-2 text-[10px] text-ink-faint">
          <span>{formatCount(rows.filter((row) => row.kind === 'item').length)} items</span>
          <span>{formatBytes(totalBytes)}</span>
        </div>
      </div>

      <div className="grid grid-cols-[1fr_88px_72px_140px_24px] items-center gap-2 border-b border-line bg-surface-sunken px-2 py-1 text-[10px] font-semibold uppercase tracking-wide text-ink-faint">
        <SortHeader label="Name" active={sort === 'name'} dir={sortDir} onClick={() => onSortChange('name', sortDir)} />
        <SortHeader label="Size" active={sort === 'size'} dir={sortDir} onClick={() => onSortChange('size', sortDir)} align="right" />
        <SortHeader
          label="Files"
          active={sort === 'count'}
          dir={sortDir}
          onClick={() => onSortChange('count', sortDir)}
          align="right"
        />
        <SortHeader
          label="Modified"
          active={sort === 'modified'}
          dir={sortDir}
          onClick={() => onSortChange('modified', sortDir)}
        />
        <span />
      </div>

      <VirtualList
        items={shown}
        rowHeight={ROW_HEIGHT}
        height={420}
        className="min-h-0 flex-1"
        keyOf={(row) => row.key}
        ariaLabel="Scan results"
        onScrollBottom={() => setVisibleCount((count) => Math.min(rows.length, count + 400))}
        empty={
          <EmptyState
            title={search ? 'No matches' : 'This folder is empty'}
            detail={search ? `Nothing in ${root.name} matches “${search}”.` : undefined}
          />
        }
        renderRow={(row) =>
          row.kind === 'group' ? (
            <div className="flex h-full items-center gap-2 bg-surface-sunken/70 px-2 text-[10px] font-semibold uppercase tracking-wide text-ink-faint">
              <span>{row.label}</span>
              <span className="dw-tnum font-normal normal-case tracking-normal">
                {formatCount(row.count ?? 0)} · {formatBytes(row.bytes ?? 0)}
              </span>
            </div>
          ) : (
            <button
              type="button"
              onClick={() => onSelect(row.node!.path)}
              onDoubleClick={() => onOpen(row.node!.path)}
              onContextMenu={(event) => {
                event.preventDefault();
                onContextMenu(row.node!.path, event.clientX, event.clientY);
              }}
              className={cx(
                'flex h-full w-full items-center gap-2 px-2 text-left transition-colors',
                selectedPath === row.node!.path ? 'bg-info-soft' : 'hover:bg-surface-sunken',
              )}
              data-testid="file-row"
            >
              <FileIcon
                name={row.node!.name}
                extension={row.node!.extension}
                kind={row.node!.kind}
                category={row.node!.category}
                className="size-3.5 shrink-0"
              />
              <span className="min-w-0 flex-1 truncate text-[12px] text-ink">{row.node!.name}</span>
              <span className="dw-tnum w-[88px] shrink-0 text-right text-[11px] text-ink">
                {formatBytes(row.node!.sizeBytes)}
              </span>
              <span className="dw-tnum w-[72px] shrink-0 text-right text-[11px] text-ink-muted">
                {row.node!.kind === 'folder' ? formatCount(row.node!.fileCount) : '—'}
              </span>
              <span className="dw-tnum w-[140px] shrink-0 truncate text-[11px] text-ink-muted">
                {formatDateTime(row.node!.modifiedAt)}
              </span>
              {row.node!.kind === 'folder' ? (
                <ChevronRight aria-hidden className="size-3 shrink-0 text-ink-faint" />
              ) : (
                <span />
              )}
            </button>
          )
        }
      />
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
