// POST /functions/v1/dodo-webhook
//
// The only place a Diskwise entitlement is granted.
//
// ## Trust model
//
// Nothing the browser does is proof of payment. This endpoint accepts a delivery only when the
// Standard Webhooks signature verifies against the configured secret. A forged body, a replayed
// body with an old timestamp, and a body with a missing signature are all refused with a 4xx and
// grant nothing.
//
// ## Idempotency
//
// Dodo retries until it gets a 2xx. Every delivery is claimed by its `webhook-id` before any work
// happens, so the second delivery of the same event is acknowledged as already-processed and does
// nothing. A delivery that reuses an id with a *different* body is refused, not silently ignored.
//
// ## Ordering
//
// Acknowledge first, process second. Dodo's timeout is shorter than the work, and a timeout causes a
// retry which this function would then have to deduplicate again. Processing happens in a background
// task so the 2xx goes out immediately.

import { createClient } from 'jsr:@supabase/supabase-js@2';
import { fail, json, log, preflight, requestId } from '../_shared/cors.ts';
import {
  GRANTING_EVENTS,
  REVOKING_EVENTS,
  bodyHash,
  parsePayment,
  verifyWebhook,
  webhookHeaders,
  type DodoEventType,
  type DodoWebhookPayload,
} from '../_shared/dodo.ts';

/**
 * A licence key: 160 bits of randomness, grouped for readability when a customer copies it out of
 * an email.
 *
 * Only its SHA-256 hash is stored. The key itself is shown once, at issuance, and is recoverable
 * only through the documented recovery flow.
 */
function generateLicenceKey(): { key: string; prefix: string } {
  const bytes = crypto.getRandomValues(new Uint8Array(20));
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'; // Crockford, no ambiguous characters.
  let body = '';
  for (const byte of bytes) {
    body += alphabet[byte % 32];
  }
  const grouped = (body.match(/.{1,5}/g) ?? []).join('-');
  return { key: grouped, prefix: grouped.slice(0, 5) };
}

async function sha256Hex(value: string): Promise<string> {
  const bytes = new TextEncoder().encode(value);
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');
}

/** The environment a payment was taken in, from the metadata the checkout function wrote. */
function environmentOf(payload: DodoWebhookPayload): 'test' | 'live' {
  const raw = payload.data?.metadata;
  if (raw && typeof raw === 'object') {
    const value = (raw as Record<string, unknown>).environment;
    if (value === 'live' || value === 'test') return value;
  }
  return 'test';
}

Deno.serve(async (request) => {
  const origin = request.headers.get('origin');
  const id = requestId();

  if (request.method === 'OPTIONS') return preflight(request) ?? new Response(null, { status: 204 });
  if (request.method !== 'POST') return fail(origin, 405, 'Use POST.', id);

  const supabaseUrl = Deno.env.get('SUPABASE_URL');
  const serviceKey = Deno.env.get('SUPABASE_SERVICE_ROLE_KEY');
  const webhookSecret = Deno.env.get('DODO_WEBHOOK_SECRET');

  if (!supabaseUrl || !serviceKey || !webhookSecret) {
    log('webhook.misconfigured', { requestId: id });
    return fail(origin, 503, 'Unavailable.', id);
  }

  // Read the body exactly once, as text. Re-serialising parsed JSON would change the signed bytes.
  const rawBody = await request.text();

  const verification = await verifyWebhook(webhookHeaders(request), rawBody, webhookSecret);
  if (!verification.ok) {
    // Deliberately terse: the reason is logged, never returned. A caller probing the endpoint learns
    // nothing about which check failed.
    log('webhook.rejected', { requestId: id, reason: verification.reason });
    return fail(origin, 401, 'Unauthorized.', id);
  }

  let payload: DodoWebhookPayload;
  try {
    payload = JSON.parse(rawBody) as DodoWebhookPayload;
  } catch {
    return fail(origin, 400, 'Bad request.', id);
  }

  const webhookId = request.headers.get('webhook-id') ?? '';
  const eventType = typeof payload.type === 'string' ? payload.type : 'unknown';
  const hash = await bodyHash(rawBody);

  const supabase = createClient(supabaseUrl, serviceKey, {
    auth: { persistSession: false, autoRefreshToken: false },
  });

  // Claim the id. A conflict means this exact delivery has been seen before: acknowledge and stop.
  // Dodo treats any 2xx as success, so a duplicate is handled exactly like a fresh one.
  const { data: claimed, error: claimError } = await supabase.rpc('claim_webhook', {
    id: webhookId,
    event_type: eventType,
    body_hash: hash,
    now_ms: Date.now(),
  });
  if (claimError) {
    log('webhook.claim-failed', { requestId: id, webhookId });
    return fail(origin, 500, 'Unavailable.', id);
  }
  if (claimed === false) {
    log('webhook.duplicate', { requestId: id, webhookId, eventType });
    return json({ received: true, duplicate: true }, { origin, status: 200 });
  }

  // Acknowledge before processing. Dodo's retry window is generous, but a slow handler still causes
  // avoidable retries, and every retry is another row in `webhook_events`.
  const background = processEvent(supabase, webhookId, eventType as DodoEventType, payload, id);
  // @ts-ignore — Deno's `waitUntil` keeps the isolate alive past the response.
  void (globalThis as unknown as { waitUntil?: (p: Promise<unknown>) => void }).waitUntil?.(
    background,
  );
  void background;

  return json({ received: true }, { origin, status: 200 });
});

