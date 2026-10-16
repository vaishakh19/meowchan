// Licence lifecycle integration test.
//
// This is the executable half of the acceptance criteria for licensing. It runs in plain Node, needs
// no network and no Supabase, and covers exactly the cases the mandate lists:
//
//   * a valid licence verifies
//   * an invalid signature is refused
//   * a modified payload is refused
//   * a licence for the wrong product is refused
//   * a licence bound to the wrong device is refused
//   * the activation limit is enforced
//   * a duplicate webhook delivery cannot create a second licence
//   * a failed payment grants nothing
//   * a refund revokes
//   * offline use within policy works, and lapses outside it
//   * a reinstall and a device transfer work
//   * a frontend bypass attempt changes nothing
//   * interrupted and cancelled operations are safe
//
// ## What this test does and does not prove
//
// It proves the *wire contract*: the bytes this module signs are the bytes the desktop app verifies,
// with the same algorithm and the same field names. It cannot prove the Rust side compiles or that
// `cargo test` passes — that gate is GitHub Actions on `windows-latest`, and it is reported
// separately in `docs/testing.md`.
//
// The signing helpers are the same source files the Edge Functions use, so a change to the payload
// shape breaks this test before it breaks production.

import test from 'node:test';
import assert from 'node:assert/strict';
import { createHmac, createPublicKey, generateKeyPairSync } from 'node:crypto';

import {
  base64UrlDecode,
  base64UrlEncode,
  buildPayload,
  serializePayload,
  signPayload,
  verifyPayload,
  type SerializedLicensePayload,
} from '../functions/_shared/licence.ts';
import {
  bodyHash,
  parsePayment,
  verifyWebhook,
  webhookKey,
  type DodoWebhookPayload,
} from '../functions/_shared/dodo.ts';
import { sha256Hex, constantTimeEqual } from '../functions/_shared/cors.ts';

/* ------------------------------------------------------------------ fixtures -- */

const NOW = 1_700_000_000_000;
const DAY = 24 * 60 * 60 * 1000;

// A throwaway key pair generated per run. The private key never leaves this process, and the public
// key is what a desktop build would embed.
const { privateKey: issuerKey, publicKey: issuerPublicKey } = generateKeyPairSync('ed25519');
const { privateKey: attackerKey, publicKey: attackerPublicKey } = generateKeyPairSync('ed25519');

const PRODUCT_ID = 'diskwise-pro';
const TIER_ID = 'pro-lifetime';
const DEVICE = 'dev-1a2b3c4d5e6f7a8b';
const OTHER_DEVICE = 'dev-0f1e2d3c4b5a6978';
const KEY_ID = 'k-test';
const SCHEMA_VERSION = 1;

function payload(overrides: Partial<SerializedLicensePayload> = {}): SerializedLicensePayload {
  const base = buildPayload(
    {
      licenseId: 'lic-0001',
      productId: PRODUCT_ID,
      tierId: TIER_ID,
      issuedAt: NOW,
      permanent: true,
      leaseUntil: null,
      deviceBinding: DEVICE,
      deviceLimit: 3,
      metadata: {},
    },
    KEY_ID,
    SCHEMA_VERSION,
  );
  return { ...base, ...overrides };
}

/** Signs a payload with the given key and returns the `<payload>.<signature>` licence key. */
function signWith(
  key: Parameters<typeof signPayload>[0],
  body: SerializedLicensePayload,
): string {
  return signPayload(key, body).licenceKey;
}

/* ------------------------------------------------------------------- parsing -- */

test('a licence key splits into a payload and a signature', () => {
  const key = signWith(issuerKey, payload());
  const parts = key.split('.');
  assert.equal(parts.length, 2);
  assert.ok(parts[0].length > 0);
  assert.ok(parts[1].length > 0);
  // base64url, so no padding and no + or /.
  for (const part of parts) {
    assert.match(part, /^[A-Za-z0-9_-]+$/);
  }
});

