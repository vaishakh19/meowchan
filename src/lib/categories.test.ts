import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  CATEGORY_COLORS,
  CATEGORY_META,
  CONTENT_CATEGORIES,
  categoryColor,
  categoryLabel,
  luminance,
  mixHex,
  readableInk,
  shadeCategoryColor,
} from './categories';
import type { StorageCategory } from '@/types/contracts';

const css = readFileSync(resolve(process.cwd(), 'src/styles.css'), 'utf8');

/**
 * The palette lives in two places by necessity: the Tailwind `@theme` block (for utility
 * classes) and this module (for D3 fills). This test is the guard rail that keeps them equal.
 */
describe('category palette', () => {
  it('matches the --color-cat-* tokens in styles.css', () => {
    for (const [category, hex] of Object.entries(CATEGORY_COLORS)) {
      const pattern = new RegExp(`--color-cat-${category}:\\s*(${hex});`);
      expect(css).toMatch(pattern);
    }
  });

  it('has metadata for every category', () => {
    for (const category of Object.keys(CATEGORY_COLORS) as StorageCategory[]) {
      expect(CATEGORY_META[category]).toBeDefined();
      expect(CATEGORY_META[category].label.length).toBeGreaterThan(0);
      expect(CATEGORY_META[category].color).toBe(CATEGORY_COLORS[category]);
      expect(CATEGORY_META[category].description.length).toBeGreaterThan(0);
    }
  });

  it('lists every content category exactly once', () => {
    expect([...CONTENT_CATEGORIES].sort()).toEqual(
      (Object.keys(CATEGORY_COLORS) as StorageCategory[]).filter((c) => c !== 'free').sort(),
    );
  });

  it('uses distinct colours', () => {
    const values = Object.values(CATEGORY_COLORS);
    expect(new Set(values).size).toBe(values.length);
  });

  it('falls back to a safe colour for unknown categories', () => {
    expect(categoryColor('nonsense' as StorageCategory)).toBe(CATEGORY_COLORS.other);
    expect(categoryLabel('nonsense' as StorageCategory)).toBe('Unknown');
  });
});

describe('colour maths', () => {
  it('mixes towards a target colour', () => {
    expect(mixHex('#000000', '#ffffff', 0)).toBe('#000000');
    expect(mixHex('#000000', '#ffffff', 1)).toBe('#ffffff');
    expect(mixHex('#000000', '#ffffff', 0.5)).toBe('#808080');
  });

  it('clamps the mix factor', () => {
    expect(mixHex('#000000', '#ffffff', -5)).toBe('#000000');
    expect(mixHex('#000000', '#ffffff', 5)).toBe('#ffffff');
  });

  it('computes relative luminance', () => {
    expect(luminance('#ffffff')).toBeCloseTo(1, 5);
    expect(luminance('#000000')).toBeCloseTo(0, 5);
  });

  it('picks readable ink for a background', () => {
    expect(readableInk('#ffffff')).toBe('#24211d');
    expect(readableInk('#000000')).toBe('#fbfaf8');
  });

  it('shades category colours for depth variation', () => {
    const base = CATEGORY_COLORS.media;
    const lighter = shadeCategoryColor('media', 0.3);
    const darker = shadeCategoryColor('media', -0.3);
    expect(lighter).not.toBe(base);
    expect(darker).not.toBe(base);
    expect(luminance(lighter)).toBeGreaterThan(luminance(base));
    expect(luminance(darker)).toBeLessThan(luminance(base));
  });
});
