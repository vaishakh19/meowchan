// Shared helpers for every Diskwise Edge Function.
//
// Rules enforced here, once, so no function can forget them:
//   * CORS is an explicit allow-list, never `*` on an endpoint that takes money.
//   * Every error response is the same shape and carries no stack trace, no SQL and no secret.
//   * Every request body is size-capped before it is parsed.
//   * Nothing is logged that could identify a user's files, and nothing logs a secret.

export const ALLOWED_ORIGINS = [
  'https://diskwise.app',
  'https://www.diskwise.app',
  'http://localhost:5173',
  'http://localhost:1420',
  'http://127.0.0.1:5173',
  'http://127.0.0.1:1420',
  'tauri://localhost',
];

/** The maximum accepted request body, in bytes. Anything larger is refused without being read. */
export const MAX_BODY_BYTES = 64 * 1024;

export function corsHeaders(origin: string | null): Record<string, string> {
  const allowed = origin && ALLOWED_ORIGINS.includes(origin) ? origin : ALLOWED_ORIGINS[0];
  return {
    'Access-Control-Allow-Origin': allowed,
    'Access-Control-Allow-Headers': 'authorization, x-client-info, apikey, content-type',
    'Access-Control-Allow-Methods': 'POST, OPTIONS',
    'Access-Control-Max-Age': '86400',
    Vary: 'Origin',
  };
}

export function json(
  body: unknown,
  init: { status?: number; origin: string | null } = {},
): Response {
  return new Response(JSON.stringify(body), {
    status: init.status ?? 200,
    headers: {
      'content-type': 'application/json; charset=utf-8',
      'cache-control': 'no-store',
      'x-content-type-options': 'nosniff',
      ...corsHeaders(init.origin),
    },
  });
}

/**
 * The single error shape.
 *
 * `message` is written for a human and never names a table, a column, a constraint or a secret.
 * `requestId` lets support correlate with server logs without the client seeing them.
 */
export function fail(
  origin: string | null,
  status: number,
  message: string,
  requestId: string,
): Response {
  return json({ error: message, requestId }, { status, origin });
}

export function preflight(request: Request): Response | null {
  if (request.method !== 'OPTIONS') return null;
  return new Response(null, { status: 204, headers: corsHeaders(request.headers.get('origin')) });
}

/** Reads and parses a JSON body, refusing anything oversized or malformed. */
export async function readJson(
  request: Request,
  origin: string | null,
  requestId: string,
): Promise<{ ok: true; value: Record<string, unknown> } | { ok: false; response: Response }> {
  const declared = Number(request.headers.get('content-length') ?? '0');
  if (Number.isFinite(declared) && declared > MAX_BODY_BYTES) {
    return { ok: false, response: fail(origin, 413, 'That request was too large.', requestId) };
  }

  let raw: string;
  try {
    raw = await request.text();
  } catch {
    return { ok: false, response: fail(origin, 400, 'That request could not be read.', requestId) };
  }
  if (raw.length > MAX_BODY_BYTES) {
    return { ok: false, response: fail(origin, 413, 'That request was too large.', requestId) };
  }
  if (raw.length === 0) return { ok: true, value: {} };

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return { ok: false, response: fail(origin, 400, 'That request was not valid JSON.', requestId) };
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return { ok: false, response: fail(origin, 400, 'That request was not a JSON object.', requestId) };
  }
  return { ok: true, value: parsed as Record<string, unknown> };
}

/** A short, non-reversible correlation id. Safe to show a customer. */
export function requestId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(8));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

/** A conservative email shape check. Deliberately not RFC-complete; it only rejects obvious junk. */
export function looksLikeEmail(value: unknown): value is string {
  return typeof value === 'string' && /^[^\s@]{1,64}@[^\s@]{1,255}$/.test(value) && value.length <= 320;
}

/** Trims, and rejects anything that is not a plausible identifier. */
export function cleanIdentifier(value: unknown, max = 128): string | null {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  if (trimmed.length === 0 || trimmed.length > max) return null;
  if (!/^[A-Za-z0-9._:-]+$/.test(trimmed)) return null;
  return trimmed;
}

/** A base64url-encoded device binding, as the desktop app produces it. */
export function cleanDeviceBinding(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  if (!/^dev-[0-9a-f]{8,64}$/.test(trimmed)) return null;
  return trimmed;
}

/** SHA-256 as lowercase hex, matching the desktop app's hashing. */
export async function sha256Hex(value: string): Promise<string> {
  const bytes = new TextEncoder().encode(value);
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');
}

/** Constant-time comparison of two hex strings of equal length. */
export function constantTimeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let index = 0; index < a.length; index += 1) {
    diff |= a.charCodeAt(index) ^ b.charCodeAt(index);
  }
  return diff === 0;
}

/**
 * Logs a structured line with the request id.
 *
 * Only ever called with ids, event names and short human notes. No request bodies, no customer
 * emails in full, no secrets, and never anything about the files on a customer's disk.
 */
export function log(event: string, fields: Record<string, unknown> = {}): void {
  console.log(JSON.stringify({ event, at: Date.now(), ...fields }));
}