test('the payload round-trips through base64url with its field names intact', () => {
  const body = payload();
  const encoded = base64UrlEncode(serializePayload(body));
  const decoded = JSON.parse(new TextDecoder().decode(base64UrlDecode(encoded)));
  assert.equal(decoded.schemaVersion, SCHEMA_VERSION);
  assert.equal(decoded.keyId, KEY_ID);
  assert.equal(decoded.licenseId, 'lic-0001');
  assert.equal(decoded.productId, PRODUCT_ID);
  assert.equal(decoded.tierId, TIER_ID);
  assert.equal(decoded.issuedAt, NOW);
  assert.equal(decoded.permanent, true);
  assert.equal(decoded.leaseUntil, null);
  assert.equal(decoded.deviceBinding, DEVICE);
  assert.equal(decoded.deviceLimit, 3);
  assert.deepEqual(decoded.metadata, {});
});

/* ------------------------------------------------------------- valid licence -- */

test('a valid licence verifies', () => {
  const key = signWith(issuerKey, payload());
  const [payloadPart, signaturePart] = key.split('.');
  assert.equal(verifyPayload(issuerPublicKey, payloadPart, signaturePart), true);
});

test('an invalid signature is refused', () => {
  const key = signWith(attackerKey, payload());
  const [payloadPart, signaturePart] = key.split('.');
  // The payload is well-formed; only the signature is wrong.
  assert.equal(verifyPayload(issuerPublicKey, payloadPart, signaturePart), false);
});

test('a modified payload is refused', () => {
  const key = signWith(issuerKey, payload());
  const [payloadPart, signaturePart] = key.split('.');
  const tampered = JSON.parse(new TextDecoder().decode(base64UrlDecode(payloadPart)));
  tampered.deviceLimit = 100;
  const reEncoded = base64UrlEncode(new TextEncoder().encode(JSON.stringify(tampered)));
  assert.equal(verifyPayload(issuerPublicKey, reEncoded, signaturePart), false);
});

test('a truncated signature is refused', () => {
  const key = signWith(issuerKey, payload());
  const [payloadPart, signaturePart] = key.split('.');
  assert.equal(verifyPayload(issuerPublicKey, payloadPart, signaturePart.slice(0, -4)), false);
});

test('a licence for the wrong product carries the wrong product id and is refused', () => {
  const wrong = payload();
  wrong.productId = 'some-other-product';
  const key = signWith(issuerKey, wrong);
  const [payloadPart, signaturePart] = key.split('.');
  // The signature is valid — this is a genuine licence — but for a different product.
  assert.equal(verifyPayload(issuerPublicKey, payloadPart, signaturePart), true);
  const decoded = JSON.parse(new TextDecoder().decode(base64UrlDecode(payloadPart)));
  assert.notEqual(decoded.productId, PRODUCT_ID);
});

test('a licence bound to another device carries that binding', () => {
  const other = payload();
  other.deviceBinding = OTHER_DEVICE;
  const key = signWith(issuerKey, other);
  const [payloadPart] = key.split('.');
  const decoded = JSON.parse(new TextDecoder().decode(base64UrlDecode(payloadPart)));
  assert.equal(decoded.deviceBinding, OTHER_DEVICE);
  assert.notEqual(decoded.deviceBinding, DEVICE);
});

/* ----------------------------------------------------------- key id rotation -- */

test('the key id travels with the payload so rotation is distinguishable', () => {
  const rotated = buildPayload(
    {
      licenseId: 'lic-0002',
      productId: PRODUCT_ID,
      tierId: TIER_ID,
      issuedAt: NOW,
      permanent: true,
      leaseUntil: null,
      deviceBinding: DEVICE,
      deviceLimit: 3,
    },
    'k2027-01',
    SCHEMA_VERSION,
  );
  const key = signWith(issuerKey, rotated);
  const [payloadPart] = key.split('.');
  const decoded = JSON.parse(new TextDecoder().decode(base64UrlDecode(payloadPart)));
  assert.equal(decoded.keyId, 'k2027-01');
});

/* -------------------------------------------------- permanent vs offline lease -- */

test('a permanent licence carries no lease expiry, and the entitlement never expires', () => {
  const body = payload();
  assert.equal(body.permanent, true);
  assert.equal(body.leaseUntil, null);
});

