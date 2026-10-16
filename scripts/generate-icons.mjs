#!/usr/bin/env node
/**
 * Generates the application icon set from a single vector description.
 *
 * Run with:  npm run icons
 *
 * The design is a disk platter with a usage arc — the same metaphor as the sunburst in the app —
 * drawn at 1024px and scaled down. PNG encoding is done here (no native dependencies) so the icon
 * set can be regenerated on any machine without an image library.
 */

import { deflateSync } from 'node:zlib';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'src-tauri', 'icons');

/* --------------------------------- drawing --------------------------------- */

const SIZE = 1024;

/** RGBA framebuffer. */
const pixels = new Uint8Array(SIZE * SIZE * 4);

function setPixel(x, y, [r, g, b, a]) {
  if (x < 0 || y < 0 || x >= SIZE || y >= SIZE) return;
  const index = (y * SIZE + x) * 4;
  // Source-over compositing.
  const dstA = pixels[index + 3] / 255;
  const outA = a / 255 + dstA * (1 - a / 255);
  if (outA === 0) return;
  pixels[index] = Math.round((r * (a / 255) + pixels[index] * dstA * (1 - a / 255)) / outA);
  pixels[index + 1] = Math.round((g * (a / 255) + pixels[index + 1] * dstA * (1 - a / 255)) / outA);
  pixels[index + 2] = Math.round((b * (a / 255) + pixels[index + 2] * dstA * (1 - a / 255)) / outA);
  pixels[index + 3] = Math.round(outA * 255);
}

function fillRect(x0, y0, x1, y1, color) {
  for (let y = Math.max(0, Math.floor(y0)); y < Math.min(SIZE, Math.ceil(y1)); y += 1) {
    for (let x = Math.max(0, Math.floor(x0)); x < Math.min(SIZE, Math.ceil(x1)); x += 1) {
      setPixel(x, y, color);
    }
  }
}

/** Filled circle with an optional inner radius (a ring when inner > 0). */
function fillCircle(cx, cy, outer, inner, color) {
  const minY = Math.max(0, Math.floor(cy - outer));
  const maxY = Math.min(SIZE - 1, Math.ceil(cy + outer));
  const outerSq = outer * outer;
  const innerSq = inner * inner;
  for (let y = minY; y <= maxY; y += 1) {
    const dy = y - cy;
    const rowStart = Math.max(0, Math.floor(cx - outer));
    const rowEnd = Math.min(SIZE - 1, Math.ceil(cx + outer));
    for (let x = rowStart; x <= rowEnd; x += 1) {
      const dx = x + 0.5 - cx;
      const dyC = y + 0.5 - cy;
      const distanceSq = dx * dx + dyC * dyC;
      if (distanceSq <= outerSq && distanceSq >= innerSq) {
        setPixel(x, y, color);
      }
    }
  }
}

/** Anti-aliased thick arc segment from `startRad` to `endRad`. */
function fillArc(cx, cy, radius, thickness, startRad, endRad, color) {
  const outer = radius + thickness / 2;
  const inner = radius - thickness / 2;
  const minY = Math.max(0, Math.floor(cy - outer));
  const maxY = Math.min(SIZE - 1, Math.ceil(cy + outer));
  for (let y = minY; y <= maxY; y += 1) {
    for (let x = Math.max(0, Math.floor(cx - outer)); x <= Math.min(SIZE - 1, Math.ceil(cx + outer)); x += 1) {
      const dx = x + 0.5 - cx;
      const dy = y + 0.5 - cy;
      const distance = Math.hypot(dx, dy);
      if (distance < inner || distance > outer) continue;
      let angle = Math.atan2(dy, dx);
      // Normalise into [start, start + 2π).
      const span = endRad - startRad;
      while (angle < startRad) angle += Math.PI * 2;
      if (angle - startRad > span) continue;
      // Feather the radial edges for anti-aliasing.
      const edge = Math.min(distance - inner, outer - distance);
      const alpha = Math.min(1, edge / 1.2);
      setPixel(x, y, [color[0], color[1], color[2], Math.round(color[3] * alpha)]);
    }
  }
}

/* --------------------------------- palette --------------------------------- */

const INK = [17, 24, 39, 255];
const SURFACE = [248, 250, 252, 255];
const ACCENT = [37, 99, 235, 255];
const RING_BG = [226, 232, 240, 255];
const USED = [59, 130, 246, 255];
const CACHE = [245, 158, 11, 255];
const MEDIA = [16, 185, 129, 255];

