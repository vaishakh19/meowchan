/**
 * Stable, collision-resistant identifiers derived from paths.
 *
 * `stablePathId` MUST stay byte-identical to `src-tauri/src/filesystem/validate.rs::stable_path_id`
 * (FNV-1a 32-bit over the UTF-8 bytes, formatted as 8 lowercase hex chars). Keeping them in sync
 * means a node keeps the same `id` across scans, which is what makes React keys, selection
 * restoration and the "focus in visualization" action reliable.
 */

const FNV_OFFSET = 0x811c9dc5;
const FNV_PRIME = 0x01000193;

export function stablePathId(path: string): string {
  let hash = FNV_OFFSET;
  for (let i = 0; i < path.length; i += 1) {
    hash ^= path.charCodeAt(i) & 0xff;
    hash = Math.imul(hash, FNV_PRIME) >>> 0;
  }
  // Fold in the high bytes of UTF-16 code units so non-ASCII paths differ.
  let extra = FNV_OFFSET;
  for (let i = 0; i < path.length; i += 1) {
    const code = path.charCodeAt(i);
    extra ^= (code >>> 8) & 0xff;
    extra = Math.imul(extra, FNV_PRIME) >>> 0;
  }
  return `${hash.toString(16).padStart(8, '0')}${extra.toString(16).padStart(8, '0')}`;
}

let counter = 0;

/** Monotonic id for client-side entities (toasts, draft cleanup rules, …). */
export function nextId(prefix = 'id'): string {
  counter += 1;
  return `${prefix}-${Date.now().toString(36)}-${counter.toString(36)}`;
}

/** Random, non-security-sensitive token (used for request correlation ids). */
export function randomToken(length = 12): string {
  const alphabet = 'abcdefghijklmnopqrstuvwxyz0123456789';
  let out = '';
  for (let i = 0; i < length; i += 1) {
    out += alphabet[Math.floor(Math.random() * alphabet.length)];
  }
  return out;
}