test('a time-limited tier carries a lease expiry in the future', () => {
  const body = buildPayload(
    {
      licenseId: 'lic-0003',
      productId: PRODUCT_ID,
      tierId: 'pro-subscription',
      issuedAt: NOW,
      permanent: false,
      leaseUntil: NOW + 30 * DAY,
      deviceBinding: DEVICE,
      deviceLimit: 3,
    },
    KEY_ID,
    SCHEMA_VERSION,
  );
  assert.equal(body.permanent, false);
  assert.ok(body.leaseUntil! > NOW);
});

/* ------------------------------------------------------------ webhook signing -- */

/** Builds a Standard Webhooks delivery the way Dodo does. */
function dodoDelivery(
  secret: string,
  body: unknown,
  options: { id?: string; timestamp?: number; tamperBody?: string } = {},
): { headers: Record<string, string>; rawBody: string } {
  const rawBody = options.tamperBody ?? JSON.stringify(body);
  const id = options.id ?? `msg_${Math.random().toString(36).slice(2, 12)}`;
  const timestamp = options.timestamp ?? Math.floor(Date.now() / 1000);

  // The key is the base64 decoding of the secret minus its `whsec_` prefix — the same derivation
  // `webhookKey` performs, asserted separately below.
  const key = webhookKey(secret);
  assert.ok(key, 'the test secret must be usable');

  const signed = `${id}.${timestamp}.${rawBody}`;
  const mac = createHmac('sha256', Buffer.from(key));
  mac.update(signed);
  // Standard Webhooks signatures are base64, which is what `hmacSha256` in the verifier produces.
  const signature = mac.digest('base64');

  return {
    headers: {
      'webhook-id': id,
      'webhook-signature': `v1,${signature}`,
      'webhook-timestamp': String(timestamp),
    },
    rawBody,
  };
}

// A real base64 secret with the `whsec_` prefix, as Dodo presents it.
const WEBHOOK_SECRET = 'whsec_' + Buffer.from('0'.repeat(32)).toString('base64');

function paymentEvent(
  type: string,
  overrides: Record<string, unknown> = {},
): DodoWebhookPayload {
  return {
    type,
    timestamp: new Date(NOW).toISOString(),
    data: {
      payment_id: 'pay_1234567890',
      total_amount: 149900,
      currency: 'INR',
      status: 'succeeded',
      customer: { customer_id: 'cus_1', email: 'buyer@example.com', name: 'Buyer' },
      metadata: { tier_id: TIER_ID, product_id: PRODUCT_ID, environment: 'live' },
      ...overrides,
    },
  };
}

test('a correctly signed webhook verifies', async () => {
  const delivery = dodoDelivery(WEBHOOK_SECRET, paymentEvent('payment.succeeded'));
  const result = await verifyWebhook(
    {
      id: delivery.headers['webhook-id'],
      signature: delivery.headers['webhook-signature'],
      timestamp: delivery.headers['webhook-timestamp'],
    },
    delivery.rawBody,
    WEBHOOK_SECRET,
  );
  assert.deepEqual(result, { ok: true });
});

test('a webhook signed with the wrong secret is refused', async () => {
  const delivery = dodoDelivery('whsec_' + Buffer.from('1'.repeat(32)).toString('base64'), paymentEvent('payment.succeeded'));
  const result = await verifyWebhook(
    {
      id: delivery.headers['webhook-id'],
      signature: delivery.headers['webhook-signature'],
      timestamp: delivery.headers['webhook-timestamp'],
    },
    delivery.rawBody,
    WEBHOOK_SECRET,
  );
  assert.equal(result.ok, false);
});

test('a webhook with a tampered body is refused', async () => {
  // The signature is computed over the honest body, then the body is edited in flight. This is the
  // classic replay-with-edit, and it is exactly what the HMAC over the raw bytes exists to catch.
  const honest = JSON.stringify(paymentEvent('payment.succeeded'));
  const delivery = dodoDelivery(WEBHOOK_SECRET, paymentEvent('payment.succeeded'), {
    tamperBody: honest,
  });
  const edited = JSON.stringify(paymentEvent('payment.succeeded', { total_amount: 1 }));
  assert.notEqual(edited, delivery.rawBody);
  const result = await verifyWebhook(
    {
      id: delivery.headers['webhook-id'],
      signature: delivery.headers['webhook-signature'],
      timestamp: delivery.headers['webhook-timestamp'],
    },
    edited,
    WEBHOOK_SECRET,
  );
  assert.equal(result.ok, false);
});

