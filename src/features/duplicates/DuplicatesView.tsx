/**
 * Duplicate finder.
 *
 * Staged detection, exactly as specified:
 *   1. group candidate files by exact size (cheap, no I/O),
 *   2. hash the survivors (SHA-256 by default),
 *   3. optionally verify byte-for-byte before anything is offered for removal.
 *
 * Filenames are never used to decide that two files are duplicates. Nothing is preselected for
 * removal: the user picks which copies to review, and at least one copy of every group is always
 * retained.
 */

import { useState } from 'react';
import { Copy, FolderOpen, ShieldCheck, ShieldOff } from 'lucide-react';
import { useApp } from '@/app/state';
import { useItemMenu } from '@/app/itemMenu';
import {
  Badge,
  Button,
  EmptyState,
  ErrorState,
  Panel,
  ProgressBar,
  SectionTitle,
  cx,
} from '@/components/ui/primitives';
import { formatBytes, formatCount, formatDateTime, formatRelativeTime } from '@/lib/format';
import type { DuplicateGroup } from '@/types/contracts';

const MB = 1024 * 1024;

export function DuplicatesView(): React.JSX.Element {
  const { state, actions } = useApp();
  const itemMenu = useItemMenu();
  const { duplicates, scan } = state;

  const [minBytes, setMinBytes] = useState(1 * MB);
  const [expanded, setExpanded] = useState<string | null>(null);

  const hasScan = scan.result !== null;
  const report = duplicates.report;
  if (!hasScan) {
    return (
      <div className="flex h-full flex-col" data-testid="duplicates-view">
        <Header />
        <EmptyState
          icon={<Copy aria-hidden className="size-6" />}
          title="Scan a drive first"
          detail="Duplicate detection runs on scan results: files are grouped by size, then hashed, then optionally verified byte-for-byte."
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
    <div className="flex h-full min-h-0 flex-col" data-testid="duplicates-view">
      <Header />

      {/* ------------------------------ controls ------------------------------ */}
      <div className="flex flex-wrap items-end gap-2 border-b border-line bg-canvas px-3 py-2">
        <label className="block">
          <span className="mb-1 block text-[11px] font-medium text-ink-muted">Ignore files smaller than</span>
          <select
            value={minBytes}
            onChange={(event) => setMinBytes(Number(event.target.value))}
            className="h-7 w-36 rounded-control border border-line bg-surface px-2 text-xs text-ink focus:border-info focus:outline-none"
          >
            <option value={1024}>1 KB</option>
            <option value={MB}>1 MB</option>
            <option value={10 * MB}>10 MB</option>
            <option value={100 * MB}>100 MB</option>
            <option value={1024 * MB}>1 GB</option>
          </select>
        </label>
        <Button variant="primary" onClick={() => void actions.runDuplicateScan({ minBytes })} loading={duplicates.loading}>
          <Copy aria-hidden className="size-3.5" />
          {report ? 'Scan again' : 'Find duplicates'}
        </Button>

      </div>

      {/* ------------------------------ progress ------------------------------ */}
      {duplicates.loading && duplicates.progress ? (
        <div className="border-b border-line bg-surface px-3 py-2">
          <ProgressBar
            value={
              duplicates.progress.etaMs && duplicates.progress.elapsedMs > 0
                ? Math.min(1, duplicates.progress.elapsedMs / (duplicates.progress.elapsedMs + duplicates.progress.etaMs))
                : 0
            }
            label="Duplicate scan progress"
          />
          <p className="dw-tnum mt-1 text-[10px] text-ink-muted">
            {duplicates.progress.phase} · {formatCount(duplicates.progress.filesHashed)} files hashed ·{' '}
            {formatBytes(duplicates.progress.bytesHashed)} read
          </p>
        </div>
      ) : null}

      {duplicates.error ? (
        <ErrorState title="Duplicate detection failed" detail={duplicates.error} onRetry={() => void actions.runDuplicateScan({ minBytes })} />
      ) : null}

      {/* ------------------------------- results ------------------------------- */}
      {!report ? (
        <EmptyState
          icon={<ShieldCheck aria-hidden className="size-6" />}
          title="No duplicate scan yet"
          detail="Diskora groups by size first, hashes the survivors with SHA-256, and can verify byte-for-byte. Filenames are never used as evidence."
        />
      ) : report.totalGroups === 0 ? (
        <EmptyState
          icon={<ShieldCheck aria-hidden className="size-6" />}
          title="No duplicates found"
          detail={`${formatCount(report.filesHashed)} files were hashed (${formatBytes(report.hashedBytes)} read) and no two of them matched.`}
        />
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2">
          <Panel className="mb-2 flex flex-wrap items-center gap-x-4 gap-y-1 p-2.5">
            <Stat label="Groups" value={formatCount(report.totalGroups)} />
            <Stat label="Duplicate copies" value={formatCount(report.totalCopies)} />
            <Stat label="Reclaimable" value={formatBytes(report.totalReclaimableBytes)} strong />
            <Stat label="Algorithm" value={report.algorithm.toUpperCase()} />
            <Stat label="Hashed" value={`${formatCount(report.filesHashed)} files · ${formatBytes(report.hashedBytes)}`} />
            {report.errors.length > 0 ? <Stat label="Unreadable" value={formatCount(report.errors.length)} /> : null}
          </Panel>

          <ul className="flex flex-col gap-2">
            {report.groups.map((group) => (
              <GroupCard
                key={group.groupId}
                group={group}
                expanded={expanded === group.groupId}
                onToggle={() => setExpanded(expanded === group.groupId ? null : group.groupId)}
                onReveal={(path) => void actions.reveal(path)}
                onExclude={(path) => actions.excludeDuplicatePath(path)}
                onContextMenu={(path, x, y) => itemMenu.open(path, x, y, 'file')}
              />
            ))}
          </ul>
        </div>
      )}

      {/* ---------------------------- exclusions ---------------------------- */}
      {state.duplicates.excludePaths.length > 0 ? (
        <div className="border-t border-line bg-surface px-3 py-2">
          <SectionTitle className="px-0 pt-0 pb-1">Excluded paths</SectionTitle>
          <div className="flex flex-wrap gap-1">
            {state.duplicates.excludePaths.map((path) => (
              <span
                key={path}
                className="flex items-center gap-1 rounded-full border border-line bg-surface-sunken px-2 py-0.5 text-[10px] text-ink-muted"
              >
                <ShieldOff aria-hidden className="size-2.5" />
                <span className="max-w-64 truncate" title={path}>
                  {path}
                </span>
                <button type="button" className="underline hover:text-ink" onClick={() => actions.removeDuplicateExclusion(path)}>
                  remove
                </button>
              </span>
            ))}
          </div>
        </div>
      ) : null}


    </div>
  );
}

function GroupCard({
  group,
  expanded,
  onToggle,
  onReveal,
  onExclude,
  onContextMenu,
}: {
  group: DuplicateGroup;
  expanded: boolean;
  onToggle: () => void;
  onReveal: (path: string) => void;
  onExclude: (path: string) => void;
  onContextMenu: (path: string, x: number, y: number) => void;
}): React.JSX.Element {

  return (
    <li>
      <Panel className="overflow-hidden">
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={expanded}
          className="flex w-full items-center gap-2 px-2.5 py-2 text-left transition-colors hover:bg-surface-sunken"
        >
          <Badge tone={group.confidence === 'verified' ? 'ok' : 'neutral'}>
            {group.confidence === 'verified' ? 'byte-verified' : 'hash match'}
          </Badge>
          <span className="dw-tnum text-[12px] font-semibold text-ink">
            {group.copies.length} copies · {formatBytes(group.sizeBytes)} each
          </span>
          <span className="dw-tnum ml-auto text-[12px] font-medium text-ink">
            {formatBytes(group.reclaimableBytes)} reclaimable
          </span>
          <span className="text-[10px] text-ink-faint">{expanded ? 'Hide' : 'Details'}</span>
        </button>

        {expanded ? (
          <div className="border-t border-line">
            <table className="w-full text-left">
              <thead>
                <tr className="bg-surface-sunken text-[10px] uppercase tracking-wide text-ink-faint">
                  <th className="px-2.5 py-1 font-semibold">Path</th>
                  <th className="px-2.5 py-1 font-semibold">Modified</th>
                  <th className="px-2.5 py-1 font-semibold" />
                </tr>
              </thead>
              <tbody>
                {group.copies.map((copy) => {
                  return (
                    <tr
                      key={copy.path}
                      className="border-t border-line/60"
                      onContextMenu={(event) => {
                        event.preventDefault();
                        onContextMenu(copy.path, event.clientX, event.clientY);
                      }}
                    >
                      <td className="px-2.5 py-1.5">
                        <p className="truncate font-mono text-[11px] text-ink" title={copy.path}>
                          {copy.path}
                        </p>
                        <p className="text-[9px] text-ink-faint">
                          {copy.name}
                          {copy.isSuggestedOriginal ? ' · suggested original' : ''}
                          {copy.byteVerified ? ' · verified identical' : ''}
                        </p>
                      </td>
                      <td
                        className="dw-tnum px-2.5 py-1.5 text-[11px] text-ink-muted"
                        title={formatDateTime(copy.modifiedAt)}
                      >
                        {formatRelativeTime(copy.modifiedAt)}
                      </td>
                      <td className="px-2.5 py-1.5">
                        <div className="flex items-center gap-1">
                          <Button size="sm" variant="ghost" onClick={() => onReveal(copy.path)} title="Reveal in File Explorer">
                            <FolderOpen aria-hidden className="size-3" />
                          </Button>
                          <Button size="sm" variant="ghost" onClick={() => onExclude(copy.path)} title="Exclude this file">
                            <ShieldOff aria-hidden className="size-3" />
                          </Button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>

          </div>
        ) : null}
      </Panel>
    </li>
  );
}

function Stat({ label, value, strong }: { label: string; value: string; strong?: boolean }) {
  return (
    <div>
      <dt className="text-[10px] text-ink-faint">{label}</dt>
      <dd className={cx('dw-tnum text-[12px]', strong ? 'font-semibold text-ink' : 'text-ink')}>{value}</dd>
    </div>
  );
}

function Header(): React.JSX.Element {
  return (
    <header className="border-b border-line bg-surface px-3 py-2">
      <h1 className="text-[13px] font-semibold text-ink">Duplicate finder</h1>
      <p className="mt-0.5 text-[10px] text-ink-faint">
        Size and SHA-256 content matching. No files are deleted.
      </p>
    </header>
  );
}