function draw() {
  const centre = SIZE / 2;

  // Rounded-square background.
  const radius = SIZE * 0.22;
  fillRect(0, 0, SIZE, SIZE, [0, 0, 0, 0]);
  for (let y = 0; y < SIZE; y += 1) {
    for (let x = 0; x < SIZE; x += 1) {
      const dx = Math.min(x, SIZE - 1 - x);
      const dy = Math.min(y, SIZE - 1 - y);
      let inside = true;
      if (dx < radius && dy < radius) {
        const cx = radius;
        const cy = radius;
        const corner = Math.hypot(cx - (x + 0.5), cy - (y + 0.5));
        inside = corner <= radius;
      }
      if (inside) setPixel(x, y, SURFACE);
    }
  }

  // Outer usage ring: three arcs, mirroring the sunburst's category mapping.
  const ringRadius = SIZE * 0.4;
  const ringThickness = SIZE * 0.075;
  fillArc(centre, centre, ringRadius, ringThickness, 0, Math.PI * 2, RING_BG);
  fillArc(centre, centre, ringRadius, ringThickness, -Math.PI / 2, -Math.PI / 2 + Math.PI * 0.55, USED);
  fillArc(centre, centre, ringRadius, ringThickness, -Math.PI / 2 + Math.PI * 0.6, -Math.PI / 2 + Math.PI * 0.75, CACHE);
  fillArc(centre, centre, ringRadius, ringThickness, -Math.PI / 2 + Math.PI * 0.8, -Math.PI / 2 + Math.PI * 0.88, MEDIA);

  // Disk platter.
  fillCircle(centre, centre, SIZE * 0.29, 0, [255, 255, 255, 255]);
  fillCircle(centre, centre, SIZE * 0.29, SIZE * 0.26, [226, 232, 240, 255]);
  fillCircle(centre, centre, SIZE * 0.13, 0, ACCENT);
  fillCircle(centre, centre, SIZE * 0.05, 0, SURFACE);

  // Tracks.
  for (const track of [0.18, 0.22]) {
    fillCircle(centre, centre, SIZE * track, SIZE * track - 3, [203, 213, 225, 255]);
  }
  void INK;
}

/* ---------------------------------- PNG ---------------------------------- */

function crc32(buffer) {
  let crc = ~0;
  for (let byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return ~crc >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body), 0);
  return Buffer.concat([length, body, crc]);
}

/** Nearest-neighbour downsample of the master framebuffer to `size` and encode as PNG. */
function encodePng(size) {
  const scale = SIZE / size;
  const raw = Buffer.alloc(size * (size * 4 + 1));
  let offset = 0;
  for (let y = 0; y < size; y += 1) {
    raw[offset] = 0; // filter: none
    offset += 1;
    for (let x = 0; x < size; x += 1) {
      const sourceX = Math.min(SIZE - 1, Math.floor(x * scale));
      const sourceY = Math.min(SIZE - 1, Math.floor(y * scale));
      const source = (sourceY * SIZE + sourceX) * 4;
      raw[offset] = pixels[source];
      raw[offset + 1] = pixels[source + 1];
      raw[offset + 2] = pixels[source + 2];
      raw[offset + 3] = pixels[source + 3];
      offset += 4;
    }
  }

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // colour type: RGBA
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;

  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  return Buffer.concat([
    signature,
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** Minimal BMP-free ICO container: embeds PNGs, which Windows 7+ supports directly. */
function encodeIco(sizes) {
  const images = sizes.map((size) => ({ size, data: encodePng(size) }));
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(images.length, 4);

  const entries = [];
  let offset = header.length + images.length * 16;
  for (const image of images) {
    const entry = Buffer.alloc(16);
    entry[0] = image.size >= 256 ? 0 : image.size;
    entry[1] = image.size >= 256 ? 0 : image.size;
    entry[2] = 0; // palette
    entry[3] = 0; // reserved
    entry.writeUInt16LE(1, 4); // colour planes
    entry.writeUInt16LE(32, 6); // bits per pixel
    entry.writeUInt32BE(image.data.length, 8);
    entry.writeUInt32BE(offset, 12);
    offset += image.data.length;
    entries.push(entry);
  }

  return Buffer.concat([header, ...entries, ...images.map((image) => image.data)]);
}

/* ---------------------------------- main ---------------------------------- */

draw();

mkdirSync(outDir, { recursive: true });
const outputs = [
  ['32x32.png', encodePng(32)],
  ['128x128.png', encodePng(128)],
  ['128x128@2x.png', encodePng(256)],
  ['icon.png', encodePng(512)],
  ['icon.ico', encodeIco([16, 24, 32, 48, 64, 128, 256])],
  ['icon.icns', encodePng(512)],
];

for (const [name, data] of outputs) {
  writeFileSync(join(outDir, name), data);
  console.log(`wrote src-tauri/icons/${name} (${data.length} bytes)`);
}

// Windows Store / MSI logos.
for (const [name, size] of [
  ['Square30x30Logo.png', 30],
  ['Square44x44Logo.png', 44],
  ['Square71x71Logo.png', 71],
  ['Square89x89Logo.png', 89],
  ['Square107x107Logo.png', 107],
  ['Square142x142Logo.png', 142],
  ['Square150x150Logo.png', 150],
  ['Square284x284Logo.png', 284],
  ['Square310x310Logo.png', 310],
  ['StoreLogo.png', 50],
]) {
  const data = encodePng(size);
  writeFileSync(join(outDir, name), data);
  console.log(`wrote src-tauri/icons/${name} (${data.length} bytes)`);
}