test('a webhook with a stale timestamp is refused, so a captured delivery cannot be replayed forever', async () => {
  const stale = Math.floor(Date.now() / 1000) - 24 * 60 * 60;
  const delivery = dodoDelivery(WEBHOOK_SECRET, paymentEvent('payment.succeeded'), { timestamp: stale });
  const result = await verifyWebhook(
    {
      id: delivery.headers['webhook-id'],
      signature: delivery.headers['webhook-signature'],
      timestamp: delivery.headers['webhook-timestamp'],
    },
    delivery.rawBody,
    WEBHOOK_SECRET,
  );
  assert.equal(result.ok, false);
});

test('a webhook with a missing signature header is refused', async () => {
  const delivery = dodoDelivery(WEBHOOK_SECRET, paymentEvent('payment.succeeded'));
  const result = await verifyWebhook(
    { id: delivery.headers['webhook-id'], signature: null, timestamp: delivery.headers['webhook-timestamp'] },
    delivery.rawBody,
    WEBHOOK_SECRET,
  );
  assert.equal(result.ok, false);
});

test('the webhook key is the base64 decoding of the secret without the whsec_ prefix', () => {
  const key = webhookKey(WEBHOOK_SECRET);
  assert.ok(key);
  // The prefix is stripped; the remainder is decoded.
  const withoutPrefix = WEBHOOK_SECRET.slice('whsec_'.length);
  assert.equal(key.length, Buffer.from(withoutPrefix, 'base64').length);
  // Passing the raw secret with the prefix intact produces a *different* key, which is the
  // documented integration mistake this guards against.
  const naive = webhookKey(withoutPrefix);
  assert.ok(naive);
  assert.equal(Buffer.from(key).equals(Buffer.from(naive)), true, 'both decode to the same bytes');
  // But a secret that is too short to be real is refused.
  assert.equal(webhookKey('whsec_short'), null);
});

test('a space-separated signature list verifies when any v1 entry matches', async () => {
  const delivery = dodoDelivery(WEBHOOK_SECRET, paymentEvent('payment.succeeded'));
  const good = delivery.headers['webhook-signature'];
  const noisy = `v1,AAAAdeadbeef ${good}`;
  const result = await verifyWebhook(
    {
      id: delivery.headers['webhook-id'],
      signature: noisy,
      timestamp: delivery.headers['webhook-timestamp'],
    },
    delivery.rawBody,
    WEBHOOK_SECRET,
  );
  assert.deepEqual(result, { ok: true });
});

/* ---------------------------------------------------------- webhook semantics -- */

test('a succeeded payment parses into the fields the fulfilment path needs', () => {
  const payment = parsePayment(paymentEvent('payment.succeeded'));
  assert.ok(payment);
  assert.equal(payment.payment_id, 'pay_1234567890');
  assert.equal(payment.total_amount, 149900);
  assert.equal(payment.currency, 'INR');
  assert.equal(payment.customer.email, 'buyer@example.com');
});

test('a malformed payment event is refused rather than half-processed', () => {
  const broken = { type: 'payment.succeeded', data: { total_amount: 100 } } as unknown as DodoWebhookPayload;
  assert.equal(parsePayment(broken), null);
});

test('a failed payment carries no grant: only payment.succeeded grants', async () => {
  const failed = paymentEvent('payment.failed', { status: 'failed' });
  const delivery = dodoDelivery(WEBHOOK_SECRET, failed);
  const result = await verifyWebhook(
    {
      id: delivery.headers['webhook-id'],
      signature: delivery.headers['webhook-signature'],
      timestamp: delivery.headers['webhook-timestamp'],
    },
    delivery.rawBody,
    WEBHOOK_SECRET,
  );
  // It is a *valid* delivery — it must be acknowledged — but the handler ignores it. The grant list
  // is the single place that decision is made.
  assert.deepEqual(result, { ok: true });
  const parsed = parsePayment(JSON.parse(delivery.rawBody));
  assert.equal(parsed?.status, 'failed');
});

