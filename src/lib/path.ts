/**
 * Path helpers shared by the UI.
 *
 * These mirror `src-tauri/src/filesystem/validate.rs`. The backend is the authority — it
 * canonicalizes and re-validates every path before touching the filesystem — but the frontend
 * uses the same rules so obviously-bad input never reaches it and error messages are identical.
 *
 * Deliberately dependency-free and synchronous: it is unit-tested and also used by the
 * in-browser demo transport.
 */

/** `C:\`, `\\server\share\`, `/home/user` */
export function isAbsolutePath(p: string): boolean {
  if (!p) return false;
  if (/^[A-Za-z]:[\\/]/.test(p)) return true; // drive letter
  if (p.startsWith('\\\\')) return true; // UNC
  if (p.startsWith('/')) return true; // POSIX
  return false;
}

export function isUncPath(p: string): boolean {
  return p.startsWith('\\\\');
}

/** Strips surrounding quotes and whitespace that Windows shell copy/paste tends to add. */
export function stripQuotes(input: string): string {
  let out = input.trim();
  while (
    out.length > 1 &&
    ((out.startsWith('"') && out.endsWith('"')) || (out.startsWith("'") && out.endsWith("'")))
  ) {
    out = out.slice(1, -1).trim();
  }
  return out;
}

/** Splits into segments, dropping empty ones and resolving `.`/`..` lexically. */
export function pathSegments(p: string): string[] {
  const raw = stripQuotes(p).replace(/\//g, '\\').split('\\').filter((s) => s.length > 0);
  const out: string[] = [];
  for (const segment of raw) {
    if (segment === '.') continue;
    if (segment === '..') {
      if (out.length > 0) out.pop();
      continue;
    }
    out.push(segment);
  }
  return out;
}

/** Collapses runs of separators, preserving a leading UNC prefix. */
export function normalizeSeparators(p: string): string {
  const input = stripQuotes(p);
  const isUnc = input.startsWith('\\\\');
  const collapsed = input.replace(/\//g, '\\').replace(/\\{2,}/g, '\\');
  return isUnc ? `\\\\${collapsed.replace(/^\\+/, '')}` : collapsed;
}

/** Separator style used by the input (Windows `\` or POSIX `/`). */
function separatorFor(cleaned: string): string {
  if (/^[A-Za-z]:/.test(cleaned) || cleaned.startsWith('\\\\')) return '\\';
  if (cleaned.startsWith('/')) return '/';
  return '\\';
}

/** Full lexical normalization (no filesystem access). */
export function normalizePath(p: string): string {
  const cleaned = stripQuotes(p);
  if (!cleaned) return '';
  let segments = pathSegments(cleaned);
  const sep = separatorFor(cleaned);

  const driveMatch = /^([A-Za-z]):/.exec(cleaned);
  if (driveMatch) {
    const drive = driveMatch[1].toUpperCase();
    // `pathSegments` keeps the drive as its first segment (`C:`); drop it so it is not doubled.
    if (segments.length > 0 && segments[0].toLowerCase() === `${driveMatch[1].toLowerCase()}:`) {
      segments = segments.slice(1);
    }
    return segments.length > 0 ? `${drive}:\\${segments.join('\\')}` : `${drive}:\\`;
  }
  if (cleaned.startsWith('\\\\')) {
    return segments.length > 0 ? `\\\\${segments.join('\\')}` : '\\\\';
  }
  if (cleaned.startsWith('/')) {
    return `/${segments.join(sep)}`;
  }
  return segments.join(sep);
}

/** Removes trailing separators, collapsing `C:\Users\` → `C:\Users` (keeps `C:\`). */
export function stripTrailingSeparator(p: string): string {
  const normalized = normalizePath(p);
  if (!normalized) return normalized;
  if (/^[A-Za-z]:\\?$/.test(normalized) || /^\/[A-Za-z]:\\?$/.test(normalized)) return normalized;
  if (normalized === '\\' || normalized === '/') return normalized;
  return normalized.replace(/[\\/]+$/, '');
}

/** Final path component. */
export function baseName(p: string): string {
  const normalized = normalizeSeparators(stripQuotes(p)).replace(/[\\/]+$/, '');
  const idx = Math.max(normalized.lastIndexOf('\\'), normalized.lastIndexOf('/'));
  return idx < 0 ? normalized : normalized.slice(idx + 1);
}

/** Parent directory, or `null` at a filesystem root. */
export function parentPath(p: string): string | null {
  const normalized = normalizePath(p);
  if (!normalized) return null;
  if (/^[A-Za-z]:\\?$/.test(normalized)) return null; // drive root
  if (/^\\\\[^\\]+\\[^\\]+$/.test(normalized)) return null; // UNC share root
  if (normalized === '/') return null;
  const sep = normalized.startsWith('/') && !/^[A-Za-z]:/.test(normalized) ? '/' : '\\';
  const idx = normalized.lastIndexOf(sep);
  if (idx < 0) return null;
  const parent = normalized.slice(0, idx);
  if (parent === '') return sep;
  if (/^[A-Za-z]:$/.test(parent)) return `${parent}\\`;
  return parent;
}

/** Lower-case extension without the dot, or `null`. */
export function extensionOf(nameOrPath: string): string | null {
  const name = baseName(nameOrPath);
  const dot = name.lastIndexOf('.');
  if (dot <= 0 || dot === name.length - 1) return null;
  const ext = name.slice(dot + 1).toLowerCase();
  if (ext.length > 16) return null;
  return ext;
}

export function joinPath(...parts: Array<string | null | undefined>): string {
  const cleaned = parts
    .filter((p): p is string => typeof p === 'string' && p.length > 0)
    .map((p) => stripQuotes(p))
    .filter((p) => p.length > 0);
  if (cleaned.length === 0) return '';
  return normalizePath(cleaned.join('\\'));
}

/**
 * True when `child` is `parent` or lives inside it. Comparison is segment-based so
 * `C:\Users\Bob` is NOT inside `C:\Users\Bobbie`.
 */
export function isWithin(child: string, parent: string): boolean {
  const c = normalizePath(child).toLowerCase();
  const p = normalizePath(parent).toLowerCase();
  if (!c || !p) return false;
  if (c === p) return true;
  if (p === '\\' || p === '/') return true;
  const prefix = p.endsWith('\\') || p.endsWith('/') ? p : `${p}\\`;
  return c.startsWith(prefix);
}

/** Rejects paths that could escape validation: empty, NUL bytes, control characters. */
export function isSafePathString(p: string): boolean {
  if (!p || p.length === 0) return false;
  if (p.includes('\0')) return false;
  // eslint-disable-next-line no-control-regex
  if (/[\u0001-\u001f]/.test(p)) return false;
  if (p.length > 32_767) return false; // Windows extended-length path ceiling
  return true;
}

/** `C:\Users\Bob` → `C:` */
export function driveLetter(p: string): string | null {
  const match = /^([A-Za-z]):/.exec(normalizePath(p));
  return match ? `${match[1].toUpperCase()}:` : null;
}

export interface Breadcrumb {
  label: string;
  path: string;
}

/**
 * Breadcrumb trail. `C:\Users\Bob\Downloads` → `C:`, `Users`, `Bob`, `Downloads`.
 * UNC paths produce `\\`, `server\share`, then the remaining segments.
 */
export function breadcrumbs(p: string): Breadcrumb[] {
  const normalized = normalizePath(p);
  if (!normalized) return [];
  const segments = pathSegments(normalized);
  const out: Breadcrumb[] = [];
  const sep = normalized.startsWith('/') && !/^[A-Za-z]:/.test(normalized) ? '/' : '\\';

  let prefix = '';
  const driveMatch = /^([A-Za-z]):/.exec(normalized);
  if (driveMatch) {
    prefix = `${driveMatch[1].toUpperCase()}:\\`;
    out.push({ label: `${driveMatch[1].toUpperCase()}:`, path: prefix });
  } else if (normalized.startsWith('\\\\')) {
    out.push({ label: '\\\\', path: '\\\\' });
    prefix = '\\\\';
    if (segments.length >= 2) {
      prefix = `\\\\${segments[0]}\\${segments[1]}`;
      out.push({ label: `${segments[0]}\\${segments[1]}`, path: `${prefix}\\` });
    }
  } else {
    out.push({ label: '/', path: '/' });
    prefix = '/';
  }

  const skip = driveMatch ? 1 : normalized.startsWith('\\\\') ? 2 : 0;
  for (let i = skip; i < segments.length; i += 1) {
    const segment = segments[i];
    prefix = prefix.endsWith('\\') || prefix.endsWith('/') ? `${prefix}${segment}` : `${prefix}${sep}${segment}`;
    out.push({ label: segment, path: prefix });
  }
  return out;
}
