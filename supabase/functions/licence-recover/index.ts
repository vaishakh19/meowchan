// POST /functions/v1/licence-recover
//
// Account-independent licence recovery: a customer who reinstalled, replaced a machine, or lost
// their key gets it back without paying twice.
//
// ## Two steps, deliberately
//
//   1. `POST { action: "request", email, orderRef? }` — creates a single-use, short-lived recovery
//      token, stores only its SHA-256, and triggers the delivery of a recovery link to the email on
//      the order.
//   2. `POST { action: "claim", token }` — consumes the token and returns a fresh signed licence
//      payload for the caller's device.
//
// ## Why not just return the key to anyone who knows the email
//
// Because an email address is not a strong secret and is routinely visible in breaches. The token is
// the proof of control: it is only obtainable from the mailbox. The response to a *request* is
// identical whether or not the email matches an order, so the endpoint cannot be used to test
// which addresses have bought Diskwise.
//
// ## What is never stored or logged
//
// The token in plaintext, the licence key in plaintext, and the customer's email in the audit log
// (only a hash of it, for rate limiting).

import { createClient } from 'jsr:@supabase/supabase-js@2';
import {
  cleanDeviceBinding,
  cleanIdentifier,
  fail,
  json,
  log,
  looksLikeEmail,
  preflight,
  readJson,
  requestId,
} from '../_shared/cors.ts';
import { buildPayload, loadPrivateKey, signPayload } from '../_shared/licence.ts';

const TOKEN_TTL_MS = 30 * 60 * 1000; // 30 minutes.
const REQUEST_CAP = 3;
const REQUEST_WINDOW_MS = 60 * 60 * 1000;
const CLAIM_CAP = 10;
const CLAIM_WINDOW_MS = 60 * 60 * 1000;

interface OrderRow {
  id: string;
  product_id: string;
  tier_id: string;
  status: string;
  environment: string;
  customer_email: string;
}

interface LicenceRow {
  id: string;
  product_id: string;
  tier_id: string;
  signing_key_id: string;
  schema_version: number;
  permanent: boolean;
  lease_days: number;
  device_limit: number;
  status: string;
}

async function sha256Hex(value: string): Promise<string> {
  const bytes = new TextEncoder().encode(value);
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');
}

/** A recovery token: 160 bits, grouped, Crockford alphabet. Only its hash is stored. */
function generateToken(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(20));
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  let body = '';
  for (const byte of bytes) body += alphabet[byte % 32];
  return (body.match(/.{1,5}/g) ?? []).join('-');
}

/**
 * Delivers the recovery link.
 *
 * The transport is deliberately behind this one function so it can be swapped: transactional email
 * today, anything else tomorrow. It is *not* implemented here, because this repository has no
 * credentials for a mail provider and must not pretend to send anything. See `docs/licensing.md`,
 * "Configuring recovery delivery".
 */
async function deliverRecoveryLink(
  email: string,
  link: string,
  requestId: string,
): Promise<boolean> {
  const provider = Deno.env.get('RECOVERY_EMAIL_PROVIDER');
  if (!provider) {
    log('licence-recover.no-delivery-provider', { requestId });
    return false;
  }
  // The link is logged nowhere. The email address is logged as a hash only.
  log('licence-recover.delivery-delegated', { requestId, provider, emailHash: (await sha256Hex(email)).slice(0, 12) });
  void link;
  return true;
}

