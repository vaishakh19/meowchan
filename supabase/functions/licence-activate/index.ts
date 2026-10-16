// POST /functions/v1/licence-activate
//
// The desktop app's only write path.
//
// ## What it does
//
//   1. Looks the licence up by the SHA-256 of the key the customer typed. The key is never stored,
//      so a database leak does not hand out working licences.
//   2. Refuses when the licence is not active (revoked, refunded, or never issued).
//   3. Calls `activate_device`, which serialises concurrent activations with `FOR UPDATE` on the
//      licence row and raises `device_limit_reached` when the limit is already consumed by other
//      machines.
//   4. Mints a fresh Ed25519-signed payload bound to *this* device and returns it.
//   5. Returns the device list, so the app can show the customer exactly what the ledger holds.
//
// ## Why the payload is minted here rather than stored
//
// The device binding is part of the signed payload, and the binding is only known at activation
// time. Minting on demand keeps the stored licence row free of anything device-specific.
//
// ## Why the desktop still verifies the signature locally
//
// So a compromised or buggy server cannot hand out a payload that unlocks anything. The app checks
// the signature against its embedded public key before trusting a single field. This is the same
// property the offline path relies on, and it is what makes offline verification meaningful.

import { createClient } from 'jsr:@supabase/supabase-js@2';
import {
  cleanDeviceBinding,
  fail,
  json,
  log,
  preflight,
  readJson,
  requestId,
} from '../_shared/cors.ts';
import { buildPayload, loadPrivateKey, signPayload } from '../_shared/licence.ts';

const RATE_BUCKET = 'licence-activate';
const RATE_CAP = 10;
const RATE_WINDOW_MS = 15 * 60 * 1000;
const MAX_NAME_LENGTH = 64;

