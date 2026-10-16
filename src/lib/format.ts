/**
 * Formatting helpers. Every user-visible number in Diskwise goes through here so units,
 * precision and locale stay consistent.
 *
 * Units: Windows Explorer reports sizes in KB/MB/GB/TB computed on 1024-byte boundaries,
 * so we do the same and never claim "GiB".
 */

const KB = 1024;
const MB = KB * 1024;
const GB = MB * 1024;
const TB = GB * 1024;

export interface FormatBytesOptions {
  /** Number of significant digits. Defaults to 2 for values >= 10, 3 below. */
  precision?: number;
  /** Append a trailing space before the unit. Defaults to true. */
  space?: boolean;
  /** Return "—" for null. Defaults to true. */
  nullPlaceholder?: string;
}

const UNITS: Array<{ limit: number; divisor: number; label: string }> = [
  { limit: TB, divisor: TB, label: 'TB' },
  { limit: GB, divisor: GB, label: 'GB' },
  { limit: MB, divisor: MB, label: 'MB' },
  { limit: KB, divisor: KB, label: 'KB' },
];

/**
 * Formats a byte count using 1024-based units with Windows-style labels.
 * `formatBytes(0)` is `"0 B"` — never an empty string, never a dash.
 */
export function formatBytes(bytes: number | null | undefined, options: FormatBytesOptions = {}): string {
  const { precision, space = true, nullPlaceholder = '—' } = options;
  if (bytes === null || bytes === undefined || Number.isNaN(bytes)) return nullPlaceholder;
  if (!Number.isFinite(bytes)) return nullPlaceholder;
  if (bytes < 0) return `-${formatBytes(-bytes, options)}`;
  if (bytes === 0) return '0 B';
  if (bytes < KB) return `${Math.round(bytes)} B`;

  const unit = UNITS.find((u) => bytes >= u.limit) ?? UNITS[UNITS.length - 1];
  const value = bytes / unit.divisor;
  const digits = precision ?? (value >= 100 ? 0 : value >= 10 ? 1 : 2);
  // Trailing zeros are only trimmed when the caller did not ask for a fixed precision.
  const text =
    precision === undefined
      ? value.toFixed(digits).replace(/\.0+$/, '').replace(/(\.\d*?)0+$/, '$1')
      : value.toFixed(digits);
  return space ? `${text} ${unit.label}` : `${text}${unit.label}`;
}

/** Compact form for dense chrome: `12.4 GB` → `12.4G`. */
export function formatBytesCompact(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return '—';
  if (bytes === 0) return '0B';
  const abs = Math.abs(bytes);
  if (abs < KB) return `${Math.round(bytes)}B`;
  const unit = UNITS.find((u) => abs >= u.limit) ?? UNITS[UNITS.length - 1];
  const value = bytes / unit.divisor;
  const digits = abs / unit.divisor >= 10 ? 1 : 2;
  return `${Number(value.toFixed(digits))}${unit.label[0]}`;
}

/** Exact byte count with thousands separators. */
export function formatBytesExact(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return '—';
  return `${Math.round(bytes).toLocaleString('en-US')} B`;
}

/** `1234` → `1,234`; `1234567` → `1.2M`. */
export function formatCount(count: number | null | undefined): string {
  if (count === null || count === undefined || !Number.isFinite(count)) return '—';
  if (count < 1000) return String(Math.round(count));
  if (count < 1_000_000) return `${(count / 1000).toFixed(count < 10_000 ? 1 : 0)}K`;
  if (count < 1_000_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  return `${(count / 1_000_000_000).toFixed(1)}B`;
}

/** `1,234` */
export function formatCountExact(count: number | null | undefined): string {
  if (count === null || count === undefined || !Number.isFinite(count)) return '—';
  return Math.round(count).toLocaleString('en-US');
}

/** Elapsed milliseconds → `4m 12s`, `1h 03m`, `820ms`. */
export function formatDuration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms < 0) return '—';
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const totalSeconds = Math.round(ms / 1000);
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) return `${minutes}m ${String(seconds).padStart(2, '0')}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${String(minutes % 60).padStart(2, '0')}m`;
}

