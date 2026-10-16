/**
 * Reusable UI primitives.
 *
 * Every visual decision (colour, radius, border, shadow, spacing) comes from the design tokens
 * in `src/styles.css`. Components never hard-code colours — that is what keeps the light and
 * dark themes, and the 100/125/150% DPI scalings, consistent.
 */

import {
  forwardRef,
  useId,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
} from 'react';
import { AlertTriangle, Check, ChevronDown, Info, Loader2, ShieldAlert, X } from 'lucide-react';

export function cx(...values: Array<string | false | null | undefined>): string {
  return values.filter(Boolean).join(' ');
}

/* ------------------------------------------------------------------ *
 * Buttons
 * ------------------------------------------------------------------ */

type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
type ButtonSize = 'sm' | 'md';

const BUTTON_BASE =
  'inline-flex items-center justify-center gap-1.5 rounded-control font-medium whitespace-nowrap ' +
  'transition-colors disabled:cursor-not-allowed disabled:opacity-45 select-none';

const BUTTON_VARIANTS: Record<ButtonVariant, string> = {
  primary: 'bg-accent text-accent-ink hover:bg-accent-hover shadow-panel',
  secondary:
    'bg-surface text-ink border border-line hover:border-line-strong hover:bg-surface-sunken shadow-panel',
  ghost: 'text-ink-muted hover:text-ink hover:bg-surface-sunken',
  danger: 'bg-danger text-white hover:brightness-110 shadow-panel',
};

const BUTTON_SIZES: Record<ButtonSize, string> = {
  sm: 'h-7 px-2.5 text-xs',
  md: 'h-8 px-3 text-[13px]',
};

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = 'secondary', size = 'md', loading = false, className, children, disabled, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type="button"
      disabled={disabled || loading}
      className={cx(BUTTON_BASE, BUTTON_VARIANTS[variant], BUTTON_SIZES[size], className)}
      {...rest}
    >
      {loading ? <Loader2 aria-hidden className="size-3.5 animate-spin" /> : null}
      {children}
    </button>
  );
});

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  variant?: ButtonVariant;
  size?: ButtonSize;
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, variant = 'ghost', size = 'sm', className, children, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type="button"
      aria-label={label}
      title={label}
      className={cx(
        BUTTON_BASE,
        BUTTON_VARIANTS[variant],
        size === 'sm' ? 'size-7' : 'size-8',
        'px-0',
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
});

/* ------------------------------------------------------------------ *
 * Surfaces
 * ------------------------------------------------------------------ */

export function Panel({
  className,
  children,
  ...rest
}: { className?: string; children: ReactNode } & React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cx(
        'rounded-panel border border-line bg-surface shadow-panel',
        className,
      )}
      {...rest}
    >
      {children}
    </div>
  );
}

export function SectionTitle({
  children,
  action,
  className,
}: {
  children: ReactNode;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cx('flex items-center justify-between gap-2 px-3 pt-3 pb-1.5', className)}>
      <h2 className="text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-faint">{children}</h2>
      {action}
    </div>
  );
}

export function Divider({ className }: { className?: string }) {
  return <div className={cx('h-px bg-line', className)} role="separator" />;
}

/* ------------------------------------------------------------------ *
 * Badges & chips
 * ------------------------------------------------------------------ */

type Tone = 'neutral' | 'info' | 'ok' | 'warn' | 'danger' | 'accent';

const TONES: Record<Tone, string> = {
  neutral: 'bg-surface-sunken text-ink-muted border-line',
  info: 'bg-info-soft text-info border-info/25',
  ok: 'bg-ok-soft text-ok border-ok/25',
  warn: 'bg-warn-soft text-warn border-warn/25',
  danger: 'bg-danger-soft text-danger border-danger/25',
  accent: 'bg-accent text-accent-ink border-transparent',
};

export function Badge({
  tone = 'neutral',
  children,
  className,
  dot,
}: {
  tone?: Tone;
  children: ReactNode;
  className?: string;
  dot?: string;
}) {
  return (
    <span
      className={cx(
        'inline-flex items-center gap-1 rounded-full border px-1.5 py-px text-[10px] font-medium leading-4',
        TONES[tone],
        className,
      )}
    >
      {dot ? <span aria-hidden className="size-1.5 rounded-full" style={{ background: dot }} /> : null}
      {children}
    </span>
  );
}

