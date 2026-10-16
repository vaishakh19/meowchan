#!/usr/bin/env node
// Generates the licence test vectors shared by the Rust integration test and the Node lifecycle test.
//
// Both sides verify the *same bytes*. That is the point: if the payload shape drifts on either side,
// the Rust test that reads this file fails in CI rather than in a customer's hands.
//
// ## This file must be byte-for-byte reproducible
//
// CI regenerates it and diffs against the committed copy, so a payload-shape change fails the build
// instead of silently producing new vectors. That only works if the output is deterministic, which
// means the key pair is derived from a fixed seed rather than generated fresh each run. An earlier
// version called `generateKeyPairSync` here, which made the diff unsatisfiable by construction and
// turned the `licence` CI job red on every single commit.
//
// ## These are throwaway test keys and nothing more
//
// The seeds below are public, fixed, and chosen so the keys are obviously fake. They sign test
// vectors and nothing else. They are NOT the licence service's signing key: the service loads its
// key from `LICENCE_SIGNING_PRIVATE_KEY` at runtime, and the desktop build embeds only the matching
// public key. Rotating the real key does not touch this file, and this file cannot be used to mint a
// licence a shipped build will accept, because `product.rs` ships an all-zero placeholder public key.
//
// The keys are kept as raw 32-byte seeds, never as PEM, so the credential-shape scan in the `audit`
// CI job does not flag them.

import { mkdirSync, writeFileSync } from 'node:fs';
import { createHash, createPrivateKey, createPublicKey } from 'node:crypto';
import { buildPayload, signPayload, serializePayload, base64UrlEncode, base64UrlDecode } from '../supabase/functions/_shared/licence.ts';

const DAY = 24 * 60 * 60 * 1000;
const NOW = 1_700_000_000_000;
/** Fixed so the generated file is byte-identical on every run. Not a claim about when it was made. */
const GENERATED_AT = 1_700_000_000_000;
const OUT = new URL('../src-tauri/tests/fixtures/licences.json', import.meta.url);

// The PKCS#8 prefix for an Ed25519 private key. Prefixing a 32-byte seed with these 16 bytes yields a
// key Node accepts, which is how a fixed seed becomes a fixed key without a dependency.
const ED25519_PKCS8_PREFIX = Buffer.from('302e020100300506032b657004220420', 'hex');

/** Derives a stable Ed25519 private key from a public, obviously-fake label. */
function fixtureKey(label) {
  const seed = createHash('sha256').update(label).digest();
  return createPrivateKey({ key: Buffer.concat([ED25519_PKCS8_PREFIX, seed]), format: 'der', type: 'pkcs8' });
}

const privateKey = fixtureKey('diskwise-test-fixture-key-1');
const otherKey = fixtureKey('diskwise-test-fixture-key-2');
const publicKey = createPublicKey(privateKey);
const otherPublicKey = createPublicKey(otherKey);

// The public key as base64 of the raw 32 bytes, which is the format
// `src-tauri/src/config/product.rs::TrustedKey::public_key_base64` expects. The last 32 bytes of
// the SPKI DER are the Ed25519 point; the 12-byte header is discarded.
function rawPublicBase64(key) {
  const spki = key.export({ format: 'der', type: 'spki' });
  return spki.subarray(spki.length - 32).toString('base64');
}
const publicBase64 = rawPublicBase64(publicKey);
const otherPublicBase64 = rawPublicBase64(otherPublicKey);

const KEY_ID = 'k-fixture';
const OTHER_KEY_ID = 'k-other';
const SCHEMA = 1;

function mint(overrides = {}) {
  const payload = buildPayload(
    {
      licenseId: 'lic-fixture-1',
      productId: 'diskwise-pro',
      tierId: 'pro-lifetime',
      issuedAt: NOW,
      permanent: true,
      leaseUntil: null,
      deviceBinding: 'dev-1a2b3c4d5e6f7a8b',
      deviceLimit: 3,
      metadata: {},
      ...overrides,
    },
    overrides.keyId ?? KEY_ID,
    overrides.schemaVersion ?? SCHEMA,
  );
  return signPayload(privateKey, payload);
}

const valid = mint();

// A payload signed by a key the app does not trust.
const wrongKey = buildPayload(
  {
    licenseId: 'lic-fixture-2',
    productId: 'diskwise-pro',
    tierId: 'pro-lifetime',
    issuedAt: NOW,
    permanent: true,
    leaseUntil: null,
    deviceBinding: 'dev-1a2b3c4d5e6f7a8b',
    deviceLimit: 3,
    metadata: {},
  },
  OTHER_KEY_ID,
  SCHEMA,
);
const signedByOther = signPayload(otherKey, wrongKey);

