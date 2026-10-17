/**
 * Windows Storage Breakdown.
 *
 * Inspects Windows-managed system areas: Recycle Bin, System Restore / Shadow Copies (VSS),
 * Hibernation file (hiberfil.sys), Pagefile (pagefile.sys/swapfile.sys), and system temp storage.
 *
 * Distinguishes measured values from estimates and inaccessible protected data.
 * Purely read-only inspection: never deletes protected system files or alters system settings.
 */

import {
  Database,
  HardDrive,
  Info,
  Layers,
  Lock,
  Moon,
  RefreshCw,
  ShieldCheck,
  Trash2,
} from 'lucide-react';
import { useApp } from '@/app/state';
import { Badge, Button, EmptyState, Panel, SectionTitle, Spinner, cx } from '@/components/ui/primitives';
import { formatBytes, formatCount, formatDateTime } from '@/lib/format';
import type { StorageBreakdownItem, StorageItemStatus } from '@/types/contracts';

function statusTone(status: StorageItemStatus): 'ok' | 'warn' | 'neutral' | 'info' {
  switch (status) {
    case 'measured':
      return 'ok';
    case 'estimated':
      return 'info';
    case 'inaccessible':
      return 'warn';
    case 'not-configured':
      return 'neutral';
    case 'unsupported':
    default:
      return 'neutral';
  }
}

function statusLabel(status: StorageItemStatus): string {
  switch (status) {
    case 'measured':
      return 'Measured';
    case 'estimated':
      return 'Estimated';
    case 'inaccessible':
      return 'Inaccessible (Admin Required)';
    case 'not-configured':
      return 'Disabled / Not Present';
    case 'unsupported':
      return 'Not Available';
    default:
      return status;
  }
}

function itemIcon(id: string) {
  switch (id) {
    case 'recycle-bin':
      return <Trash2 aria-hidden className="size-4 shrink-0 text-ink-muted" />;
    case 'system-restore-vss':
      return <Layers aria-hidden className="size-4 shrink-0 text-ink-muted" />;
    case 'hibernation-file':
      return <Moon aria-hidden className="size-4 shrink-0 text-ink-muted" />;
    case 'virtual-memory':
      return <Database aria-hidden className="size-4 shrink-0 text-ink-muted" />;
    default:
      return <HardDrive aria-hidden className="size-4 shrink-0 text-ink-muted" />;
  }
}

export function StorageBreakdownSection(): React.JSX.Element {
  const { state, actions } = useApp();
  const { breakdown, selectedDriveId, drives } = state;
  const activeDrive = drives.find((d) => d.id === selectedDriveId) ?? drives[0];
  const report = breakdown.report;

  return (
    <section className="px-5 pb-4" data-testid="storage-breakdown-section">
      <SectionTitle
        className="px-0"
        action={
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void actions.loadStorageBreakdown(selectedDriveId ?? undefined)}
            disabled={breakdown.loading}
          >
            <RefreshCw
              aria-hidden
              className={cx('size-3', breakdown.loading && 'animate-spin')}
            />
            {breakdown.loading ? 'Measuring…' : 'Refresh breakdown'}
          </Button>
        }
      >
        <span className="flex items-center gap-1.5">
          <Layers aria-hidden className="size-3.5" />
          Windows Storage & System Allocation
        </span>
      </SectionTitle>

      {breakdown.loading && !report ? (
        <Panel className="flex items-center justify-center p-6">
          <Spinner label="Measuring Windows system storage allocations…" />
        </Panel>
      ) : breakdown.error ? (
        <Panel className="p-4 text-xs text-danger">
          <p className="font-medium">Could not inspect Windows storage breakdown:</p>
          <p className="mt-1 font-mono text-[11px] text-ink-muted">{breakdown.error}</p>
        </Panel>
      ) : !report || report.items.length === 0 ? (
        <Panel className="p-4">
          <EmptyState
            icon={<Layers aria-hidden className="size-6" />}
            title="Storage breakdown ready"
            detail={`Inspect system storage allocations on ${activeDrive ? activeDrive.name : 'the selected volume'}.`}
            action={
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void actions.loadStorageBreakdown(selectedDriveId ?? undefined)}
              >
                Inspect storage now
              </Button>
            }
          />
        </Panel>
      ) : (
        <div className="space-y-2">
          <div className="grid grid-cols-[repeat(auto-fit,minmax(300px,1fr))] gap-2">
            {report.items.map((item) => (
              <BreakdownCard key={item.id} item={item} />
            ))}
          </div>

          <div className="flex flex-wrap items-center justify-between gap-2 rounded-control bg-surface-sunken px-3 py-2 text-[11px] text-ink-muted">
            <span className="flex items-center gap-1.5">
              <ShieldCheck aria-hidden className="size-3.5 text-ok" />
              <span>Read-only inspection: Diskora never deletes protected system files or alters system configuration automatically.</span>
            </span>
            <span className="dw-tnum text-[10px] text-ink-faint">
              Measured {formatDateTime(report.measuredAt)}
            </span>
          </div>
        </div>
      )}
    </section>
  );
}

function BreakdownCard({ item }: { item: StorageBreakdownItem }): React.JSX.Element {
  return (
    <Panel className="flex flex-col justify-between p-3" data-testid={`breakdown-card-${item.id}`}>
      <div>
        <div className="flex items-start justify-between gap-2">
          <div className="flex items-center gap-2 min-w-0">
            {itemIcon(item.id)}
            <h3 className="truncate text-[12px] font-semibold text-ink" title={item.name}>
              {item.name}
            </h3>
          </div>
          <Badge tone={statusTone(item.status)}>{statusLabel(item.status)}</Badge>
        </div>

        <div className="mt-2.5 flex items-baseline justify-between gap-2 border-b border-line pb-2">
          <div className="dw-tnum text-base font-bold tracking-tight text-ink">
            {item.sizeBytes !== null ? (
              formatBytes(item.sizeBytes)
            ) : item.status === 'inaccessible' ? (
              <span className="text-xs font-normal text-warn">Inaccessible without admin</span>
            ) : (
              <span className="text-xs font-normal text-ink-faint">—</span>
            )}
          </div>
          {item.itemCount !== null && item.itemCount > 0 ? (
            <span className="dw-tnum text-[11px] text-ink-muted">
              {formatCount(item.itemCount)} items
            </span>
          ) : null}
        </div>

        <p className="mt-2 text-[11px] leading-relaxed text-ink-muted">{item.description}</p>
      </div>

      <div className="mt-3 rounded-control bg-surface-sunken p-2 text-[10.5px] leading-relaxed">
        <p className="font-medium text-ink flex items-center gap-1">
          {item.isSystemProtected ? (
            <Lock aria-hidden="true" className="size-3 text-ink-faint shrink-0" />
          ) : (
            <Info aria-hidden="true" className="size-3 text-ink-faint shrink-0" />
          )}
          <span>{item.isSystemProtected ? 'System Protected' : 'Actionable Note'}</span>
        </p>
        <p className="mt-0.5 text-ink-muted">{item.actionableExplanation}</p>
      </div>
    </Panel>
  );
}