/** A small colour swatch + label used in legends and category rows. */
export function CategoryChip({ color, label }: { color: string; label: string }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-[11px] text-ink-muted">
      <span aria-hidden className="size-2.5 rounded-sm ring-1 ring-black/5" style={{ background: color }} />
      {label}
    </span>
  );
}

/* ------------------------------------------------------------------ *
 * Progress & feedback
 * ------------------------------------------------------------------ */

export function ProgressBar({
  value,
  tone = 'accent',
  className,
  label,
}: {
  /** 0..1 */
  value: number;
  tone?: 'accent' | 'danger' | 'ok';
  className?: string;
  label?: string;
}) {
  const pct = Math.max(0, Math.min(1, Number.isFinite(value) ? value : 0)) * 100;
  const fill = tone === 'danger' ? 'bg-danger' : tone === 'ok' ? 'bg-ok' : 'bg-accent';
  return (
    <div
      className={cx('h-1.5 w-full overflow-hidden rounded-full bg-surface-sunken', className)}
      role="progressbar"
      aria-valuenow={Math.round(pct)}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-label={label}
    >
      <div className={cx('h-full rounded-full transition-[width] duration-200', fill)} style={{ width: `${pct}%` }} />
    </div>
  );
}

export function Spinner({ className, label }: { className?: string; label?: string }) {
  return (
    <span role="status" aria-live="polite" className={cx('inline-flex items-center gap-2 text-ink-muted', className)}>
      <Loader2 aria-hidden className="size-4 animate-spin" />
      {label ? <span className="text-xs">{label}</span> : null}
    </span>
  );
}

export function Skeleton({ className }: { className?: string }) {
  return <div aria-hidden className={cx('animate-pulse rounded bg-surface-sunken', className)} />;
}

/* ------------------------------------------------------------------ *
 * Empty / error / loading states
 * ------------------------------------------------------------------ */

export function EmptyState({
  icon,
  title,
  detail,
  action,
  className,
}: {
  icon?: ReactNode;
  title: string;
  detail?: string;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cx(
        'flex h-full min-h-32 flex-col items-center justify-center gap-2 px-6 py-10 text-center',
        className,
      )}
    >
      {icon ? <div className="text-ink-faint">{icon}</div> : null}
      <p className="text-[13px] font-medium text-ink">{title}</p>
      {detail ? <p className="max-w-sm text-xs leading-relaxed text-ink-muted">{detail}</p> : null}
      {action ? <div className="mt-1">{action}</div> : null}
    </div>
  );
}

export function ErrorState({
  title = 'Something went wrong',
  detail,
  onRetry,
  className,
}: {
  title?: string;
  detail?: string;
  onRetry?: () => void;
  className?: string;
}) {
  return (
    <div
      role="alert"
      className={cx('flex h-full min-h-32 flex-col items-center justify-center gap-2 px-6 py-10 text-center', className)}
    >
      <AlertTriangle aria-hidden className="size-5 text-danger" />
      <p className="text-[13px] font-medium text-ink">{title}</p>
      {detail ? <p className="max-w-md break-words text-xs leading-relaxed text-ink-muted">{detail}</p> : null}
      {onRetry ? (
        <Button size="sm" onClick={onRetry} className="mt-1">
          Try again
        </Button>
      ) : null}
    </div>
  );
}

export function Notice({
  tone = 'info',
  title,
  children,
  className,
}: {
  tone?: 'info' | 'warn' | 'ok' | 'danger';
  title?: string;
  children?: ReactNode;
  className?: string;
}) {
  const Icon = tone === 'warn' ? AlertTriangle : tone === 'ok' ? Check : tone === 'danger' ? ShieldAlert : Info;
  const color =
    tone === 'warn' ? 'text-warn' : tone === 'ok' ? 'text-ok' : tone === 'danger' ? 'text-danger' : 'text-info';
  return (
    <div
      className={cx(
        'flex items-start gap-2 rounded-control border border-line bg-surface-sunken px-2.5 py-2 text-xs text-ink-muted',
        className,
      )}
    >
      <Icon aria-hidden className={cx('mt-px size-3.5 shrink-0', color)} />
      <div className="min-w-0">
        {title ? <p className="font-medium text-ink">{title}</p> : null}
        {children ? <div className="leading-relaxed">{children}</div> : null}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ *
 * Controls
 * ------------------------------------------------------------------ */

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
  title?: string;
}

export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
  label,
  className,
  size = 'md',
}: {
  options: Array<SegmentedOption<T>>;
  value: T;
  onChange: (value: T) => void;
  label: string;
  className?: string;
  size?: 'sm' | 'md';
}) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={cx(
        'inline-flex items-center gap-0.5 rounded-control border border-line bg-surface-sunken p-0.5',
        className,
      )}
    >
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={active}
            title={option.title ?? option.label}
            onClick={() => onChange(option.value)}
            className={cx(
              'inline-flex items-center gap-1 rounded-[6px] font-medium transition-colors',
              size === 'sm' ? 'h-6 px-2 text-[11px]' : 'h-7 px-2.5 text-xs',
              active
                ? 'bg-surface text-ink shadow-panel'
                : 'text-ink-muted hover:text-ink',
            )}
          >
            {option.icon}
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