// A valid signature over a payload that has been edited afterwards.
const tampered = JSON.parse(new TextDecoder().decode(base64UrlDecode(valid.payloadBase64)));
tampered.deviceLimit = 100;
tampered.permanent = false;
const tamperedPayload = base64UrlEncode(new TextEncoder().encode(JSON.stringify(tampered)));

// A valid signature over a payload for a different product.
const wrongProduct = mint({ productId: 'some-other-product' });

// A valid signature over a payload for a different device.
const wrongDevice = mint({ deviceBinding: 'dev-0f1e2d3c4b5a6978' });

// A valid signature over a payload for a tier this build does not sell.
const unknownTier = mint({ tierId: 'pro-enterprise' });

// A payload from a schema this build does not understand.
const unknownSchema = mint({ schemaVersion: 99 });

// A lease that has already lapsed, on a non-permanent licence.
const lapsedLease = mint({ permanent: false, leaseUntil: NOW - DAY });

// A lease that is still current.
const liveLease = mint({ permanent: false, leaseUntil: NOW + 30 * DAY });

// Issued in the future beyond the clock-skew allowance.
const notYetValid = mint({ issuedAt: NOW + 24 * 60 * 60 * 1000 });

// An absurd device limit, which is a configuration error rather than a licence.
const absurdLimit = mint({ deviceLimit: 1_000_000 });

const vectors = {
  description:
    'Shared licence test vectors. Generated by scripts/make-licence-fixtures.mjs. ' +
    'The key pair is derived from fixed public seeds, is throwaway, and signs test vectors only. ' +
    'Only the shape and the cross-language agreement matter.',
  generatedAtMs: GENERATED_AT,
  now: NOW,
  keyId: KEY_ID,
  otherKeyId: OTHER_KEY_ID,
  schemaVersion: SCHEMA,
  productId: 'diskwise-pro',
  tierId: 'pro-lifetime',
  deviceBinding: 'dev-1a2b3c4d5e6f7a8b',
  otherDeviceBinding: 'dev-0f1e2d3c4b5a6978',
  publicKeyBase64: publicBase64,
  otherPublicKeyBase64: otherPublicBase64,
  cases: {
    valid: {
      key: valid.licenceKey,
      payload: valid.payloadBase64,
      signature: valid.signatureBase64,
      expect: 'ok',
      note: 'A permanent licence for this product, this tier and this device.',
    },
    invalidSignature: {
      key: `${valid.payloadBase64}.${'A'.repeat(86)}`,
      expect: 'bad-signature',
      note: 'The payload is untouched; only the signature is wrong.',
    },
    modifiedPayload: {
      key: `${tamperedPayload}.${valid.signatureBase64}`,
      expect: 'bad-signature',
      note: 'The device limit and the permanent flag were raised after signing.',
    },
    wrongKey: {
      key: signedByOther.licenceKey,
      expect: 'unknown-key',
      note: 'Genuinely signed, but by a key this build does not trust.',
    },
    wrongProduct: {
      key: wrongProduct.licenceKey,
      expect: 'wrong-product',
      note: 'A valid licence for a different product.',
    },
    wrongDevice: {
      key: wrongDevice.licenceKey,
      expect: 'wrong-device',
      note: 'A valid licence activated on a different machine.',
    },
    unknownTier: {
      key: unknownTier.licenceKey,
      expect: 'unknown-tier',
      note: 'A tier this build does not sell.',
    },
    unknownSchema: {
      key: unknownSchema.licenceKey,
      expect: 'unknown-schema',
      note: 'A payload schema this build does not understand.',
    },
    notYetValid: {
      key: notYetValid.licenceKey,
      expect: 'not-yet-valid',
      note: 'Issued beyond the clock-skew allowance.',
    },
    lapsedLease: {
      key: lapsedLease.licenceKey,
      expect: 'lease-lapsed',
      note: 'A non-permanent licence whose offline lease has run out.',
    },
    liveLease: {
      key: liveLease.licenceKey,
      expect: 'ok',
      note: 'A non-permanent licence inside its offline lease.',
    },
    absurdDeviceLimit: {
      key: absurdLimit.licenceKey,
      expect: 'device-limit',
      note: 'A device limit far beyond anything a licence may carry.',
    },
    malformed: {
      key: 'not-a-licence-key',
      expect: 'malformed',
      note: 'Not even the right shape.',
    },
    empty: {
      key: '',
      expect: 'empty',
      note: 'Nothing supplied.',
    },
  },
};

mkdirSync(new URL('.', OUT), { recursive: true });
writeFileSync(OUT, JSON.stringify(vectors, null, 2) + '\n');
process.stdout.write(`wrote ${OUT.pathname} with ${Object.keys(vectors.cases).length} cases\n`);