async function processEvent(
  supabase: ReturnType<typeof createClient>,
  webhookId: string,
  eventType: DodoEventType,
  payload: DodoWebhookPayload,
  requestId: string,
): Promise<void> {
  const now = Date.now();
  try {
    if ((GRANTING_EVENTS as readonly string[]).includes(eventType)) {
      await grant(supabase, payload, requestId);
    } else if ((REVOKING_EVENTS as readonly string[]).includes(eventType)) {
      await revoke(supabase, payload, eventType, requestId);
    } else {
      // Recorded, acknowledged, and deliberately ignored. `payment.failed`,
      // `payment.processing`, `payment.cancelled`, `refund.failed`, `dispute.opened` and the
      // remaining dispute states change nothing about entitlement.
      log('webhook.ignored', { requestId, webhookId, eventType });
    }
    await supabase.rpc('finish_webhook', { id: webhookId, error: null, now_ms: Date.now() });
  } catch (error) {
    const message = error instanceof Error ? error.message.slice(0, 200) : 'unknown';
    log('webhook.processing-failed', { requestId, webhookId, eventType, reason: message });
    // The failure is recorded against the claimed id. A retry of the same id will be treated as a
    // duplicate and will *not* retry the work, which is correct: the failure is a bug to fix, not a
    // delivery to replay. Manual reprocessing is a documented support operation.
    await supabase.rpc('finish_webhook', { id: webhookId, error: message, now_ms: Date.now() });
  }
}

/* --------------------------------------------------------------- granting -- */

async function grant(
  supabase: ReturnType<typeof createClient>,
  payload: DodoWebhookPayload,
  requestId: string,
): Promise<void> {
  const payment = parsePayment(payload);
  if (!payment) {
    log('webhook.grant.malformed', { requestId });
    return;
  }

  const environment = environmentOf(payload);
  const metadata = (payload.data?.metadata ?? {}) as Record<string, unknown>;
  const tierId = typeof metadata.tier_id === 'string' ? metadata.tier_id : '';
  const productId = typeof metadata.product_id === 'string' ? metadata.product_id : '';

  if (!tierId || !productId) {
    log('webhook.grant.no-tier', { requestId, paymentId: payment.payment_id });
    return;
  }

  // A test-mode payment is recorded but never granted. This is the belt to the compile-time
  // environment default's braces: even if a live deployment is pointed at the sandbox, a sandbox
  // payment cannot produce a real entitlement.
  if (environment !== 'live') {
    await upsertOrder(supabase, payment, tierId, productId, 'succeeded', 'test');
    log('webhook.grant.test-mode', { requestId, paymentId: payment.payment_id, environment });
    return;
  }

  const { data: tier } = await supabase
    .from('entitlement_configs')
    .select('id, product_id, device_limit, offline_lease_days, permanent, update_eligibility_months')
    .eq('id', tierId)
    .eq('active', true)
    .maybeSingle<{
      id: string;
      product_id: string;
      device_limit: number;
      offline_lease_days: number;
      permanent: boolean;
      update_eligibility_months: number | null;
    }>();

  if (!tier) {
    log('webhook.grant.unknown-tier', { requestId, tierId });
    return;
  }

  const order = await upsertOrder(supabase, payment, tierId, productId, 'succeeded', 'live');

  // Exactly one licence per order, enforced by a unique index. A replayed `payment.succeeded` with
  // a different webhook id but the same payment id hits the unique constraint on
  // `orders.provider_payment_id` first and is a no-op.
  const { data: existing } = await supabase
    .from('licences')
    .select('id')
    .eq('order_id', order.id)
    .maybeSingle<{ id: string }>();
  if (existing) {
    log('webhook.grant.already-issued', { requestId, orderId: order.id });
    return;
  }

  // The key is generated here and never stored in plaintext. Its hash is what the desktop app's
  // activation path looks up.
  const generated = generateLicenceKey();
  const keyHash = await sha256Hex(generated.key);

  const signingKeyId = Deno.env.get('LICENCE_SIGNING_KEY_ID') ?? 'k2026-01';
  const schemaVersion = Number(Deno.env.get('LICENCE_SCHEMA_VERSION') ?? '1');
  const updateUntil =
    tier.update_eligibility_months === null
      ? null
      : Date.now() + tier.update_eligibility_months * 30 * 24 * 60 * 60 * 1000;

  const { data: licence, error: licenceError } = await supabase
    .from('licences')
    .insert({
      order_id: order.id,
      product_id: tier.product_id,
      tier_id: tier.id,
      key_prefix: generated.prefix,
      key_hash: keyHash,
      signing_key_id: signingKeyId,
      schema_version: Number.isFinite(schemaVersion) ? schemaVersion : 1,
      permanent: tier.permanent,
      lease_days: tier.offline_lease_days,
      device_limit: tier.device_limit,
      update_until: updateUntil,
      issued_at: Date.now(),
      status: 'active',
      created_at: Date.now(),
      updated_at: Date.now(),
    })
    .select('id')
    .single<{ id: string }>();

  if (licenceError) {
    // The unique index on `licences.order_id` is what makes this safe to retry: a duplicate
    // `payment.succeeded` for the same order cannot create a second licence.
    log('webhook.grant.insert-failed', { requestId, orderId: order.id });
    throw new Error('licence-insert-failed');
  }

  await supabase.from('audit_log').insert({
    at_ms: Date.now(),
    actor: 'dodo-webhook',
    action: 'licence.issued',
    licence_id: licence.id,
    order_id: order.id,
    detail: `Licence issued for tier ${tier.id}.`,
  });

  // The key is delivered out of band (see `docs/licensing.md`). It is deliberately not returned in
  // the webhook response, which Dodo does not need, and not written to the log.
  log('webhook.grant.issued', { requestId, orderId: order.id, licenceId: licence.id, tierId: tier.id });
}

