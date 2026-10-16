/**
 * Cleanup assistant.
 *
 * Every finding states what it is, why it was suggested and exactly which paths it covers. Nothing
 * is deleted until the user confirms, the preferred action is the Recycle Bin, and permanent
 * deletion is a separate, clearly-labelled step with its own confirmation.
 *
 * Findings are classified exactly as the brief requires:
 *   informational — reported only; the checkbox is disabled and no action is offered
 *   low          — safe to offer (temp files, caches, Recycle Bin)
 *   review       — large, old or personal-looking; the user must opt in per finding
 *
 * Diskora never auto-classifies documents, browser profiles, development dependencies or package
 * caches as junk, and never offers Windows system files or application data for deletion.
 */

import { useMemo, useState } from 'react';
import { AlertTriangle, FolderOpen, Info, ShieldCheck, Trash2 } from 'lucide-react';
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
  SectionTitle,
  Select,
  cx,
} from '@/components/ui/primitives';
import { Modal } from '@/components/ui/feedback';
import { formatBytes, formatCount, formatRelativeTime } from '@/lib/format';
import type { CleanupFinding, CleanupPlan, CleanupPlanItem, CleanupRisk, CleanupRule } from '@/types/contracts';

const RISK_TONE: Record<CleanupRisk, 'neutral' | 'warn' | 'danger'> = {
  informational: 'neutral',
  low: 'warn',
  review: 'danger',
};

const RISK_LABEL: Record<CleanupRisk, string> = {
  informational: 'Informational',
  low: 'Low risk',
  review: 'Needs review',
};

