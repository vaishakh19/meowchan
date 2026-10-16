/**
 * Diskora Pro — the pricing and licence screen.
 *
 * One one-time purchase ($12.99) unlocks both Pro features: safe
 * Recycle-Bin deletion and full-path viewing/copying. Every free
 * feature stays free. The checkout itself is hosted by Dodo
 * Payments; this screen only reports the licence state the native
 * backend verified.
 */

import { useEffect, useState } from 'react';
import {
  Check,
  Copy,
  CreditCard,
  Crown,
  Lock,
  RotateCcw,
  ShieldCheck,
  Sparkles,
  Trash2,
} from 'lucide-react';
import { useApp } from '@/app/state';
import { Badge, Button, Notice, Panel, SectionTitle, TextField, cx } from '@/components/ui/primitives';
import { formatDateTime } from '@/lib/format';

const PRICE = '$12.99';

export function ProView(): React.JSX.Element {
  const { state, actions } = useApp();
  const pro = state.pro;
  const [key, setKey] = useState('');
  const [activating, setActivating] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The page always re-verifies with Dodo when it is opened, so the
  // shown state is fresh rather than whatever the cache remembers.
  useEffect(() => {
    let cancelled = false;
    setVerifying(true);
    actions
      .verifyPro()
      .catch(() => undefined)
      .finally(() => {
        if (!cancelled) setVerifying(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const active = pro?.enabled === true;
  const configured = pro?.configured !== false;

  const submitKey = async () => {
    if (!key.trim() || activating) return;
    setActivating(true);
    setError(null);
    try {
      await actions.activateLicense(key);
      setKey('');
    } catch (activationError) {
      setError(describe(activationError));
    } finally {
      setActivating(false);
    }
  };

  const restore = async () => {
    if (restoring) return;
    setRestoring(true);
    setError(null);
    try {
      await actions.restorePurchases();
    } catch (restoreError) {
      setError(describe(restoreError));
    } finally {
      setRestoring(false);
    }
  };

  return (
    <div className="min-h-0 flex-1 overflow-y-auto" data-testid="pro-view">
      <header className="border-b border-line bg-surface px-4 py-3">
        <h1 className="flex items-center gap-2 text-[13px] font-semibold text-ink">
          <Crown aria-hidden className="size-4 text-accent" />
          Diskora Pro
        </h1>
        <p className="mt-0.5 text-[10px] text-ink-faint">
          {active
            ? 'Active on this device.'
            : 'Two premium features, one one-time payment. No subscription, ever.'}
        </p>
      </header>

      <div className="max-w-3xl px-4 py-4">
        {/* ------------------------------ pricing ------------------------------ */}
        <Panel className="p-4" data-testid="pro-pricing">
          <div className="flex flex-wrap items-end justify-between gap-3">
            <div>
              <p className="text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-faint">
                Diskora Pro
              </p>
              <p className="mt-1 text-[30px] leading-none font-semibold text-ink">
                {PRICE}
              </p>
              <p className="mt-1.5 text-[13px] font-medium text-ink">
                Pay once, use permanently.
              </p>
              <p className="mt-1 text-[11px] text-ink-muted">
                One-time purchase · No subscription · No account · No renewals
              </p>
            </div>
            {active ? (
              <Badge tone="ok" className="text-[11px]">
                <Check aria-hidden className="size-3" />
                Purchased
              </Badge>
            ) : null}
          </div>

          <ul className="mt-3 space-y-2">
            <FeatureRow
              icon={<Trash2 aria-hidden className="size-4" />}
              title="Safe file deletion"
              detail="Move files and folders to the Windows Recycle Bin, behind a confirmation that shows the exact name, full path and size. Nothing is ever deleted permanently."
              locked={!active}
            />
            <FeatureRow
              icon={<Copy aria-hidden className="size-4" />}
              title="Full file paths"
              detail="View and copy the complete absolute path of any scanned file or folder."
              locked={!active}
            />
          </ul>

          <p className="mt-3 flex items-start gap-1.5 text-[11px] leading-relaxed text-ink-muted">
            <ShieldCheck aria-hidden className="mt-px size-3.5 shrink-0 text-ok" />
            Scanning, filtering, size calculations and duplicate detection stay
            free forever — Pro only adds the two features above.
          </p>

          {!configured ? (
            <Notice tone="warn" className="mt-3" title="Purchases are not configured on this build">
              This build has no Dodo Payments product or payment link compiled
              in, so nothing can be purchased yet. The merchant needs to create
              a one-time $12.99 product with license keys in Dodo Payments and
              rebuild with <code>DISKORA_DODO_PRODUCT_ID</code> and{' '}
              <code>DISKORA_DODO_CHECKOUT_URL</code> set. See PRO-SETUP.md.
            </Notice>
          ) : null}

          {/* --------------------------- purchase states --------------------------- */}
          {configured && !active ? (
            <div className="mt-4 flex flex-col gap-2 border-t border-line pt-4">
              <Button
                variant="primary"
                className="w-full sm:w-auto"
                onClick={() => void actions.openProCheckout()}
                data-testid="upgrade-to-pro"
              >
                <CreditCard aria-hidden className="size-3.5" />
                Upgrade to Pro — {PRICE}
              </Button>
              <p className="text-[10px] text-ink-faint">
                Checkout is hosted by Dodo Payments in your browser. You will
                receive a license key by email and on the success page.
              </p>

              <div className="mt-1 flex flex-col gap-1.5 sm:flex-row sm:items-end">
                <div className="sm:w-72">
                  <TextField
                    label="Already have a license key?"
                    placeholder="Paste the key Dodo Payments emailed you"
                    value={key}
                    onChange={(event) => setKey(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') void submitKey();
                    }}
                    data-testid="license-key"
                  />
                </div>
                <Button
                  variant="secondary"
                  onClick={() => void submitKey()}
                  disabled={!key.trim() || activating}
                  loading={activating}
                  data-testid="activate-license"
                >
                  Activate
                </Button>
              </div>

              <div>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => void restore()}
                  disabled={restoring}
                  data-testid="restore-purchases"
                >
                  <RotateCcw aria-hidden className="size-3" />
                  {restoring ? 'Restoring…' : 'Restore purchases'}
                </Button>
              </div>
            </div>
          ) : null}
        </Panel>

        {/* ------------------------------ status ------------------------------ */}
        {error ? (
          <Panel className="mt-3 border-danger/30 p-3">
            <p role="alert" className="text-[12px] leading-relaxed text-danger">
              {error}
            </p>
          </Panel>
        ) : null}

        {pro?.offline ? (
          <Panel className="mt-3 p-3">
            <Notice tone="warn" title="Offline — showing the last verified purchase">
              Dodo Payments could not be reached. Pro stays active for a short
              grace period; reconnect and the purchase is re-verified.
            </Notice>
          </Panel>
        ) : null}

        {active && pro ? (
          <Panel className="mt-3 p-3" data-testid="pro-active">
            <SectionTitle className="px-0 pt-0">
              <span className="flex items-center gap-1.5">
                <Sparkles aria-hidden className="size-3 text-accent" />
                Your Diskora Pro purchase
              </span>
            </SectionTitle>
            <dl className="dw-tnum space-y-1 text-[11px]">
              <StatusRow label="Status" value="Active — both Pro features are unlocked" />
              {pro.licenseKey ? <StatusRow label="License key" value={pro.licenseKey} mono /> : null}
              {pro.activatedAt ? (
                <StatusRow label="Purchased" value={formatDateTime(pro.activatedAt)} />
              ) : null}
              {pro.lastValidatedAt ? (
                <StatusRow label="Last verified" value={formatDateTime(pro.lastValidatedAt)} />
              ) : null}
              <StatusRow label="Payment" value="One-time $12.99 via Dodo Payments" />
            </dl>
            <div className="mt-2 flex flex-wrap gap-1">
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void restore()}
                disabled={restoring}
                data-testid="restore-purchases"
              >
                <RotateCcw aria-hidden className="size-3" />
                {restoring ? 'Restoring…' : 'Restore purchases'}
              </Button>
            </div>
            <p className="mt-2 text-[10px] leading-relaxed text-ink-faint">
              Purchases are re-verified with Dodo Payments. If this device
              loses its connection, Pro keeps working for a short grace
              period. Keys are limited to a small number of devices; restore
              here after a reinstall.
            </p>
          </Panel>
        ) : null}

        {!active && !verifying && pro ? (
          <Panel className="mt-3 p-3">
            <p className="flex items-start gap-2 text-[11px] leading-relaxed text-ink-muted">
              <Lock aria-hidden className="mt-px size-3.5 shrink-0 text-ink-faint" />
              {pro.message}
            </p>
          </Panel>
        ) : null}
      </div>
    </div>
  );
}

function FeatureRow({
  icon,
  title,
  detail,
  locked,
}: {
  icon: React.ReactNode;
  title: string;
  detail: string;
  locked: boolean;
}): React.JSX.Element {
  return (
    <li className="flex items-start gap-2.5">
      <span
        className={cx(
          'mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-control border',
          locked ? 'border-line bg-surface-sunken text-ink-faint' : 'border-accent/30 bg-accent text-accent-ink',
        )}
        aria-hidden
      >
        {locked ? <Lock className="size-3" /> : icon}
      </span>
      <div className="min-w-0">
        <p className="flex flex-wrap items-center gap-1.5 text-[12px] font-medium text-ink">
          {title}
          {locked ? (
            <Badge tone="accent" className="text-[9px]">
              PRO
            </Badge>
          ) : null}
        </p>
        <p className="mt-0.5 text-[11px] leading-relaxed text-ink-muted">{detail}</p>
      </div>
    </li>
  );
}

function StatusRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex flex-wrap items-baseline justify-between gap-x-3">
      <dt className="shrink-0 text-ink-faint">{label}</dt>
      <dd className={cx('min-w-0 break-all text-right text-ink', mono && 'font-mono text-[10px]')} title={value}>
        {value}
      </dd>
    </div>
  );
}

function describe(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'The operation could not be completed.';
}
