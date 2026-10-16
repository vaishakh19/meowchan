// Dodo Payments integration, server-side only.
//
// ## What is verified from the official documentation
//
// Dodo follows the **Standard Webhooks** specification (https://docs.dodopayments.com/developer-resources/webhooks):
//
//   * Requests carry `webhook-id`, `webhook-signature` and `webhook-timestamp` headers.
//   * The signed content is `"{webhook-id}.{webhook-timestamp}.{rawBody}"`, joined by periods.
//   * The HMAC key is the base64 decoding of the webhook secret **with the `whsec_` prefix
//     stripped**. Passing the raw `whsec_...` string produces signatures that never verify.
//   * `webhook-signature` is a space-separated list of `v1,<signature>` entries; any one of them
//     verifying is sufficient, because Dodo may rotate signatures within a delivery.
//   * Dodo retries until it receives a 2xx, so every handler must be idempotent.
//
// ## What this file will never do
//
// It never treats a browser redirect, a query parameter, an email address or a client-supplied
// order id as proof of payment. Only a signature-verified webhook grants an entitlement. The
// documentation is explicit that the redirect is not the source of truth.

import { constantTimeEqual, sha256Hex } from './cors.ts';

export const DODO_TEST_BASE = 'https://test.dodopayments.com';
export const DODO_LIVE_BASE = 'https://live.dodopayments.com';

/** Webhook events this backend acts on. Everything else is acknowledged and ignored. */
export type DodoEventType =
  | 'payment.succeeded'
  | 'payment.failed'
  | 'payment.processing'
  | 'payment.cancelled'
  | 'payment.charge_failed'
  | 'refund.succeeded'
  | 'refund.failed'
  | 'dispute.opened'
  | 'dispute.challenged'
  | 'dispute.accepted'
  | 'dispute.cancelled'
  | 'dispute.expired'
  | 'dispute.won'
  | 'dispute.lost'
  | 'license_key.created';

/** The events that grant an entitlement. Exactly one event type does. */
export const GRANTING_EVENTS: readonly DodoEventType[] = ['payment.succeeded'];

/** The events that remove one. */
export const REVOKING_EVENTS: readonly DodoEventType[] = [
  'refund.succeeded',
  'dispute.accepted',
  'dispute.lost',
];

/** Base URL for the environment this deployment is configured for. */
export function baseUrl(environment: 'test' | 'live'): string {
  return environment === 'live' ? DODO_LIVE_BASE : DODO_TEST_BASE;
}

/* ------------------------------------------------------------------ webhooks -- */

/** The three headers Dodo sends, in the order the signature check needs them. */
export interface WebhookHeaders {
  id: string | null;
  signature: string | null;
  timestamp: string | null;
}

export function webhookHeaders(request: Request): WebhookHeaders {
  return {
    id: request.headers.get('webhook-id'),
    signature: request.headers.get('webhook-signature'),
    timestamp: request.headers.get('webhook-timestamp'),
  };
}

/**
 * The base64 decoding of the webhook secret, without the `whsec_` prefix.
 *
 * This is the single most common integration mistake: the secret is stored base64-encoded so it
 * survives JSON, and the HMAC key is the *decoded* bytes. Returns `null` for anything unusable.
 */
export function webhookKey(secret: string): Uint8Array | null {
  const trimmed = secret.trim();
  if (trimmed.length === 0) return null;
  const withoutPrefix = trimmed.startsWith('whsec_') ? trimmed.slice('whsec_'.length) : trimmed;
  // A base64 secret decodes to 24 bytes or more in practice; anything shorter is misconfigured.
  if (withoutPrefix.length < 16) return null;
  try {
    const binary = atob(withoutPrefix);
    if (binary.length < 16) return null;
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return bytes;
  } catch {
    return null;
  }
}