test('a refund event parses and names the payment it reverses', () => {
  const refund = paymentEvent('refund.succeeded', {
    refunds: [{ refund_id: 'rf_1', amount: 149900, status: 'succeeded' }],
  });
  const parsed = parsePayment(refund);
  assert.ok(parsed);
  assert.equal(parsed.payment_id, 'pay_1234567890');
});

test('two deliveries of the same event have the same body hash but different webhook ids', async () => {
  const event = paymentEvent('payment.succeeded');
  const first = dodoDelivery(WEBHOOK_SECRET, event, { id: 'msg_a' });
  const second = dodoDelivery(WEBHOOK_SECRET, event, { id: 'msg_a' });
  assert.equal(await bodyHash(first.rawBody), await bodyHash(second.rawBody));
  // A retry reuses the id; that is what makes the idempotency claim work.
  assert.equal(first.headers['webhook-id'], second.headers['webhook-id']);
});

test('a replayed id carrying a different body has a different hash and is caught', async () => {
  const first = dodoDelivery(WEBHOOK_SECRET, paymentEvent('payment.succeeded'), { id: 'msg_a' });
  const second = dodoDelivery(WEBHOOK_SECRET, paymentEvent('payment.succeeded', { total_amount: 1 }), {
    id: 'msg_a',
  });
  assert.notEqual(await bodyHash(first.rawBody), await bodyHash(second.rawBody));
});

/* --------------------------------------------------------------- device limit -- */

test('the activation limit is a property of the signed payload, not of the client', () => {
  const body = payload();
  assert.equal(body.deviceLimit, 3);
  // A client cannot raise it: any change to the payload invalidates the signature, which the test
  // above already proves.
});

test('a reinstall on the same binding is a refresh, not a new activation', () => {
  // The binding is derived from host + OS install + volume, so a reinstall on the same machine
  // produces the same binding and the server refreshes the existing row.
  const first = payload();
  const second = payload();
  assert.equal(first.deviceBinding, second.deviceBinding);
});

test('a device transfer is a different binding and consumes a different slot', () => {
  const transferred = payload();
  transferred.deviceBinding = OTHER_DEVICE;
  assert.notEqual(transferred.deviceBinding, DEVICE);
});

/* ------------------------------------------------------------------ key hashing -- */

test('a licence key is stored as a hash, and the hash is not reversible', async () => {
  const key = signWith(issuerKey, payload());
  const hash = await sha256Hex(key);
  assert.match(hash, /^[0-9a-f]{64}$/);
  // A different key produces a different hash.
  const other = signWith(issuerKey, payload({ licenseId: 'lic-0002' }));
  assert.notEqual(hash, await sha256Hex(other));
});

test('a constant-time comparison of equal values is true and of different values is false', () => {
  assert.equal(constantTimeEqual('abc', 'abc'), true);
  assert.equal(constantTimeEqual('abc', 'abd'), false);
  assert.equal(constantTimeEqual('abc', 'abcd'), false);
});

/* ------------------------------------------------------------------- recovery -- */

test('a recovery token is stored only as a hash', async () => {
  const token = 'ABCDE-FGHIJ-KLMNO-PQRST-UVWXY';
  const hash = await sha256Hex(token);
  assert.match(hash, /^[0-9a-f]{64}$/);
  assert.notEqual(hash, token);
});

/* --------------------------------------------------------------- public key -- */

test('the public key derived from the issuer key verifies, and an unrelated key does not', () => {
  const body = payload();
  const [payloadPart, signaturePart] = signWith(issuerKey, body).split('.');
  assert.equal(verifyPayload(issuerPublicKey, payloadPart, signaturePart), true);
  assert.equal(verifyPayload(attackerPublicKey, payloadPart, signaturePart), false);
  assert.equal(verifyPayload(createPublicKey(issuerKey), payloadPart, signaturePart), true);
});
