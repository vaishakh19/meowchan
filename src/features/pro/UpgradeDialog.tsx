/**
 * "Upgrade to Pro" dialog.
 *
 * Every locked action (Delete, Reveal in File Explorer, Copy full path) opens this dialog instead
 * of doing nothing. It states the one-time price, starts checkout, and lets the user activate or
 * restore a licence in place. The licence itself is always checked by the native backend against
 * Dodo Payments; this dialog only reports the result.
 */

import { useState } from 'react';
import { Check, Copy, CreditCard, Crown, RotateCcw, ShieldCheck, Trash2 } from 'lucide-react';
import { useApp } from '@/app/state';
import { Button, Notice, TextField } from '@/components/ui/primitives';
import { Modal } from '@/components/ui/feedback';
import { formatDateTime } from '@/lib/format';

const PRICE = '$12.99';

export function UpgradeDialog(): React.JSX.Element {
  const { state, actions } = useApp();
  const pro = state.pro;
  const active = pro?.enabled === true;
  const configured = pro?.configured !== false;
  const feature = state.upgrade.feature;

  const [key, setKey] = useState('');
  const [busy, setBusy] = useState<'activate' | 'restore' | 'checkout' | null>(null);
  const [error, setError] = useState<string | null>(null);

  const close = () => {
    setError(null);
    setKey('');
    actions.closeUpgrade();
  };

  const activate = async () => {
    if (!key.trim() || busy) return;
    setBusy('activate');
    setError(null);
    try {
      const status = await actions.activateLicense(key);
      if (status.enabled) {
        setKey('');
        close();
      } else {
        setError(status.message);
      }
    } catch (activationError) {
      setError(describe(activationError));
    } finally {
      setBusy(null);
    }
  };

  const restore = async () => {
    if (busy) return;
    setBusy('restore');
    setError(null);
    try {
      const status = await actions.restorePurchases();
      if (status.enabled) close();
      else setError(status.message);
    } catch (restoreError) {
      setError(describe(restoreError));
    } finally {
      setBusy(null);
    }
  };

  const checkout = async () => {
    setBusy('checkout');
    try {
      await actions.openProCheckout();
    } finally {
      setBusy(null);
    }
  };

  const footer = (
    <Button variant="ghost" onClick={close}>
      {active ? 'Close' : 'Not now'}
    </Button>
  );

  return (
    <Modal
      open={state.upgrade.open}
      onClose={close}
      title={active ? 'Diskora Pro is active' : 'Upgrade to Diskora Pro'}
      description={
        active
          ? 'Both Pro features are unlocked on this device.'
          : feature
            ? `${feature} is a Diskora Pro feature. One payment unlocks it.`
            : 'One payment unlocks both Pro features on this device.'
      }
      footer={footer}
      width="max-w-md"
    >
      <div className="space-y-3" data-testid="upgrade-dialog">
        <div className="flex items-end justify-between gap-3 rounded-control border border-line bg-surface-sunken p-3">
          <div>
            <p className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-faint">
              <Crown aria-hidden className="size-3 text-accent" />
              Diskora Pro
            </p>
            <p className="mt-1 text-[26px] leading-none font-semibold text-ink">{PRICE}</p>
            <p className="mt-1 text-[11px] text-ink-muted">One-time payment · No subscription · No account</p>
          </div>
          {active ? (
            <span className="inline-flex items-center gap-1 rounded-full bg-ok-soft px-2 py-0.5 text-[11px] font-medium text-ok">
              <Check aria-hidden className="size-3" />
              Active
            </span>
          ) : null}
        </div>

        <ul className="space-y-2 text-[12px] text-ink">
          <li className="flex items-start gap-2">
            <Trash2 aria-hidden className="mt-0.5 size-3.5 shrink-0 text-accent" />
            <span>
              <span className="font-medium">Delete to Recycle Bin.</span>{' '}
              <span className="text-ink-muted">Confirmed first, and never deleted permanently.</span>
            </span>
          </li>
          <li className="flex items-start gap-2">
            <Copy aria-hidden className="mt-0.5 size-3.5 shrink-0 text-accent" />
            <span>
              <span className="font-medium">Full paths.</span>{' '}
              <span className="text-ink-muted">Show, copy and reveal in File Explorer.</span>
            </span>
          </li>
          <li className="flex items-start gap-2">
            <ShieldCheck aria-hidden className="mt-0.5 size-3.5 shrink-0 text-ok" />
            <span className="text-ink-muted">
              Scanning, the charts, file lists, large files and duplicate finding stay free.
            </span>
          </li>
        </ul>

        {!configured ? (
          <Notice tone="warn" title="Purchases are not set up in this build">
            Checkout is unavailable until the merchant builds Diskora with a Dodo Payments product.
          </Notice>
        ) : null}

        {active && pro?.licenseKey ? (
          <p className="text-[11px] text-ink-muted">
            License key <span className="font-mono text-ink">{pro.licenseKey}</span>
            {pro.lastValidatedAt ? <> · verified {formatDateTime(pro.lastValidatedAt)}</> : null}
          </p>
        ) : null}

        {!active && pro && pro.state !== 'not-purchased' && pro.state !== 'not-configured' ? (
          <Notice
            tone={pro.state === 'invalid' ? 'danger' : 'warn'}
            title={pro.state === 'invalid' ? 'This license is no longer valid' : 'Pro needs a fresh check'}
          >
            {pro.message}
          </Notice>
        ) : null}

        {error ? (
          <p role="alert" className="text-[12px] leading-relaxed text-danger">
            {error}
          </p>
        ) : null}

        {configured && !active ? (
          <div className="space-y-2 border-t border-line pt-3">
            <Button
              variant="primary"
              className="w-full"
              onClick={() => void checkout()}
              loading={busy === 'checkout'}
              data-testid="upgrade-checkout"
            >
              <CreditCard aria-hidden className="size-3.5" />
              Buy Pro — {PRICE}
            </Button>
            <p className="text-[10px] text-ink-faint">
              Checkout opens in your browser. Your license key arrives by email and on the success page.
            </p>
            <div className="flex flex-col gap-1.5 sm:flex-row sm:items-end">
              <div className="min-w-0 flex-1">
                <TextField
                  label="Already have a license key?"
                  placeholder="Paste the key from your email"
                  value={key}
                  onChange={(event) => setKey(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') void activate();
                  }}
                  data-testid="upgrade-license-key"
                />
              </div>
              <Button
                variant="secondary"
                onClick={() => void activate()}
                disabled={!key.trim() || busy !== null}
                loading={busy === 'activate'}
              >
                Activate
              </Button>
            </div>
            <Button size="sm" variant="ghost" onClick={() => void restore()} disabled={busy !== null}>
              <RotateCcw aria-hidden className="size-3" />
              {busy === 'restore' ? 'Restoring…' : 'Restore purchases'}
            </Button>
          </div>
        ) : null}
      </div>
    </Modal>
  );
}

function describe(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'The operation could not be completed.';
}