/* -------------------------------------------------------------- revoking -- */

async function revoke(
  supabase: ReturnType<typeof createClient>,
  payload: DodoWebhookPayload,
  eventType: DodoEventType,
  requestId: string,
): Promise<void> {
  const payment = parsePayment(payload);
  if (!payment) {
    log('webhook.revoke.malformed', { requestId });
    return;
  }

  const { data: order } = await supabase
    .from('orders')
    .select('id, status, environment')
    .eq('provider_payment_id', payment.payment_id)
    .maybeSingle<{ id: string; status: string; environment: string }>();

  if (!order) {
    // A refund for a payment this deployment never recorded is either a test-mode payment or an
    // order from a different environment. Nothing to revoke.
    log('webhook.revoke.unknown-order', { requestId, paymentId: payment.payment_id });
    return;
  }

  const source = eventType.startsWith('refund') ? 'refund' : eventType.startsWith('dispute') ? 'dispute' : 'admin';

  if (source === 'refund') {
    await supabase.from('refunds').upsert(
      {
        order_id: order.id,
        provider_refund_id: `${payment.payment_id}:${eventType}`,
        amount_minor_units: payment.total_amount,
        currency: payment.currency,
        status: 'succeeded',
        created_at: Date.now(),
      },
      { onConflict: 'provider_refund_id' },
    );
    await supabase.from('orders').update({ status: 'refunded', updated_at: Date.now() }).eq('id', order.id);
  }

  const { data: licences } = await supabase
    .from('licences')
    .select('id')
    .eq('order_id', order.id);

  const licenceIds = (licences ?? []).map((row: { id: string }) => row.id);

  // `neq('status', 'refunded')` makes this idempotent: a replayed refund event changes nothing.
  await supabase
    .from('licences')
    .update({
      status: 'refunded',
      status_reason: source,
      status_changed_at: Date.now(),
      updated_at: Date.now(),
    })
    .eq('order_id', order.id)
    .neq('status', 'refunded');

  // One revocation row per licence, so the reason survives even if the licence is later re-issued.
  for (const licenceId of licenceIds) {
    await supabase.from('revocations').insert({
      licence_id: licenceId,
      reason: source,
      source,
      actor: 'dodo-webhook',
      created_at: Date.now(),
    });
  }

  await supabase.from('audit_log').insert({
    at_ms: Date.now(),
    actor: 'dodo-webhook',
    action: `licence.${source}`,
    order_id: order.id,
    detail: `Licence revoked after ${source}.`,
  });

  log('webhook.revoke.done', { requestId, orderId: order.id, source });
}

/* ----------------------------------------------------------------- orders -- */

async function upsertOrder(
  supabase: ReturnType<typeof createClient>,
  payment: {
    payment_id: string;
    total_amount: number;
    currency: string;
    customer: { email: string; name?: string; customer_id?: string };
    metadata?: Record<string, unknown>;
  },
  tierId: string,
  productId: string,
  status: string,
  storedEnvironment: 'test' | 'live',
): Promise<{ id: string }> {
  const now = Date.now();
  const { data, error } = await supabase
    .from('orders')
    .upsert(
      {
        provider: 'dodo',
        provider_payment_id: payment.payment_id,
        product_id: productId,
        tier_id: tierId,
        amount_minor_units: Math.round(payment.total_amount),
        currency: payment.currency,
        status,
        customer_email: payment.customer.email,
        customer_name: payment.customer.name ?? null,
        customer_id: payment.customer.customer_id ?? null,
        metadata: payment.metadata ?? {},
        environment: storedEnvironment,
        paid_at: status === 'succeeded' ? now : null,
        created_at: now,
        updated_at: now,
      },
      { onConflict: 'provider_payment_id' },
    )
    .select('id')
    .single<{ id: string }>();

  if (error || !data) {
    throw new Error(`order-upsert-failed:${error?.code ?? 'no-row'}`);
  }
  return data;
}