const DATE_TIME = new Intl.DateTimeFormat('en-GB', {
  year: 'numeric',
  month: 'short',
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  hour12: false,
});

const DATE_ONLY = new Intl.DateTimeFormat('en-GB', {
  year: 'numeric',
  month: 'short',
  day: '2-digit',
});

const TIME_ONLY = new Intl.DateTimeFormat('en-GB', {
  hour: '2-digit',
  minute: '2-digit',
  hour12: false,
});

/** Unix ms → `12 Mar 2026, 14:05`. */
export function formatDateTime(ts: number | null | undefined): string {
  if (ts === null || ts === undefined || !Number.isFinite(ts)) return '—';
  return DATE_TIME.format(new Date(ts)).replace(',', '');
}

export function formatDate(ts: number | null | undefined): string {
  if (ts === null || ts === undefined || !Number.isFinite(ts)) return '—';
  return DATE_ONLY.format(new Date(ts));
}

export function formatTime(ts: number | null | undefined): string {
  if (ts === null || ts === undefined || !Number.isFinite(ts)) return '—';
  return TIME_ONLY.format(new Date(ts));
}

const RTF = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });

/** Unix ms → `3 days ago`, `just now`, `in 2 months`. */
export function formatRelativeTime(ts: number | null | undefined, now: number = Date.now()): string {
  if (ts === null || ts === undefined || !Number.isFinite(ts)) return '—';
  const deltaSeconds = Math.round((ts - now) / 1000);
  const abs = Math.abs(deltaSeconds);
  if (abs < 45) return 'just now';
  if (abs < 3600) return RTF.format(Math.round(deltaSeconds / 60), 'minute');
  if (abs < 86_400) return RTF.format(Math.round(deltaSeconds / 3600), 'hour');
  if (abs < 2_592_000) return RTF.format(Math.round(deltaSeconds / 86_400), 'day');
  if (abs < 31_536_000) return RTF.format(Math.round(deltaSeconds / 2_592_000), 'month');
  return RTF.format(Math.round(deltaSeconds / 31_536_000), 'year');
}

/** `0.42` → `42%`. */
export function formatPercent(fraction: number | null | undefined, digits = 0): string {
  if (fraction === null || fraction === undefined || !Number.isFinite(fraction)) return '—';
  return `${(fraction * 100).toFixed(digits)}%`;
}

/**
 * Parses a user-entered size such as `500MB`, `1.5 gb`, `1024`, `2 GB`.
 * Returns `null` when the text is not a size. Binary units (1024).
 */
export function parseBytes(text: string): number | null {
  const trimmed = text.trim().toLowerCase();
  if (!trimmed) return null;
  const match = /^(\d+(?:[.,]\d+)?)\s*(b|kb|k|mb|m|gb|g|tb|t|kib|mib|gib|tib)?$/.exec(trimmed);
  if (!match) return null;
  const value = Number.parseFloat(match[1].replace(',', '.'));
  if (!Number.isFinite(value) || value < 0) return null;
  const unit = match[2] ?? 'b';
  const multipliers: Record<string, number> = {
    b: 1,
    kb: KB,
    k: KB,
    kib: KB,
    mb: MB,
    m: MB,
    mib: MB,
    gb: GB,
    g: GB,
    gib: GB,
    tb: TB,
    t: TB,
    tib: TB,
  };
  return Math.round(value * multipliers[unit]);
}

/** Clamps a number into `[min, max]`. */
export function clamp(value: number, min: number, max: number): number {
  if (Number.isNaN(value)) return min;
  return Math.min(Math.max(value, min), max);
}

/** Sums an iterable of numbers without losing precision on huge arrays. */
export function sumBytes(values: Iterable<number>): number {
  let total = 0;
  for (const value of values) {
    if (Number.isFinite(value)) total += value;
  }
  return total;
}