export function CleanupView(): React.JSX.Element {
  const { state, actions } = useApp();
  const itemMenu = useItemMenu();
  const { cleanup, scan, entitlement } = state;

  const [dryRun, setDryRun] = useState(true);
  const [targetsText, setTargetsText] = useState('');
  const [targets, setTargets] = useState<string[]>([]);
  const [olderThanDays, setOlderThanDays] = useState(180);
  const [confirm, setConfirm] = useState<{ permanent: boolean } | null>(null);

  const hasScan = scan.result !== null;
  const findings = cleanup.findings;
  const selectedIds = cleanup.selectedFindingIds;

  const selected = useMemo(
    () => findings.filter((finding) => selectedIds.includes(finding.id) && !finding.protected),
    [findings, selectedIds],
  );
  const selectedBytes = selected.reduce((sum, finding) => sum + finding.sizeBytes, 0);
  const selectedFiles = selected.reduce((sum, finding) => sum + finding.fileCount, 0);

  const byRisk = useMemo(() => {
    const map = new Map<CleanupRisk, CleanupFinding[]>();
    for (const finding of findings) {
      const list = map.get(finding.risk) ?? [];
      list.push(finding);
      map.set(finding.risk, list);
    }
    return map;
  }, [findings]);

  const buildPlan = (permanent: boolean): CleanupPlan | null => {
    if (selected.length === 0) return null;
    const items: CleanupPlanItem[] = selected.map((finding) => ({
      findingId: finding.id,
      paths: finding.paths,
      action: permanent ? 'permanent' : 'recycle',
    }));
    return { scanId: cleanup.scanId ?? scan.result?.scanId ?? null, items, dryRun };
  };

  const run = (permanent: boolean) => {
    const plan = buildPlan(permanent);
    if (!plan) return;
    void actions.executeCleanup(plan).then(() => setConfirm(null));
  };

  if (!hasScan) {
    return (
      <div className="flex h-full flex-col" data-testid="cleanup-view">
        <Header />
        <EmptyState
          icon={<Trash2 aria-hidden className="size-6" />}
          title="Scan a drive first"
          detail="Cleanup suggestions are derived from scan results, so every finding lists real paths and real byte totals."
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
    <div className="flex h-full min-h-0 flex-col" data-testid="cleanup-view">
      <Header />

      <div className="flex min-h-0 flex-1">
        <div className="min-h-0 flex-1 overflow-y-auto">
          {/* ----------------------------- rule setup ----------------------------- */}
          <Panel className="m-3 p-3">
            <SectionTitle className="px-0 pt-0">Folders to review</SectionTitle>
            <p className="mb-2 text-[11px] leading-relaxed text-ink-muted">
              Diskora never decides on your own documents, browser profiles, development
              dependencies or package caches. Add the temporary and cache folders you want reviewed;
              everything else is only reported, never offered for deletion.
            </p>
            <form
              className="flex flex-wrap items-end gap-2"
              onSubmit={(event) => {
                event.preventDefault();
                const parts = targetsText
                  .split(/[\n;]/)
                  .map((part) => part.trim())
                  .filter((part) => part.length > 0);
                setTargets(parts);
                void actions.runCleanupScan(parts.map((path) => buildRule(path, olderThanDays)));
              }}
            >
              <label className="block">
                <span className="mb-1 block text-[11px] font-medium text-ink-muted">Target directories (one per line)</span>
                <textarea
                  value={targetsText}
                  onChange={(event) => setTargetsText(event.target.value)}
                  placeholder={'C:\\Users\\you\\AppData\\Local\\Temp'}
                  rows={3}
                  className="w-full max-w-lg rounded-control border border-line bg-surface px-2.5 py-1.5 font-mono text-[11px] text-ink placeholder:text-ink-faint focus:border-info focus:outline-none"
                />
              </label>
              <label className="block">
                <span className="mb-1 block text-[11px] font-medium text-ink-muted">Only files not modified in</span>
                <Select
                  value={String(olderThanDays)}
                  onChange={(event) => setOlderThanDays(Number(event.target.value))}
                  className="h-8 w-44 text-xs"
                >
                  <option value="0">Any age</option>
                  <option value="30">30 days</option>
                  <option value="90">90 days</option>
                  <option value="180">180 days</option>
                  <option value="365">a year</option>
                </Select>
              </label>
              <Button type="submit" variant="primary" loading={cleanup.loading}>
                <ShieldCheck aria-hidden className="size-3.5" />
                Analyse these folders
              </Button>
            </form>
            {targets.length > 0 ? (
              <ul className="mt-2 flex flex-wrap gap-1">
                {targets.map((path) => (
                  <li
                    key={path}
                    className="max-w-80 truncate rounded-full border border-line bg-surface-sunken px-2 py-0.5 font-mono text-[10px] text-ink-muted"
                    title={path}
                  >
                    {path}
                  </li>
                ))}
              </ul>
            ) : null}
          </Panel>

          {cleanup.error ? (
            <div className="px-3">
              <ErrorState title="Cleanup analysis failed" detail={cleanup.error} onRetry={() => void actions.loadCleanup()} />
            </div>
          ) : null}

          {/* ------------------------------ findings ------------------------------ */}
          {findings.length === 0 ? (
            <div className="px-3 pb-3">
              <Panel>
                <EmptyState
                  icon={<ShieldCheck aria-hidden className="size-6" />}
                  title="Nothing to review yet"
                  detail="Add the folders you want analysed above. Suggestions then appear with their exact paths, byte totals and the reason Diskora flagged them."
                />
              </Panel>
            </div>
          ) : (
            <div className="px-3 pb-3">
              {(['review', 'low', 'informational'] as CleanupRisk[]).map((risk) => {
                const list = byRisk.get(risk);
                if (!list || list.length === 0) return null;
                return (
                  <section key={risk} className="mb-3">
                    <SectionTitle
                      className="px-0 pt-0 pb-1"
                      action={
                        <span className="dw-tnum text-[10px] text-ink-faint">
                          {formatCount(list.reduce((sum, f) => sum + f.fileCount, 0))} items ·{' '}
                          {formatBytes(list.reduce((sum, f) => sum + f.sizeBytes, 0))}
                        </span>
                      }
                    >
                      {RISK_LABEL[risk]}
                    </SectionTitle>
                    <ul className="flex flex-col gap-2">
                      {list.map((finding) => (
                        <FindingCard
                          key={finding.id}
                          finding={finding}
                          checked={selectedIds.includes(finding.id)}
                          onToggle={() => actions.toggleCleanupFinding(finding.id)}
                          onReveal={(path) => void actions.reveal(path)}
                          onContextMenu={(path, x, y) => itemMenu.open(path, x, y)}
                        />
                      ))}
                    </ul>
                  </section>
                );
              })}
            </div>
          )}

          {/* ----------------------------- recycle bin ----------------------------- */}
          {cleanup.recycleBin ? (
            <Panel className="mx-3 mb-3 p-3">
              <SectionTitle className="px-0 pt-0">Recycle Bin</SectionTitle>
              <p className="dw-tnum text-[12px] text-ink">
                {cleanup.recycleBin.itemCount === 0
                  ? 'Empty'
                  : `${formatCount(cleanup.recycleBin.itemCount)} items${
                      cleanup.recycleBin.sizeBytes !== null ? ` · ${formatBytes(cleanup.recycleBin.sizeBytes)}` : ''
                    }`}
              </p>
              <p className="mt-1 text-[10px] leading-relaxed text-ink-faint">{cleanup.recycleBin.detail}</p>
              <Button
                size="sm"
                className="mt-2"
                variant="secondary"
                disabled={cleanup.executing || cleanup.recycleBin.itemCount === 0}
                onClick={() =>
                  void actions.executeCleanup({
                    scanId: cleanup.scanId,
                    items: [{ findingId: 'recycle-bin', paths: [], action: 'recycle' }],
                    dryRun: true,
                  })
                }
              >
                Preview emptying the Recycle Bin
              </Button>
            </Panel>
          ) : null}

          {/* ------------------------------- preview ------------------------------- */}
          {cleanup.preview ? (
            <Panel className="mx-3 mb-3 p-3">
              <SectionTitle className="px-0 pt-0">Dry-run preview</SectionTitle>
              <div className="dw-tnum flex flex-wrap gap-4 text-[12px]">
                <Stat label="Items" value={formatCount(cleanup.preview.totalPathCount)} />
                <Stat label="Total" value={formatBytes(cleanup.preview.totalSizeBytes)} strong />
              </div>
              {cleanup.preview.warnings.length > 0 ? (
                <Notice tone="warn" className="mt-2">
                  {cleanup.preview.warnings.join(' ')}
                </Notice>
              ) : null}
              <ul className="mt-2 max-h-40 overflow-y-auto rounded-control bg-surface-sunken p-2">
                {cleanup.preview.items.flatMap((item) =>
                  item.paths.map((outcome) => (
                    <li key={`${item.findingId}:${outcome.path}`} className="truncate font-mono text-[10px] text-ink-muted" title={outcome.path}>
                      <span className={cx(outcome.status !== 'ok' && 'text-warn')}>{outcome.status}</span> · {outcome.path}
                    </li>
                  )),
                )}
              </ul>
            </Panel>
          ) : null}

          {/* ------------------------------ execution ------------------------------ */}
          {cleanup.result ? (
            <Panel className="mx-3 mb-3 p-3">
              <SectionTitle className="px-0 pt-0">Result</SectionTitle>
              <div className="dw-tnum flex flex-wrap gap-4">
                <Stat label="Removed" value={formatCount(cleanup.result.totalRemovedCount)} />
                <Stat label="Freed" value={formatBytes(cleanup.result.totalRemovedBytes)} strong />
                <Stat label="Failed" value={formatCount(cleanup.result.totalFailedCount)} />
              </div>
              {cleanup.result.partial ? (
                <Notice tone="warn" className="mt-2">
                  Only some items could be handled. The list below shows exactly what failed and why.
                </Notice>
              ) : null}
              <ul className="mt-2 max-h-40 overflow-y-auto rounded-control bg-surface-sunken p-2">
                {cleanup.result.items.flatMap((item) =>
                  item.results
                    .filter((outcome) => outcome.status !== 'ok')
                    .map((outcome) => (
                      <li key={`${item.findingId}:${outcome.path}`} className="truncate font-mono text-[10px] text-danger" title={outcome.path}>
                        {outcome.status} · {outcome.path} — {outcome.detail}
                      </li>
                    )),
                )}
              </ul>
            </Panel>
          ) : null}
        </div>

        {/* ------------------------------ action rail ------------------------------ */}
        <aside className="w-60 shrink-0 border-l border-line bg-surface p-3" aria-label="Cleanup actions">
          <SectionTitle className="px-0 pt-0">Selected for action</SectionTitle>
          {selected.length === 0 ? (
            <p className="text-[11px] leading-relaxed text-ink-muted">
              Nothing selected. Tick the suggestions you understand and want to act on.
            </p>
          ) : (
            <ul className="mb-2 flex flex-col gap-1">
              {selected.map((finding) => (
                <li key={finding.id} className="flex items-start gap-1.5">
                  <span className="min-w-0 flex-1 truncate text-[11px] text-ink" title={finding.title}>
                    {finding.title}
                  </span>
                  <span className="dw-tnum shrink-0 text-[11px] text-ink-muted">{formatBytes(finding.sizeBytes)}</span>
                </li>
              ))}
            </ul>
          )}

          <dl className="dw-tnum space-y-1 border-t border-line pt-2 text-[11px]">
            <div className="flex justify-between">
              <dt className="text-ink-faint">Items</dt>
              <dd className="text-ink">{formatCount(selectedFiles)}</dd>
            </div>
            <div className="flex justify-between">
              <dt className="text-ink-faint">Total</dt>
              <dd className="font-semibold text-ink" data-testid="cleanup-selected-bytes">
                {formatBytes(selectedBytes)}
              </dd>
            </div>
          </dl>

          <div className="mt-3 border-t border-line pt-3">
            <label className="flex items-center gap-2">
              <Checkbox
                checked={dryRun}
                onChange={setDryRun}
                label={<span className="text-[11px] text-ink">Dry run (report only, change nothing)</span>}
              />
            </label>
            <p className="mt-1 text-[10px] leading-relaxed text-ink-faint">
              Every path is re-checked immediately before acting. Anything that changed since the scan
              is skipped and reported.
            </p>
          </div>

          <div className="mt-3 space-y-1.5 border-t border-line pt-3">
            <Button
              variant="primary"
              className="w-full justify-center"
              disabled={selected.length === 0 || cleanup.executing}
              loading={cleanup.executing}
              onClick={() => run(false)}
            >
              <Trash2 aria-hidden className="size-3.5" />
              {dryRun ? 'Preview cleanup' : 'Move to Recycle Bin'}
            </Button>
            <Button
              variant="danger"
              className="w-full justify-center"
              disabled={selected.length === 0 || cleanup.executing}
              onClick={() => setConfirm({ permanent: true })}
              title="Permanently delete the selected items. This cannot be undone."
            >
              <AlertTriangle aria-hidden className="size-3.5" />
              Permanently delete…
            </Button>
            {!entitlement.features.customCleanupRules ? (
              <p className="text-[10px] leading-relaxed text-ink-faint">
                Custom cleanup rules are part of Diskora Pro.{' '}
                <button type="button" className="underline" onClick={() => actions.navigate('licensing')}>
                  See plans
                </button>
              </p>
            ) : null}
          </div>
        </aside>
      </div>

      <Modal
        open={confirm !== null}
        title="Permanently delete these items?"
        description="Permanent deletion bypasses the Recycle Bin and cannot be undone."
        onClose={() => setConfirm(null)}
        footer={
          <>
            <Button variant="ghost" onClick={() => setConfirm(null)}>
              Cancel
            </Button>
            <Button variant="danger" onClick={() => run(true)}>
              Delete permanently
            </Button>
          </>
        }
      >
        {confirm ? (
          <>
            <p className="mb-2 text-[11px] font-medium text-ink">
              {selected.length} findings · {formatCount(selectedFiles)} items · {formatBytes(selectedBytes)}
            </p>
            <p className="mb-1 text-[11px] font-medium text-ink">Exact paths:</p>
            <ul className="max-h-40 overflow-y-auto rounded-control border border-line bg-surface-sunken p-2">
              {selected.flatMap((finding) => finding.paths).map((path) => (
                <li key={path} className="truncate font-mono text-[10px] text-ink-muted" title={path}>
                  {path}
                </li>
              ))}
            </ul>
            <Notice tone="danger" className="mt-2">
              At least one copy of every duplicate group is always preserved, and files that changed
              since the scan are re-checked immediately before deletion and skipped if they moved.
            </Notice>
          </>
        ) : null}
      </Modal>
    </div>
  );
}

function buildRule(path: string, olderThanDays: number): CleanupRule {
  return {
    id: `rule-${path}`,
    label: `User rule: ${path}`,
    category: 'custom',
    risk: 'review',
    path,
    olderThanDays,
    minBytes: 0,
    extensions: [],
    recursive: true,
    enabled: true,
  };
}

function FindingCard({
  finding,
  checked,
  onToggle,
  onReveal,
  onContextMenu,
}: {
  finding: CleanupFinding;
  checked: boolean;
  onToggle: () => void;
  onReveal: (path: string) => void;
  onContextMenu: (path: string, x: number, y: number) => void;
}): React.JSX.Element {
  return (
    <li>
      <Panel className={cx('p-2.5', checked && 'border-accent')}>
        <div className="flex items-start gap-2.5">
          <Checkbox
            checked={checked}
            onChange={onToggle}
            disabled={finding.protected}
            label={<span className="sr-only">Select {finding.title}</span>}
          />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-1.5">
              <p className="text-[12px] font-medium text-ink">{finding.title}</p>
              <Badge tone={RISK_TONE[finding.risk]}>{RISK_LABEL[finding.risk]}</Badge>
              {finding.protected ? (
                <span className="flex items-center gap-1 text-[10px] text-ink-faint">
                  <Info aria-hidden className="size-2.5" />
                  protected — reported only
                </span>
              ) : null}
            </div>
            <p className="mt-0.5 text-[11px] leading-relaxed text-ink-muted">{finding.reason}</p>
            <div className="dw-tnum mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[10px] text-ink-faint">
              <span>
                <strong className="text-ink">{formatBytes(finding.sizeBytes)}</strong>
                {finding.diskSizeBytes !== null ? ` (${formatBytes(finding.diskSizeBytes)} on disk)` : ''} in{' '}
                {formatCount(finding.fileCount)} files, {formatCount(finding.folderCount)} folders
              </span>
              {finding.oldestModifiedAt !== null ? (
                <span>oldest {formatRelativeTime(finding.oldestModifiedAt)}</span>
              ) : null}
              {finding.newestModifiedAt !== null ? (
                <span>newest {formatRelativeTime(finding.newestModifiedAt)}</span>
              ) : null}
              <span>category {finding.category}</span>
            </div>
            {finding.paths.length > 0 ? (
              <ul className="mt-1.5 max-h-24 overflow-y-auto rounded-control bg-surface-sunken p-1.5">
                {finding.paths.slice(0, 8).map((path) => (
                  <li
                    key={path}
                    className="truncate font-mono text-[10px] text-ink-muted"
                    onContextMenu={(event) => {
                      event.preventDefault();
                      onContextMenu(path, event.clientX, event.clientY);
                    }}
                  >
                    <button type="button" className="hover:text-ink hover:underline" onClick={() => onReveal(path)} title={path}>
                      {path}
                    </button>
                  </li>
                ))}
                {finding.paths.length > 8 ? (
                  <li className="text-[10px] text-ink-faint">… and {formatCount(finding.paths.length - 8)} more</li>
                ) : null}
              </ul>
            ) : null}
          </div>
          <div className="flex shrink-0 flex-col gap-1">
            <Button
              size="sm"
              variant="ghost"
              onClick={() => onReveal(finding.paths[0] ?? '')}
              disabled={finding.paths.length === 0}
            >
              <FolderOpen aria-hidden className="size-3" />
              Open
            </Button>
          </div>
        </div>
      </Panel>
    </li>
  );
}

function Stat({ label, value, strong }: { label: string; value: string; strong?: boolean }) {
  return (
    <div>
      <dt className="text-[10px] text-ink-faint">{label}</dt>
      <dd className={cx('dw-tnum text-[12px]', strong && 'font-semibold text-ink')}>{value}</dd>
    </div>
  );
}

function Header(): React.JSX.Element {
  return (
    <header className="border-b border-line bg-surface px-3 py-2">
      <h1 className="text-[13px] font-semibold text-ink">Cleanup suggestions</h1>
      <p className="mt-0.5 text-[10px] text-ink-faint">
        Every action lists exact paths, requires confirmation, and prefers the Recycle Bin.
      </p>
    </header>
  );
}
