// POST /functions/v1/create-checkout
//
// Creates a Dodo checkout session for a tier the caller names.
//
// The client sends exactly one thing it is allowed to decide: which tier, and the email and name to
// prefill. Everything that determines what the customer is charged — product id, price, currency,
// quantity — is read from `entitlement_configs` here, on the server. A tampered frontend cannot
// change the price, and cannot invent a product id.
//
// This function does **not** grant anything. It hands back a checkout URL and forgets about it.
// Entitlement is granted only by a signature-verified `payment.succeeded` webhook.

import { createClient } from 'jsr:@supabase/supabase-js@2';
import {
  cleanIdentifier,
  fail,
  json,
  log,
  looksLikeEmail,
  preflight,
  readJson,
  requestId,
} from '../_shared/cors.ts';
import { createCheckout, type DodoEventType } from '../_shared/dodo.ts';

interface TierRow {
  id: string;
  product_id: string;
  name: string;
  price_minor_units: number;
  currency: string;
  permanent: boolean;
  update_eligibility_months: number | null;
  device_limit: number;
  offline_lease_days: number;
  active: boolean;
}

const RATE_BUCKET = 'create-checkout';
const RATE_CAP = 10;
const RATE_WINDOW_MS = 60 * 60 * 1000;

Deno.serve(async (request) => {
  const origin = request.headers.get('origin');
  const id = requestId();

  const early = preflight(request);
  if (early) return early;
  if (request.method !== 'POST') {
    return fail(origin, 405, 'Use POST.', id);
  }

  const supabaseUrl = Deno.env.get('SUPABASE_URL');
  const serviceKey = Deno.env.get('SUPABASE_SERVICE_ROLE_KEY');
  const dodoKey = Deno.env.get('DODO_PAYMENTS_API_KEY');
  const environment = (Deno.env.get('DODO_ENVIRONMENT') ?? 'test') as 'test' | 'live';

  if (!supabaseUrl || !serviceKey || !dodoKey) {
    // A missing configuration is a server fault. The client gets the same generic message it would
    // get for any other failure, and the detail stays in the log.
    log('create-checkout.misconfigured', { requestId: id, hasUrl: !!supabaseUrl, hasKey: !!serviceKey, hasDodo: !!dodoKey });
    return fail(origin, 503, 'Checkout is unavailable right now. Please try again shortly.', id);
  }

  const parsed = await readJson(request, origin, id);
  if (!parsed.ok) return parsed.response;
  const body = parsed.value;

  const tierId = cleanIdentifier(body.tierId, 64);
  const email = looksLikeEmail(body.email) ? body.email.trim().toLowerCase() : null;
  const name = typeof body.name === 'string' ? body.name.trim().slice(0, 120) : '';
  const returnUrl = typeof body.returnUrl === 'string' ? body.returnUrl.trim().slice(0, 500) : undefined;

  if (!tierId || !email) {
    return fail(origin, 400, 'A tier and a valid email address are required.', id);
  }
  if (returnUrl !== undefined && !/^https?:\/\//.test(returnUrl)) {
    return fail(origin, 400, 'That return address is not valid.', id);
  }

  const supabase = createClient(supabaseUrl, serviceKey, {
    auth: { persistSession: false, autoRefreshToken: false },
  });

  // Rate limit before doing any work that costs money or provider quota.
  const now = Date.now();
  const { data: allowed, error: rateError } = await supabase.rpc('rate_limit_hit', {
    bucket: RATE_BUCKET,
    subject: email,
    cap: RATE_CAP,
    window_ms: RATE_WINDOW_MS,
    now_ms: now,
  });
  if (rateError) {
    log('create-checkout.rate-limit-failed', { requestId: id });
    return fail(origin, 429, 'Too many checkout attempts. Please try again later.', id);
  }
  if (allowed === false) {
    return fail(origin, 429, 'Too many checkout attempts. Please try again later.', id);
  }

  // The price comes from here and nowhere else.
  const { data: tier, error: tierError } = await supabase
    .from('entitlement_configs')
    .select('id, product_id, name, price_minor_units, currency, permanent, update_eligibility_months, device_limit, offline_lease_days, active')
    .eq('id', tierId)
    .eq('active', true)
    .maybeSingle<TierRow>();

  if (tierError || !tier) {
    return fail(origin, 404, 'That edition is not available.', id);
  }

  // A zero-price permanent tier is a configuration mistake, not a product. Refuse rather than
  // minting a free Pro licence.
  if (tier.permanent && tier.price_minor_units <= 0) {
    log('create-checkout.free-paid-tier', { requestId: id, tierId });
    return fail(origin, 409, 'That edition is not available.', id);
  }

  const { data: product, error: productError } = await supabase
    .from('products')
    .select('id')
    .eq('id', tier.product_id)
    .eq('active', true)
    .maybeSingle<{ id: string }>();
  if (productError || !product) {
    return fail(origin, 404, 'That edition is not available.', id);
  }

  // The provider's product id is a separate mapping: the database's `entitlement_configs.id` is the
  // stable internal name, and `DODO_PRODUCT_<TIER>` is what the seller configured in Dodo. Reading
  // it from the environment keeps it out of the repository.
  const providerProductId = Deno.env.get(`DODO_PRODUCT_${tierId.replace(/[^A-Za-z0-9]/g, '_').toUpperCase()}`);
  if (!providerProductId) {
    log('create-checkout.no-provider-product', { requestId: id, tierId });
    return fail(origin, 503, 'Checkout is unavailable right now. Please try again shortly.', id);
  }

  try {
    const session = await createCheckout(environment, dodoKey, {
      // Built here. Never from the request body.
      product_cart: [{ product_id: providerProductId, quantity: 1 }],
      customer: { email, name: name || undefined },
      ...(returnUrl ? { return_url: returnUrl } : {}),
      metadata: {
        tier_id: tier.id,
        product_id: tier.product_id,
        environment,
      },
    });

    log('create-checkout.created', {
      requestId: id,
      tierId: tier.id,
      environment,
      // The session id is not a secret and is what the webhook correlates on.
      sessionId: session.session_id,
    });

    return json({ checkoutUrl: session.checkout_url, sessionId: session.session_id }, { origin });
  } catch (error) {
    log('create-checkout.provider-error', {
      requestId: id,
      reason: error instanceof Error ? error.message.slice(0, 120) : 'unknown',
    });
    return fail(origin, 502, 'The payment provider did not respond. Please try again.', id);
  }
});