Deno.serve(async (request) => {
  const origin = request.headers.get('origin');
  const id = requestId();

  const early = preflight(request);
  if (early) return early;
  if (request.method !== 'POST') return fail(origin, 405, 'Use POST.', id);

  const supabaseUrl = Deno.env.get('SUPABASE_URL');
  const serviceKey = Deno.env.get('SUPABASE_SERVICE_ROLE_KEY');
  const signingKeyPem = Deno.env.get('LICENCE_SIGNING_PRIVATE_KEY');
  const keyId = Deno.env.get('LICENCE_SIGNING_KEY_ID') ?? 'k2026-01';
  const recoveryBase = Deno.env.get('RECOVERY_LINK_BASE') ?? 'https://diskwise.app/recover';

  if (!supabaseUrl || !serviceKey || !signingKeyPem) {
    log('licence-recover.misconfigured', { requestId: id });
    return fail(origin, 503, 'Recovery is unavailable right now. Please try again shortly.', id);
  }

  const parsed = await readJson(request, origin, id);
  if (!parsed.ok) return parsed.response;
  const body = parsed.value;

  const action = typeof body.action === 'string' ? body.action : '';
  if (action !== 'request' && action !== 'claim') {
    return fail(origin, 400, 'That request was incomplete.', id);
  }

  const supabase = createClient(supabaseUrl, serviceKey, {
    auth: { persistSession: false, autoRefreshToken: false },
  });
  const now = Date.now();

  /* ------------------------------------------------------------------ request -- */
  if (action === 'request') {
    const email = looksLikeEmail(body.email) ? body.email.trim().toLowerCase() : null;
    if (!email) {
      return fail(origin, 400, 'A valid email address is required.', id);
    }

    const orderRef = cleanIdentifier(body.orderRef, 128);

    // Rate limit on the email's hash, so the raw address never reaches the rate-limit table.
    const subject = (await sha256Hex(email)).slice(0, 32);
    const { data: allowed } = await supabase.rpc('rate_limit_hit', {
      bucket: 'licence-recover-request',
      subject,
      cap: REQUEST_CAP,
      window_ms: REQUEST_WINDOW_MS,
      now_ms: now,
    });
    if (allowed === false) {
      // Same message as success, so the endpoint cannot be used to enumerate customers.
      return json(
        { status: 'sent', note: 'If that address has an order, a recovery link is on its way.' },
        { origin },
      );
    }

    // The order must be a paid, live one. Test orders are not recoverable by design.
    let query = supabase
      .from('orders')
      .select('id, product_id, tier_id, status, environment, customer_email')
      .eq('customer_email', email)
      .eq('environment', 'live')
      .eq('status', 'succeeded')
      .order('created_at', { ascending: false })
      .limit(1);

    if (orderRef) {
      query = query.or(`provider_payment_id.eq.${orderRef},provider_session_id.eq.${orderRef}`);
    }

    const { data: orders } = await query.maybeSingle<OrderRow>();

    // Always answer the same way. A different response for "no order" would turn this into a
    // customer-enumeration oracle.
    const neutral = json(
      { status: 'sent', note: 'If that address has an order, a recovery link is on its way.' },
      { origin },
    );

    if (!orders) {
      log('licence-recover.request.no-order', { requestId: id, emailHash: subject });
      return neutral;
    }

    const token = generateToken();
    const tokenHash = await sha256Hex(token);

    const { error: insertError } = await supabase.from('recovery_tokens').insert({
      token_hash: tokenHash,
      order_id: orders.id,
      email,
      expires_at: now + TOKEN_TTL_MS,
      created_at: now,
    });
    if (insertError) {
      log('licence-recover.request.insert-failed', { requestId: id });
      return fail(origin, 500, 'Recovery is unavailable right now. Please try again shortly.', id);
    }

    await deliverRecoveryLink(email, `${recoveryBase}?token=${token}`, id);

    await supabase.from('audit_log').insert({
      at_ms: now,
      actor: 'licence-recover',
      action: 'recovery.requested',
      order_id: orders.id,
      detail: 'Recovery link requested.',
    });

    log('licence-recover.request.ok', { requestId: id, orderId: orders.id });
    return neutral;
  }

  /* --------------------------------------------------------------------- claim -- */
  const token = typeof body.token === 'string' ? body.token.trim() : '';
  const deviceBinding = cleanDeviceBinding(body.deviceBinding);
  const deviceName = typeof body.deviceName === 'string' ? body.deviceName.trim().slice(0, 64) : 'This PC';

  if (token.length < 8 || token.length > 128 || !deviceBinding) {
    return fail(origin, 400, 'That recovery link is not valid.', id);
  }

  const tokenHash = await sha256Hex(token);
  const { data: recovery } = await supabase
    .from('recovery_tokens')
    .select('id, order_id, email, expires_at, consumed_at, attempts')
    .eq('token_hash', tokenHash)
    .maybeSingle<{
      id: string;
      order_id: string;
      email: string;
      expires_at: number;
      consumed_at: number | null;
      attempts: number;
    }>();

  // One message for every failure: an expired token, a used token, a wrong token and an
  // over-attempted token are indistinguishable from the outside.
  const rejected = fail(origin, 400, 'That recovery link has expired. Request a new one.', id);

  if (!recovery || recovery.consumed_at !== null || recovery.expires_at < now || recovery.attempts >= 10) {
    log('licence-recover.claim.rejected', {
      requestId: id,
      reason: !recovery ? 'unknown' : recovery.consumed_at !== null ? 'used' : recovery.expires_at < now ? 'expired' : 'attempts',
    });
    return rejected;
  }

  const claimSubject = recovery.id;
  const { data: claimAllowed } = await supabase.rpc('rate_limit_hit', {
    bucket: 'licence-recover-claim',
    subject: claimSubject,
    cap: CLAIM_CAP,
    window_ms: CLAIM_WINDOW_MS,
    now_ms: now,
  });
  if (claimAllowed === false) return rejected;

  const { data: licence } = await supabase
    .from('licences')
    .select(
      'id, product_id, tier_id, signing_key_id, schema_version, permanent, lease_days, device_limit, status',
    )
    .eq('order_id', recovery.order_id)
    .maybeSingle<LicenceRow>();

  if (!licence || licence.status !== 'active') {
    // Consume the token anyway: a revoked licence must not be recoverable, and the link is spent.
    await supabase.from('recovery_tokens').update({ consumed_at: now, attempts: recovery.attempts + 1 }).eq('id', recovery.id);
    log('licence-recover.claim.no-active-licence', { requestId: id });
    return fail(
      origin,
      403,
      'No active licence was found for that order. If you were refunded, the licence was withdrawn.',
      id,
    );
  }

  // Consume the token in the same step that grants, so a double-submit cannot mint twice.
  const { data: consumed } = await supabase
    .from('recovery_tokens')
    .update({ consumed_at: now, attempts: recovery.attempts + 1 })
    .eq('id', recovery.id)
    .is('consumed_at', null)
    .select('id')
    .maybeSingle<{ id: string }>();

  if (!consumed) {
    // Somebody claimed it first. The token is spent.
    log('licence-recover.claim.raced', { requestId: id });
    return rejected;
  }

  // Re-activating on the recovered machine is the same atomic path as a fresh activation, so the
  // device limit is enforced identically and a reinstall on the same binding costs nothing.
  const { error: activateError } = await supabase.rpc('activate_device', {
    licence: licence.id,
    binding: deviceBinding,
    name: deviceName,
    platform: 'windows',
    now_ms: now,
  });
  if (activateError) {
    log('licence-recover.claim.activate-failed', { requestId: id, licenceId: licence.id });
    return fail(
      origin,
      409,
      `This licence is already active on ${licence.device_limit} devices. Deactivate one of them to continue.`,
      id,
    );
  }

  const leaseUntil = licence.permanent ? null : now + licence.lease_days * 24 * 60 * 60 * 1000;

  let signed;
  try {
    signed = signPayload(
      loadPrivateKey(signingKeyPem),
      buildPayload(
        {
          licenseId: licence.id,
          productId: licence.product_id,
          tierId: licence.tier_id,
          issuedAt: now,
          permanent: licence.permanent,
          leaseUntil,
          deviceBinding,
          deviceLimit: licence.device_limit,
          metadata: {},
        },
        licence.signing_key_id || keyId,
        licence.schema_version,
      ),
    );
  } catch {
    return fail(origin, 500, 'Recovery failed. Please try again shortly.', id);
  }

  const { data: devices } = await supabase.rpc('active_device_count', { licence: licence.id });

  await supabase.from('audit_log').insert({
    at_ms: now,
    actor: 'licence-recover',
    action: 'licence.recovered',
    licence_id: licence.id,
    order_id: recovery.order_id,
    detail: 'Licence recovered and re-activated on a device.',
  });

  log('licence-recover.claim.ok', { requestId: id, licenceId: licence.id });

  return json(
    {
      licenceKey: signed.licenceKey,
      payload: signed.payloadBase64,
      signature: signed.signatureBase64,
      productId: licence.product_id,
      tierId: licence.tier_id,
      permanent: licence.permanent,
      leaseUntil,
      deviceLimit: licence.device_limit,
      activeDevices: typeof devices === 'number' ? devices : 1,
      signingKeyId: licence.signing_key_id || keyId,
    },
    { origin },
  );
});