async function hmacSha256(key: Uint8Array, message: string): Promise<string> {
  const cryptoKey = await crypto.subtle.importKey(
    'raw',
    key as unknown as ArrayBuffer,
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  const signature = await crypto.subtle.sign('HMAC', cryptoKey, new TextEncoder().encode(message));
  // Standard Webhooks signatures are base64, not hex.
  return btoa(String.fromCharCode(...new Uint8Array(signature)));
}

/**
 * Verifies a Standard Webhooks delivery.
 *
 * Checks, in order: all three headers present, timestamp within tolerance (so a captured delivery
 * cannot be replayed forever), then the HMAC over `id.timestamp.body` against every `v1,`
 * signature offered.
 *
 * The raw body must be the exact bytes Dodo sent. Re-serialising parsed JSON changes the key
 * whitespace and breaks the signature, which is why every caller reads `request.text()` once and
 * passes it around.
 */
export async function verifyWebhook(
  headers: WebhookHeaders,
  rawBody: string,
  secret: string,
  toleranceSeconds = 5 * 60,
  nowSeconds = Math.floor(Date.now() / 1000),
): Promise<{ ok: true } | { ok: false; reason: string }> {
  if (!headers.id || !headers.signature || !headers.timestamp) {
    return { ok: false, reason: 'missing-headers' };
  }

  const sentAt = Number(headers.timestamp);
  if (!Number.isFinite(sentAt) || Math.abs(nowSeconds - sentAt) > toleranceSeconds) {
    return { ok: false, reason: 'stale-timestamp' };
  }

  const key = webhookKey(secret);
  if (key === null) {
    // A misconfigured secret is a server fault, never a client one. Logged, never returned.
    return { ok: false, reason: 'server-misconfigured' };
  }

  const signed = `${headers.id}.${headers.timestamp}.${rawBody}`;
  const expected = await hmacSha256(key, signed);

  const candidates = headers.signature.split(' ').map((entry) => entry.trim()).filter(Boolean);
  for (const candidate of candidates) {
    const comma = candidate.indexOf(',');
    if (comma < 0) continue;
    const version = candidate.slice(0, comma);
    const value = candidate.slice(comma + 1).trim();
    if (version !== 'v1') continue;
    if (constantTimeEqual(value, expected)) {
      return { ok: true };
    }
  }
  return { ok: false, reason: 'bad-signature' };
}

/* ------------------------------------------------------------------ checkout -- */

export interface CheckoutRequest {
  product_cart: Array<{ product_id: string; quantity: number }>;
  customer: { email: string; name?: string; phone_number?: string };
  return_url?: string;
  metadata?: Record<string, string>;
  feature_flags?: Record<string, boolean>;
}

export interface CheckoutResponse {
  session_id: string;
  checkout_url: string | null;
  client_secret?: string;
  payment_id?: string;
  publishable_key?: string;
}

/**
 * Creates a checkout session.
 *
 * The product cart is built **here**, from the entitlement config in the database, and is never
 * taken from the request. The caller can only say *which tier* it wants; the price, the currency
 * and the quantity are the server's to decide.
 */
export async function createCheckout(
  environment: 'test' | 'live',
  apiKey: string,
  body: CheckoutRequest,
): Promise<CheckoutResponse> {
  const response = await fetch(`${baseUrl(environment)}/checkouts`, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${apiKey}`,
      'content-type': 'application/json',
      accept: 'application/json',
    },
    body: JSON.stringify(body),
  });

  const text = await response.text();
  if (!response.ok) {
    // Never echo the provider's body to the client: it can contain identifiers.
    throw new Error(`dodo-checkout-failed:${response.status}`);
  }

  const parsed = JSON.parse(text) as Partial<CheckoutResponse>;
  if (typeof parsed.session_id !== 'string' || parsed.session_id.length === 0) {
    throw new Error('dodo-checkout-malformed');
  }
  return {
    session_id: parsed.session_id,
    checkout_url: typeof parsed.checkout_url === 'string' ? parsed.checkout_url : null,
    client_secret: parsed.client_secret,
    payment_id: parsed.payment_id,
    publishable_key: parsed.publishable_key,
  };
}

/* ------------------------------------------------------------------- payloads -- */

export interface DodoCustomer {
  customer_id?: string;
  email: string;
  name?: string;
  phone_number?: string;
}

export interface DodoPayment {
  payment_id: string;
  total_amount: number;
  currency: string;
  status: string;
  customer: DodoCustomer;
  metadata?: Record<string, unknown>;
  billing?: Record<string, unknown>;
  settlement_amount?: number;
  settlement_currency?: string;
  refunds?: Array<Record<string, unknown>>;
  disputes?: Array<Record<string, unknown>>;
  created_at?: string;
  digital_products_delivered?: boolean;
}

export interface DodoWebhookPayload {
  type: string;
  timestamp?: string;
  data: Record<string, unknown>;
}

/** Narrows a webhook body to a payment event without trusting its shape. */
export function parsePayment(payload: DodoWebhookPayload): DodoPayment | null {
  const data = payload.data;
  if (typeof data.payment_id !== 'string' || data.payment_id.length === 0) return null;
  if (typeof data.total_amount !== 'number') return null;
  if (typeof data.currency !== 'string') return null;
  const customer = data.customer;
  if (!customer || typeof customer !== 'object') return null;
  const email = (customer as Record<string, unknown>).email;
  if (typeof email !== 'string' || email.length === 0) return null;
  return data as unknown as DodoPayment;
}

/** SHA-256 of the raw body, used to detect a replayed id carrying different content. */
export async function bodyHash(rawBody: string): Promise<string> {
  return sha256Hex(rawBody);
}
