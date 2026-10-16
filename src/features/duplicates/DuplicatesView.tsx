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

import { useMemo, useState } from 'react';
import { Copy, FolderOpen, ShieldCheck, ShieldOff } from 'lucide-react';
import { useApp } from '@/app/state';
import { useItemMenu } from '@/app/itemMenu';
import {
  Badge,
  Button,
  Checkbox,
  EmptyState,
  ErrorState,
  Notice,
  Panel,
  ProgressBar,
  SectionTitle,
  cx,
} from '@/components/ui/primitives';
import { Modal } from '@/components/ui/feedback';
import { formatBytes, formatCount, formatDateTime, formatRelativeTime } from '@/lib/format';
import { applyDuplicateLimit } from '@/features/licensing/entitlements';
import type { DuplicateGroup } from '@/types/contracts';

const MB = 1024 * 1024;

export function DuplicatesView(): React.JSX.Element {
  const { state, actions } = useApp();
  const itemMenu = useItemMenu();
  const { duplicates, scan, entitlement } = state;

  const [minBytes, setMinBytes] = useState(1 * MB);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);

  const hasScan = scan.result !== null;
  const report = duplicates.report;
  const limit = entitlement.duplicateGroupLimit;
  const limited = useMemo(
    () => (report ? applyDuplicateLimit(report.groups, limit) : { groups: [], limited: false }),
    [report, limit],
  );

  const selectedBytes = useMemo(() => {
    if (!report) return 0;
    let total = 0;
    for (const group of report.groups) {
      for (const copy of group.copies) {
        if (state.duplicates.selectedPaths.includes(copy.path)) total += copy.sizeBytes;
      }
    }
    return total;
  }, [report, state.duplicates.selectedPaths]);

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
        {state.duplicates.selectedPaths.length > 0 ? (
          <>
            <span className="dw-tnum text-[11px] text-ink-muted">
              {formatCount(state.duplicates.selectedPaths.length)} copies selected · {formatBytes(selectedBytes)}
            </span>
            <Button size="sm" variant="ghost" onClick={actions.clearDuplicateSelection}>
              Clear selection
            </Button>
            <Button size="sm" onClick={() => setConfirmOpen(true)}>
              Review for cleanup…
            </Button>
          </>
        ) : null}
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

          {limited.limited ? (
            <Notice tone="warn" title="Free plan preview" className="mb-2">
              Showing the first {formatCount(limit)} of {formatCount(report.totalGroups)} groups. Diskora Pro
              unlocks every group plus batch review and byte-for-byte verification.{' '}
              <button type="button" className="underline" onClick={() => actions.navigate('licensing')}>
                Compare plans
              </button>
            </Notice>
          ) : null}

          <ul className="flex flex-col gap-2">
            {limited.groups.map((group) => (
              <GroupCard
                key={group.groupId}
                group={group}
                expanded={expanded === group.groupId}
                onToggle={() => setExpanded(expanded === group.groupId ? null : group.groupId)}
                selectedPaths={state.duplicates.selectedPaths}
                onTogglePath={actions.toggleDuplicatePath}
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
          <SectionTitle className="px-0 pt-0 pb-1">Excluded folders</SectionTitle>
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

      <Modal
        open={confirmOpen}
        title="Review duplicate copies"
        description="Nothing has been deleted. These copies are handed to the cleanup assistant, which asks for a final confirmation and prefers the Recycle Bin."
        onClose={() => setConfirmOpen(false)}
        footer={
          <>
            <Button variant="ghost" onClick={() => setConfirmOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                setConfirmOpen(false);
                actions.navigate('cleanup');
                actions.toast({
                  kind: 'info',
                  title: 'Copies queued for review',
                  detail: `${formatCount(state.duplicates.selectedPaths.length)} copies · ${formatBytes(selectedBytes)}`,
                });
              }}
            >
              Continue to cleanup review
            </Button>
          </>
        }
      >
        <ul className="flex flex-col gap-1">
          {state.duplicates.selectedPaths.map((path) => (
            <li key={path} className="truncate font-mono text-[11px] text-ink-muted" title={path}>
              {path}
            </li>
          ))}
        </ul>
        <Notice tone="warn" className="mt-2">
          At least one copy of every group is always kept. Diskora never offers the only remaining
          copy of a file for removal.
        </Notice>
      </Modal>
    </div>
  );
}

function GroupCard({
  group,
  expanded,
  onToggle,
  selectedPaths,
  onTogglePath,
  onReveal,
  onExclude,
  onContextMenu,
}: {
  group: DuplicateGroup;
  expanded: boolean;
  onToggle: () => void;
  selectedPaths: string[];
  onTogglePath: (path: string) => void;
  onReveal: (path: string) => void;
  onExclude: (path: string) => void;
  onContextMenu: (path: string, x: number, y: number) => void;
}): React.JSX.Element {
  const selectedInGroup = group.copies.filter((copy) => selectedPaths.includes(copy.path)).length;
  // Never allow every copy in a group to be selected for removal.
  const blockAll = selectedInGroup === group.copies.length;

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
                  <th className="px-2.5 py-1 font-semibold">Keep</th>
                  <th className="px-2.5 py-1 font-semibold">Path</th>
                  <th className="px-2.5 py-1 font-semibold">Modified</th>
                  <th className="px-2.5 py-1 font-semibold" />
                </tr>
              </thead>
              <tbody>
                {group.copies.map((copy) => {
                  const selected = selectedPaths.includes(copy.path);
                  return (
                    <tr
                      key={copy.path}
                      className={cx('border-t border-line/60', selected && 'bg-info-soft')}
                      onContextMenu={(event) => {
                        event.preventDefault();
                        onContextMenu(copy.path, event.clientX, event.clientY);
                      }}
                    >
                      <td className="px-2.5 py-1.5">
                        <Checkbox
                          checked={selected}
                          disabled={blockAll && !selected}
                          onChange={() => onTogglePath(copy.path)}
                          label={<span className="sr-only">Select {copy.path} for cleanup review</span>}
                        />
                      </td>
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
                          <Button size="sm" variant="ghost" onClick={() => onExclude(copy.path)} title="Exclude this folder">
                            <ShieldOff aria-hidden className="size-3" />
                          </Button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            {blockAll ? (
              <p className="border-t border-line bg-warn-soft px-2.5 py-1.5 text-[10px] text-warn">
                Every copy is selected — at least one must be kept, so removal is disabled until you
                deselect one.
              </p>
            ) : null}
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
        Size → SHA-256 → optional byte-for-byte verification. Filenames are never treated as evidence.
      </p>
    </header>
  );
}
