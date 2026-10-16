/**
 * Storage categories — the vocabulary the whole app uses to talk about *what* is using space.
 *
 * The category for a file is decided by the Rust backend (`src-tauri/src/filesystem/category.rs`)
 * so scans, the visualization, large-file finder and cleanup assistant all agree. This module
 * only owns presentation: labels, colours and ordering.
 *
 * `CATEGORY_COLORS` is the single source of truth for the palette; `categories.test.ts` asserts
 * that it stays in sync with the `--color-cat-*` tokens in `src/styles.css`.
 */

import type { StorageCategory } from '@/types/contracts';

export interface CategoryMeta {
  label: string;
  /** Muted pastel used in the sunburst, treemap, legend and badges. */
  color: string;
  /** One-line explanation shown in tooltips and the cleanup assistant. */
  description: string;
}

export const CATEGORY_COLORS: Record<StorageCategory, string> = {
  system: '#c9c2b6',
  applications: '#a0c0d9',
  games: '#c8a0d9',
  documents: '#a0d9a0',
  media: '#d9a0ac',
  development: '#d9d2a0',
  downloads: '#d9bfa0',
  cache: '#a0d2d9',
  archives: '#bfd9a0',
  other: '#cbc7be',
  free: '#e6e2da',
};

export const CATEGORY_META: Record<StorageCategory, CategoryMeta> = {
  system: {
    label: 'System',
    color: CATEGORY_COLORS.system,
    description: 'Windows, drivers and installed OS components.',
  },
  applications: {
    label: 'Applications',
    color: CATEGORY_COLORS.applications,
    description: 'Installed programs and their bundled resources.',
  },
  games: {
    label: 'Games',
    color: CATEGORY_COLORS.games,
    description: 'Game installs, assets and launchers.',
  },
  documents: {
    label: 'Documents',
    color: CATEGORY_COLORS.documents,
    description: 'Documents, spreadsheets, PDFs and personal files.',
  },
  media: {
    label: 'Media',
    color: CATEGORY_COLORS.media,
    description: 'Photos, video and audio.',
  },
  development: {
    label: 'Development',
    color: CATEGORY_COLORS.development,
    description: 'Source control, build output and dependencies.',
  },
  downloads: {
    label: 'Downloads',
    color: CATEGORY_COLORS.downloads,
    description: 'Files in your Downloads folder.',
  },
  cache: {
    label: 'Caches',
    color: CATEGORY_COLORS.cache,
    description: 'Temporary data an app can regenerate.',
  },
  archives: {
    label: 'Archives',
    color: CATEGORY_COLORS.archives,
    description: 'ZIP, ISO, disk images and other compressed data.',
  },
  other: {
    label: 'Other',
    color: CATEGORY_COLORS.other,
    description: 'Files Diskwise could not classify.',
  },
  free: {
    label: 'Free space',
    color: CATEGORY_COLORS.free,
    description: 'Unallocated space on the volume.',
  },
};

/** Categories that represent real content (used for legends and breakdown tables). */
export const CONTENT_CATEGORIES: StorageCategory[] = [
  'system',
  'applications',
  'games',
  'documents',
  'media',
  'development',
  'downloads',
  'cache',
  'archives',
  'other',
];

export function categoryLabel(category: StorageCategory): string {
  return CATEGORY_META[category]?.label ?? 'Unknown';
}

export function categoryColor(category: StorageCategory): string {
  return CATEGORY_COLORS[category] ?? CATEGORY_COLORS.other;
}

/**
 * Nudges a category colour lighter/darker so adjacent rings and siblings stay readable.
 * `amount` is roughly -1..1; positive lightens.
 */
export function shadeCategoryColor(category: StorageCategory, amount: number): string {
  const base = categoryColor(category);
  const factor = Math.max(-0.6, Math.min(0.6, amount));
  return mixHex(base, factor >= 0 ? '#ffffff' : '#3a352f', Math.abs(factor));
}

/** Linear blend between two hex colours. `t` = 0 → a, 1 → b. */
export function mixHex(a: string, b: string, t: number): string {
  const clamped = Math.max(0, Math.min(1, t));
  const pa = parseHex(a);
  const pb = parseHex(b);
  if (!pa || !pb) return a;
  const r = Math.round(pa[0] + (pb[0] - pa[0]) * clamped);
  const g = Math.round(pa[1] + (pb[1] - pa[1]) * clamped);
  const bl = Math.round(pa[2] + (pb[2] - pa[2]) * clamped);
  return `#${[r, g, bl].map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}

/** Relative luminance (WCAG). */
export function luminance(hex: string): number {
  const rgb = parseHex(hex);
  if (!rgb) return 0;
  const [r, g, b] = rgb.map((v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** Picks charcoal or white text for a given background. */
export function readableInk(hex: string): string {
  return luminance(hex) > 0.45 ? '#24211d' : '#fbfaf8';
}

function parseHex(hex: string): [number, number, number] | null {
  const match = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!match) return null;
  const value = Number.parseInt(match[1], 16);
  return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
}
