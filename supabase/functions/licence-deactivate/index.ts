// POST /functions/v1/licence-deactivate
//
// Releases the licence slot for one device.
//
// ## What it will not do
//
// It will not release a device the caller does not own. The caller proves ownership by presenting
// the licence key *and* the device binding that key was activated for; releasing somebody else's
// device would be a way to defeat the limit. Releasing another machine is an account-page
// operation, authenticated by the customer's own login, not by a licence key.
//
// ## Why this exists at all
//
// Because a customer who replaces a laptop must be able to move their licence without paying again.
// The slot has to be releasable, and it has to be releasable only by the person entitled to.

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

interface LicenceRow {
  id: string;
  product_id: string;
  tier_id: string;
  status: string;
}

async function sha256Hex(value: string): Promise<string> {
  const bytes = new TextEncoder().encode(value);
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');
}

Deno.serve(async (request) => {
  const origin = request.headers.get('origin');
  const id = requestId();

  const early = preflight(request);
  if (early) return early;
  if (request.method !== 'POST') return fail(origin, 405, 'Use POST.', id);

  const supabaseUrl = Deno.env.get('SUPABASE_URL');
  const serviceKey = Deno.env.get('SUPABASE_SERVICE_ROLE_KEY');
  if (!supabaseUrl || !serviceKey) {
    log('licence-deactivate.misconfigured', { requestId: id });
    return fail(origin, 503, 'Unavailable right now. Please try again shortly.', id);
  }

  const parsed = await readJson(request, origin, id);
  if (!parsed.ok) return parsed.response;
  const body = parsed.value;

  const licenceKey = typeof body.licenceKey === 'string' ? body.licenceKey.trim() : '';
  const deviceBinding = cleanDeviceBinding(body.deviceBinding);
  if (licenceKey.length < 8 || !deviceBinding) {
    return fail(origin, 400, 'That request was incomplete.', id);
  }

  const supabase = createClient(supabaseUrl, serviceKey, {
    auth: { persistSession: false, autoRefreshToken: false },
  });

  const keyHash = await sha256Hex(licenceKey);
  const { data: licence } = await supabase
    .from('licences')
    .select('id, product_id, tier_id, status')
    .eq('key_hash', keyHash)
    .maybeSingle<LicenceRow>();

  if (!licence) {
    return fail(origin, 404, 'That licence key was not recognised.', id);
  }

  // Ownership of the binding is proven by the activation row: the binding must have been activated
  // against this licence, and the caller is holding the key for that licence.
  const { data: activation } = await supabase
    .from('device_activations')
    .select('id')
    .eq('licence_id', licence.id)
    .eq('device_binding', deviceBinding)
    .is('deactivated_at', null)
    .maybeSingle<{ id: string }>();

  if (!activation) {
    log('licence-deactivate.not-activated', { requestId: id, licenceId: licence.id });
    return fail(origin, 404, 'That device is not active on this licence.', id);
  }

  const { data: released } = await supabase.rpc('deactivate_device', {
    licence: licence.id,
    binding: deviceBinding,
    reason: 'user-request',
    now_ms: Date.now(),
  });

  await supabase.from('audit_log').insert({
    at_ms: Date.now(),
    actor: 'licence-deactivate',
    action: 'device.deactivated',
    licence_id: licence.id,
    detail: 'Device released a licence slot.',
  });

  const { data: activeCount } = await supabase.rpc('active_device_count', { licence: licence.id });

  log('licence-deactivate.ok', { requestId: id, licenceId: licence.id, released: released === true });

  return json(
    {
      released: released === true,
      activeDevices: typeof activeCount === 'number' ? activeCount : 0,
    },
    { origin },
  );
});
