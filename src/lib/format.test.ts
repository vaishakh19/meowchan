import { describe, expect, it } from 'vitest';
import {
  clamp,
  formatBytes,
  formatBytesCompact,
  formatBytesExact,
  formatCount,
  formatCountExact,
  formatDateTime,
  formatDuration,
  formatPercent,
  formatRelativeTime,
  parseBytes,
  sumBytes,
} from './format';

const KB = 1024;
const MB = 1024 * KB;
const GB = 1024 * MB;
const TB = 1024 * GB;

describe('formatBytes', () => {
  it('uses 1024-based units with Windows-style labels', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(1)).toBe('1 B');
    expect(formatBytes(1023)).toBe('1023 B');
    expect(formatBytes(KB)).toBe('1 KB');
    expect(formatBytes(1536 * MB)).toBe('1.5 GB');
    expect(formatBytes(2 * TB)).toBe('2 TB');
  });

  it('trims trailing zeros', () => {
    expect(formatBytes(1024 * MB)).toBe('1 GB');
    expect(formatBytes(2.5 * GB)).toBe('2.5 GB');
    expect(formatBytes(100 * MB)).toBe('100 MB');
  });

  it('handles null, undefined and invalid input', () => {
    expect(formatBytes(null)).toBe('—');
    expect(formatBytes(undefined)).toBe('—');
    expect(formatBytes(Number.NaN)).toBe('—');
    expect(formatBytes(Number.POSITIVE_INFINITY)).toBe('—');
  });

  it('handles negative values', () => {
    expect(formatBytes(-2048)).toBe('-2 KB');
  });

  it('respects the precision and space options', () => {
    expect(formatBytes(1536 * MB, { precision: 3 })).toBe('1.500 GB');
    expect(formatBytes(1536 * MB, { space: false })).toBe('1.5GB');
    expect(formatBytes(null, { nullPlaceholder: 'n/a' })).toBe('n/a');
  });

  it('formats compactly', () => {
    expect(formatBytesCompact(0)).toBe('0B');
    expect(formatBytesCompact(512)).toBe('512B');
    expect(formatBytesCompact(12.4 * GB)).toBe('12.4G');
    expect(formatBytesCompact(null)).toBe('—');
  });

  it('formats exact byte counts', () => {
    expect(formatBytesExact(1234567)).toBe('1,234,567 B');
    expect(formatBytesExact(null)).toBe('—');
  });
});

describe('formatCount', () => {
  it('abbreviates large counts', () => {
    expect(formatCount(0)).toBe('0');
    expect(formatCount(999)).toBe('999');
    expect(formatCount(1500)).toBe('1.5K');
    expect(formatCount(15_000)).toBe('15K');
    expect(formatCount(2_400_000)).toBe('2.4M');
    expect(formatCountExact(2400000)).toBe('2,400,000');
  });
});

describe('formatDuration', () => {
  it('scales the unit', () => {
    expect(formatDuration(420)).toBe('420ms');
    expect(formatDuration(1500)).toBe('2s');
    expect(formatDuration(65_000)).toBe('1m 05s');
    expect(formatDuration(3_780_000)).toBe('1h 03m');
    expect(formatDuration(-1)).toBe('—');
    expect(formatDuration(null)).toBe('—');
  });
});

describe('formatDateTime / formatRelativeTime', () => {
  const ts = Date.UTC(2026, 2, 12, 14, 5);

  it('formats a stable date time', () => {
    expect(formatDateTime(ts)).toMatch(/12 Mar 2026/);
    expect(formatDateTime(null)).toBe('—');
  });

  it('formats relative times', () => {
    const now = Date.UTC(2026, 2, 15, 14, 5);
    expect(formatRelativeTime(now, now)).toBe('just now');
    expect(formatRelativeTime(now - 3 * 86_400_000, now)).toMatch(/3 days ago/);
    expect(formatRelativeTime(now + 2 * 3_600_000, now)).toMatch(/in 2 hours/);
    expect(formatRelativeTime(null)).toBe('—');
  });
});

describe('formatPercent', () => {
  it('formats fractions as percentages', () => {
    expect(formatPercent(0.4237)).toBe('42%');
    expect(formatPercent(0.4237, 1)).toBe('42.4%');
    expect(formatPercent(null)).toBe('—');
  });
});

describe('parseBytes', () => {
  it('parses human sizes', () => {
    expect(parseBytes('500MB')).toBe(500 * MB);
    expect(parseBytes('1.5 gb')).toBe(Math.round(1.5 * GB));
    expect(parseBytes('1024')).toBe(1024);
    expect(parseBytes('2 GB')).toBe(2 * GB);
    expect(parseBytes('10KiB')).toBe(10 * KB);
  });

  it('rejects nonsense', () => {
    expect(parseBytes('')).toBeNull();
    expect(parseBytes('abc')).toBeNull();
    expect(parseBytes('-5MB')).toBeNull();
    expect(parseBytes('12 PB')).toBeNull();
  });

  it('round-trips with formatBytes', () => {
    for (const text of ['1 KB', '10 MB', '1.5 GB', '2 TB']) {
      expect(formatBytes(parseBytes(text)!)).toBe(text);
    }
  });
});

describe('misc', () => {
  it('clamps', () => {
    expect(clamp(5, 0, 10)).toBe(5);
    expect(clamp(-1, 0, 10)).toBe(0);
    expect(clamp(11, 0, 10)).toBe(10);
    expect(clamp(Number.NaN, 3, 10)).toBe(3);
  });

  it('sums safely', () => {
    expect(sumBytes([1, 2, Number.NaN, 4])).toBe(7);
    expect(sumBytes([])).toBe(0);
  });
});
