/**
 * Right details panel (~280px).
 *
 * Shows everything Diskora knows about the current selection and nothing more — there is no
 * "delete" button anywhere in this panel. Destructive actions live in the cleanup assistant,
 * behind an explicit confirmation that lists the exact paths.
 */

import { useMemo } from 'react';
import { Crosshair, Copy, ExternalLink, FolderOpen, Info, ShieldCheck, Trash2 } from 'lucide-react';
import { Badge, Button, EmptyState, Panel, SectionTitle, Skeleton, cx } from '@/components/ui/primitives';
import { FileIcon } from '@/lib/icons';
import { categoryColor, categoryLabel } from '@/lib/categories';
import {
  formatBytes,
  formatBytesExact,
  formatCount,
  formatDateTime,
  formatRelativeTime,
} from '@/lib/format';
import { baseName, parentPath } from '@/lib/path';
import { findNode, largestChildren } from '@/features/visualization/layout';
import { isSyntheticPath } from '@/lib/tree';
import { useApp } from './state';

export function DetailsPanel(): React.JSX.Element {
  const { state, actions } = useApp();
  const { details, scan, viz } = state;
  const node = useMemo(
    () => (scan.result && viz.selectedPath ? findNode(scan.result.tree, viz.selectedPath) : null),
    [scan.result, viz.selectedPath],
  );

  const data = details.data;
  const selected = viz.selectedPath !== null;
  const synthetic = viz.selectedPath ? isSyntheticPath(viz.selectedPath) : false;

  return (
    <aside
      className="flex w-[280px] shrink-0 flex-col gap-2 overflow-y-auto border-l border-line bg-canvas p-2"
      aria-label="Item details"
      data-testid="details-panel"
    >
      {!selected ? (
        <Panel className="p-3">
          <EmptyState
            icon={<Info aria-hidden className="size-5" />}
            title="Nothing selected"
            detail="Click a ring segment, a treemap block or a list row to inspect it here."
            className="py-6"
          />
          {scan.result ? (
            <div className="mt-1 border-t border-line pt-2">
              <SectionTitle className="px-0 pt-0 pb-1">Scan summary</SectionTitle>
              <dl className="dw-tnum space-y-0.5 text-[11px]">
                <Stat label="Analyzed" value={formatBytes(scan.result.totalBytes)} />
                <Stat label="Files" value={formatCount(scan.result.fileCount)} />
                <Stat label="Folders" value={formatCount(scan.result.folderCount)} />
                <Stat label="Duration" value={`${Math.round((scan.result.durationMs ?? 0) / 1000)}s`} />
                <Stat label="Status" value={statusLabel(scan.result.status)} />
                {scan.result.errors.length > 0 ? (
                  <Stat label="Unreadable" value={formatCount(scan.result.errors.length)} />
                ) : null}
              </dl>
            </div>
          ) : null}
        </Panel>
      ) : (
        <>
          {/* ---------------- header ---------------- */}
          <Panel className="p-2.5">
            <div className="flex items-start gap-2">
              <FileIcon
                name={node?.name ?? baseName(viz.selectedPath ?? '')}
                extension={node?.extension ?? null}
                kind={node?.kind ?? 'file'}
                category={node?.category ?? 'other'}
                isRoot={node?.path === scan.result?.rootPath}
                className="mt-0.5 size-6 shrink-0"
              />
              <div className="min-w-0 flex-1">
                <h2 className="break-words text-[13px] font-semibold leading-snug text-ink" data-testid="details-name">
                  {node?.name ?? baseName(viz.selectedPath ?? '')}
                </h2>
                <p className="mt-0.5 flex flex-wrap items-center gap-1">
                  <Badge tone="neutral">{node?.kind === 'folder' ? 'Folder' : 'File'}</Badge>
                  {node ? (
                    <Badge tone="neutral" dot={categoryColor(node.category)}>
                      {categoryLabel(node.category)}
                    </Badge>
                  ) : null}
                  {synthetic ? <Badge tone="warn">Aggregate</Badge> : null}
                </p>
              </div>
            </div>

            <p className="mt-2 break-all rounded-control bg-surface-sunken px-2 py-1.5 font-mono text-[10px] leading-relaxed text-ink-muted">
              {viz.selectedPath}
            </p>

            <div className="mt-2 flex flex-wrap gap-1">
              <Button
                size="sm"
                onClick={() => void actions.reveal(viz.selectedPath!)}
                disabled={synthetic}
                title="Open File Explorer with this item selected"
              >
                <FolderOpen aria-hidden className="size-3" />
                Reveal
              </Button>
              <Button size="sm" onClick={() => void actions.copyPath(viz.selectedPath!)}>
                <Copy aria-hidden className="size-3" />
                Copy path
              </Button>
              {node?.kind === 'file' ? (
                <Button size="sm" onClick={() => void actions.open(viz.selectedPath!)} disabled={synthetic}>
                  <ExternalLink aria-hidden className="size-3" />
                  Open
                </Button>
              ) : null}
              {node && node.kind === 'folder' ? (
                <Button size="sm" onClick={() => actions.focusPath(node.path)}>
                  <Crosshair aria-hidden className="size-3" />
                  Focus
                </Button>
              ) : null}
            </div>
          </Panel>

          {/* ---------------- sizes ---------------- */}
          <Panel className="p-2.5">
            <SectionTitle className="px-0 pt-0 pb-1">Size</SectionTitle>
            {details.loading ? (
              <div className="space-y-1.5">
                <Skeleton className="h-4 w-24" />
                <Skeleton className="h-3 w-full" />
                <Skeleton className="h-3 w-3/4" />
              </div>
            ) : details.error ? (
              <p className="text-[11px] leading-relaxed text-ink-muted">{details.error}</p>
            ) : (
              <dl className="dw-tnum space-y-1 text-[11px]">
                <div>
                  <dt className="text-ink-faint">Logical size</dt>
                  <dd className="text-[15px] font-semibold text-ink" data-testid="details-size">
                    {formatBytes(node?.sizeBytes ?? 0)}
                  </dd>
                </div>
                {node?.diskSizeBytes !== null && node?.diskSizeBytes !== undefined ? (
                  <Stat label="Size on disk" value={formatBytes(node.diskSizeBytes)} />
                ) : null}
                <Stat label="Exact bytes" value={formatBytesExact(node?.sizeBytes ?? 0)} />
                {node && node.sizeBytes > 0 ? (
                  <Stat
                    label="Share of scan"
                    value={`${((node.sizeBytes / Math.max(1, scan.result?.totalBytes ?? 1)) * 100).toFixed(2)}%`}
                  />
                ) : null}
                {node ? (
                  <>
                    <Stat label="Files inside" value={formatCount(node.fileCount)} />
                    <Stat label="Folders inside" value={formatCount(node.folderCount)} />
                  </>
                ) : null}
              </dl>
            )}
          </Panel>

          {/* ---------------- metadata ---------------- */}
          <Panel className="p-2.5">
            <SectionTitle className="px-0 pt-0 pb-1">Details</SectionTitle>
            <dl className="space-y-1 text-[11px]">
              <Meta label="Parent folder" value={parentPath(viz.selectedPath ?? '') ?? 'Volume root'} mono />
              <Meta label="Extension" value={node?.extension ? `.${node.extension}` : '—'} />
              <Meta label="Modified" value={formatDateTime(node?.modifiedAt ?? null)} />
              <Meta label="Modified" hint={formatRelativeTime(node?.modifiedAt ?? null)} value="" hideLabel />
              <Meta label="Created" value={formatDateTime(node?.createdAt ?? null)} />
              {data?.attributes && data.attributes.length > 0 ? (
                <Meta label="Attributes" value={data.attributes.join(', ')} />
              ) : null}
              {node?.accessDenied ? (
                <Meta label="Access" value="Denied — contents were not read" />
              ) : null}
            </dl>
          </Panel>

          {/* ---------------- largest children ---------------- */}
          {node && node.children.length > 0 ? (
            <Panel className="p-2.5">
              <SectionTitle className="px-0 pt-0 pb-1">Largest inside</SectionTitle>
              <ul className="flex flex-col gap-0.5">
                {largestChildren(node, 6).map((child) => (
                  <li key={child.path}>
                    <button
                      type="button"
                      onClick={() => actions.selectNode(child.path)}
                      className="flex w-full items-center gap-1.5 rounded px-1 py-1 text-left transition-colors hover:bg-surface-sunken"
                    >
                      <FileIcon
                        name={child.name}
                        extension={child.extension}
                        kind={child.kind}
                        category={child.category}
                        className="size-3 shrink-0"
                      />
                      <span className="min-w-0 flex-1 truncate text-[11px] text-ink">{child.name}</span>
                      <span className="dw-tnum shrink-0 text-[10px] text-ink-muted">
                        {formatBytes(child.sizeBytes)}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </Panel>
          ) : null}

          {/* ---------------- cleanup review ---------------- */}
          {!synthetic ? (
            <Panel className="p-2.5">
              <SectionTitle className="px-0 pt-0 pb-1">Cleanup review</SectionTitle>
              <p className="mb-2 text-[11px] leading-relaxed text-ink-muted">
                Add this item to the cleanup assistant to review it with everything else. Nothing is
                removed until you confirm the exact paths.
              </p>
              <Button
                size="sm"
                className="w-full justify-center"
                onClick={() => {
                  actions.navigate('cleanup');
                  actions.toast({
                    kind: 'info',
                    title: 'Added to review',
                    detail: viz.selectedPath ?? undefined,
                    timeoutMs: 3500,
                  });
                }}
              >
                <Trash2 aria-hidden className="size-3" />
                Add to cleanup review
              </Button>
            </Panel>
          ) : null}

          <Panel className="flex items-start gap-2 p-2.5">
            <ShieldCheck aria-hidden className="mt-px size-3.5 shrink-0 text-ok" />
            <p className="text-[10px] leading-relaxed text-ink-muted">
              Diskora never deletes anything because you selected it. Every removal goes through the
              cleanup assistant with an explicit confirmation and prefers the Recycle Bin.
            </p>
          </Panel>
        </>
      )}
    </aside>
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

function Meta({
  label,
  value,
  hint,
  mono,
  hideLabel,
}: {
  label: string;
  value: string;
  hint?: string;
  mono?: boolean;
  hideLabel?: boolean;
}) {
  if (hideLabel && hint) {
    return (
      <div className="flex items-baseline justify-between gap-2">
        <dt className="sr-only">{label}</dt>
        <dd className="text-[10px] italic text-ink-faint">{hint}</dd>
      </div>
    );
  }
  return (
    <div className="flex items-baseline justify-between gap-2">
      <dt className="shrink-0 text-ink-faint">{label}</dt>
      <dd className={cx('min-w-0 truncate text-right text-ink', mono && 'font-mono text-[10px]')} title={value}>
        {value}
      </dd>
    </div>
  );
}

function statusLabel(status: string): string {
  switch (status) {
    case 'complete':
      return 'Complete';
    case 'partial':
      return 'Partial';
    case 'cancelled':
      return 'Cancelled';
    case 'error':
      return 'Failed';
    default:
      return status;
  }
}