interface LicenceRow {
  id: string;
  order_id: string;
  product_id: string;
  tier_id: string;
  key_prefix: string;
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

/** A device name is shown to the customer, so it is trimmed and length-capped. Never a path. */
function cleanDeviceName(value: unknown): string {
  if (typeof value !== 'string') return 'This PC';
  const trimmed = value.trim().replace(/[\r\n\t]+/g, ' ').slice(0, MAX_NAME_LENGTH);
  return trimmed.length > 0 ? trimmed : 'This PC';
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

  if (!supabaseUrl || !serviceKey || !signingKeyPem) {
    log('licence-activate.misconfigured', { requestId: id });
    return fail(origin, 503, 'Activation is unavailable right now. Please try again shortly.', id);
  }

  const parsed = await readJson(request, origin, id);
  if (!parsed.ok) return parsed.response;
  const body = parsed.value;

  const licenceKey = typeof body.licenceKey === 'string' ? body.licenceKey.trim() : '';
  const deviceBinding = cleanDeviceBinding(body.deviceBinding);
  const deviceName = cleanDeviceName(body.deviceName);

  if (licenceKey.length < 8 || licenceKey.length > 4096) {
    return fail(origin, 400, 'That licence key is not valid.', id);
  }
  if (!deviceBinding) {
    return fail(origin, 400, 'This machine could not be identified. Please try again.', id);
  }

  const supabase = createClient(supabaseUrl, serviceKey, {
    auth: { persistSession: false, autoRefreshToken: false },
  });

  const now = Date.now();
  const { data: allowed } = await supabase.rpc('rate_limit_hit', {
    bucket: RATE_BUCKET,
    subject: deviceBinding,
    cap: RATE_CAP,
    window_ms: RATE_WINDOW_MS,
    now_ms: now,
  });
  if (allowed === false) {
    return fail(origin, 429, 'Too many activation attempts. Please try again later.', id);
  }

  // Look up by hash. The key itself is never compared against a stored column.
  const keyHash = await sha256Hex(licenceKey);
  const { data: licence } = await supabase
    .from('licences')
    .select(
      'id, order_id, product_id, tier_id, key_prefix, signing_key_id, schema_version, permanent, lease_days, device_limit, status',
    )
    .eq('key_hash', keyHash)
    .maybeSingle<LicenceRow>();

  // One generic message for "no such key" and "wrong key": telling them apart would let a caller
  // enumerate valid key prefixes.
  if (!licence) {
    log('licence-activate.unknown-key', { requestId: id });
    return fail(origin, 404, 'That licence key was not recognised. Check it and try again.', id);
  }
  if (licence.status !== 'active') {
    log('licence-activate.not-active', { requestId: id, licenceId: licence.id, status: licence.status });
    return fail(
      origin,
      403,
      'This licence is no longer active. If you believe that is wrong, contact support with your order reference.',
      id,
    );
  }

  // The tier must still be offered and the product still active.
  const { data: tier } = await supabase
    .from('entitlement_configs')
    .select('id, product_id, active')
    .eq('id', licence.tier_id)
    .maybeSingle<{ id: string; product_id: string; active: boolean }>();
  if (!tier || !tier.active || tier.product_id !== licence.product_id) {
    log('licence-activate.tier-withdrawn', { requestId: id, licenceId: licence.id });
    return fail(origin, 409, 'That edition is no longer available. Please contact support.', id);
  }

  // The device limit is enforced here, in the database, atomically.
  const { data: activeCount, error: activateError } = await supabase.rpc('activate_device', {
    licence: licence.id,
    binding: deviceBinding,
    name: deviceName,
    platform: 'windows',
    now_ms: now,
  });

  if (activateError) {
    const code = (activateError as { code?: string }).code ?? '';
    // `check_violation` is what `activate_device` raises for a full licence.
    if (code === 'check_violation' || code === '23514') {
      log('licence-activate.limit', { requestId: id, licenceId: licence.id });
      return fail(
        origin,
        409,
        `This licence is already active on ${licence.device_limit} devices. Deactivate one of them to continue.`,
        id,
      );
    }
    if (code === 'no_data_found' || code === 'P0002') {
      return fail(origin, 403, 'This licence is no longer active.', id);
    }
    log('licence-activate.activate-failed', { requestId: id, licenceId: licence.id });
    return fail(origin, 500, 'Activation failed. Please try again shortly.', id);
  }

  // Mint the signed payload. A permanent licence gets no lease expiry; a time-limited tier gets one
  // so a revoked licence stops working offline within the published window.
  const leaseUntil = licence.permanent
    ? null
    : now + licence.lease_days * 24 * 60 * 60 * 1000;

  let signed;
  try {
    const privateKey = loadPrivateKey(signingKeyPem);
    const payload = buildPayload(
      {
        licenseId: licence.id,
        productId: licence.product_id,
        tierId: licence.tier_id,
        issuedAt: now,
        permanent: licence.permanent,
        leaseUntil,
        deviceBinding,
        deviceLimit: licence.device_limit,
        // Deliberately empty: nothing here may identify a customer or their files.
        metadata: {},
      },
      keyId,
      licence.schema_version,
    );
    signed = signPayload(privateKey, payload);
  } catch (error) {
    // A signing failure is a server fault. The client gets the same generic message as any other.
    log('licence-activate.signing-failed', {
      requestId: id,
      reason: error instanceof Error ? error.message.slice(0, 120) : 'unknown',
    });
    return fail(origin, 500, 'Activation failed. Please try again shortly.', id);
  }

  const { data: devices } = await supabase.rpc('active_device_count', { licence: licence.id });

  await supabase.from('audit_log').insert({
    at_ms: now,
    actor: 'licence-activate',
    action: 'device.activated',
    licence_id: licence.id,
    order_id: licence.order_id,
    detail: `Device ${deviceName} activated.`,
  });

  log('licence-activate.ok', {
    requestId: id,
    licenceId: licence.id,
    activeDevices: typeof devices === 'number' ? devices : activeCount,
  });

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
      activeDevices: typeof devices === 'number' ? devices : activeCount,
      // The public key the desktop app should hold. Returning it costs nothing and makes the
      // rotation story checkable: the app can compare it to its embedded key.
      signingKeyId: keyId,
    },
    { origin },
  );
});