export function TextField({
  label,
  hint,
  error,
  className,
  id,
  ...rest
}: { label?: string; hint?: string; error?: string } & InputHTMLAttributes<HTMLInputElement>) {
  const generated = useId();
  const inputId = id ?? generated;
  return (
    <label htmlFor={inputId} className="block">
      {label ? <span className="mb-1 block text-[11px] font-medium text-ink-muted">{label}</span> : null}
      <input
        id={inputId}
        className={cx(
          'h-8 w-full rounded-control border bg-surface px-2.5 text-[13px] text-ink placeholder:text-ink-faint',
          'focus:border-info focus:outline-none',
          error ? 'border-danger' : 'border-line',
          className,
        )}
        aria-invalid={error ? true : undefined}
        {...rest}
      />
      {hint && !error ? <span className="mt-1 block text-[11px] text-ink-faint">{hint}</span> : null}
      {error ? (
        <span className="mt-1 block text-[11px] text-danger" role="alert">
          {error}
        </span>
      ) : null}
    </label>
  );
}

export function Select({
  label,
  className,
  id,
  children,
  ...rest
}: { label?: string } & SelectHTMLAttributes<HTMLSelectElement>) {
  const generated = useId();
  const selectId = id ?? generated;
  return (
    <label htmlFor={selectId} className="block">
      {label ? <span className="mb-1 block text-[11px] font-medium text-ink-muted">{label}</span> : null}
      <span className="relative block">
        <select
          id={selectId}
          className={cx(
            'h-8 w-full appearance-none rounded-control border border-line bg-surface pl-2.5 pr-7 text-[13px] text-ink',
            'focus:border-info focus:outline-none',
            className,
          )}
          {...rest}
        >
          {children}
        </select>
        <ChevronDown
          aria-hidden
          className="pointer-events-none absolute right-2 top-1/2 size-3.5 -translate-y-1/2 text-ink-faint"
        />
      </span>
    </label>
  );
}

export function Checkbox({
  checked,
  onChange,
  label,
  detail,
  disabled,
  className,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  detail?: ReactNode;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <label
      className={cx(
        'flex cursor-pointer items-start gap-2 text-[13px] text-ink',
        disabled && 'cursor-not-allowed opacity-50',
        className,
      )}
    >
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
        className="mt-0.5 size-3.5 shrink-0 accent-[var(--color-accent)]"
      />
      <span className="min-w-0">
        <span className="block leading-snug">{label}</span>
        {detail ? <span className="mt-0.5 block text-[11px] leading-relaxed text-ink-muted">{detail}</span> : null}
      </span>
    </label>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  detail,
  disabled,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  detail?: ReactNode;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-start justify-between gap-3">
      <span className="min-w-0">
        <span className="block text-[13px] leading-snug text-ink">{label}</span>
        {detail ? <span className="mt-0.5 block text-[11px] leading-relaxed text-ink-muted">{detail}</span> : null}
      </span>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={typeof label === 'string' ? label : undefined}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={cx(
          'relative mt-0.5 h-4 w-7 shrink-0 rounded-full border transition-colors disabled:opacity-50',
          checked ? 'border-accent bg-accent' : 'border-line-strong bg-surface-sunken',
        )}
      >
        <span
          className={cx(
            'absolute top-0.5 size-2.5 rounded-full transition-all',
            checked ? 'left-3.5 bg-accent-ink' : 'left-0.5 bg-ink-faint',
          )}
        />
      </button>
    </div>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="rounded border border-line bg-surface-sunken px-1 py-px font-sans text-[10px] text-ink-muted">
      {children}
    </kbd>
  );
}

export function CloseButton({ onClick, label = 'Close' }: { onClick: () => void; label?: string }) {
  return (
    <IconButton label={label} onClick={onClick}>
      <X aria-hidden className="size-3.5" />
    </IconButton>
  );
}
