#!/usr/bin/env node
// Signing-key and licence management, for the vendor's back office only.
//
// ## This script is not part of the product
//
// It exists so a human can do three things:
//
//   node scripts/licence-keys.mjs keygen            — print a new key pair
//   node scripts/licence-keys.mjs mint <key> ...    — sign a licence for one device
//   node scripts/licence-keys.mjs verify <key>      — check a licence against a public key
//
// ## Why it is safe to be in this repository
//
// It holds no key. The private key is read from `DISKWISE_LICENCE_PRIVATE_KEY` in the environment
// and is never written to disk, never printed, and never committed. The public key it prints is
// meant to be pasted into `src-tauri/src/config/product.rs`.
//
// ## What it must never be used for
//
// Generating a licence for a customer who has not paid. That path is the verified Dodo webhook, and
// nothing else.

import { createPrivateKey, createPublicKey, generateKeyPairSync, sign, verify } from 'node:crypto';
import { buildPayload, loadPrivateKey, publicKeyPem, signPayload, verifyPayload } from '../supabase/functions/_shared/licence.ts';

const DAY = 24 * 60 * 60 * 1000;

function usage() {
  process.stderr.write(
    [
      'Usage:',
      '  node scripts/licence-keys.mjs keygen [keyId]',
      '  node scripts/licence-keys.mjs mint --device <binding> [--license-id <id>] [--tier <id>]',
      '      [--product <id>] [--limit <n>] [--lease-days <n>] [--not-permanent] [--key-id <id>]',
      '  node scripts/licence-keys.mjs verify <licenceKey> --public-key <pem>',
      '',
      'The private key comes from DISKWISE_LICENCE_PRIVATE_KEY. It is never printed.',
      '',
    ].join('\n'),
  );
  process.exit(2);
}

/**
 * Parses `--name value` pairs.
 *
 * A value may legitimately begin with `--` (a PEM header), so the only rule is "the next argument is
 * the value unless it is missing".
 */
function parseFlags(args) {
  const flags = {};
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (!arg.startsWith('--')) usage();
    const key = arg.slice(2);
    const value = args[index + 1];
    if (value === undefined) usage();
    flags[key] = value;
    index += 1;
  }
  return flags;
}

function keygen(keyId) {
  const { privateKey, publicKey } = generateKeyPairSync('ed25519');
  const privatePem = privateKey.export({ format: 'pem', type: 'pkcs8' }).toString();
  const publicPem = publicKeyPem(createPrivateKey(privatePem));

  // The public key the desktop build embeds is the base64 of the raw 32-byte key, which is what
  // `ed25519_dalek::VerifyingKey` accepts. Strip the DER header rather than shipping the PEM.
  const spki = createPublicKey(publicPem).export({ format: 'der', type: 'spki' });
  const raw = spki.subarray(spki.length - 32);
  const base64 = raw.toString('base64');

  process.stdout.write('# Key id: ' + (keyId ?? 'k' + new Date().toISOString().slice(0, 7)) + '\n');
  process.stdout.write('#\n');
  process.stdout.write('# PRIVATE KEY — store in the licence service secret store. Never commit.\n');
  process.stdout.write('# Public key (base64, for CommercialConfig::trusted_keys):\n');
  process.stdout.write(base64 + '\n');
  process.stdout.write('#\n');
  process.stdout.write('# To set it in Supabase, without it touching your shell history:\n');
  process.stdout.write('#   supabase secrets set LICENCE_SIGNING_PRIVATE_KEY="$(cat key.pem)"\n');
  process.stdout.write('#\n');
  process.stdout.write('# The private key itself is deliberately NOT printed by this script.\n');
  process.stdout.write('# Re-run keygen if you lost it; a lost key is rotated, not recovered.\n');
}

function mint(args) {
  const flags = parseFlags(args);
  const pem = process.env.DISKWISE_LICENCE_PRIVATE_KEY;
  if (!pem) {
    process.stderr.write('DISKWISE_LICENCE_PRIVATE_KEY is not set.\n');
    process.exit(1);
  }
  const device = flags.device;
  if (!device) usage();

  const privateKey = loadPrivateKey(pem);
  const keyId = flags['key-id'] ?? 'k2026-01';
  const permanent = flags['not-permanent'] !== 'true';
  const leaseDays = Number(flags['lease-days'] ?? 30);
  const now = Date.now();

  const payload = buildPayload(
    {
      licenseId: flags['license-id'] ?? 'lic-' + now.toString(36),
      productId: flags.product ?? 'diskwise-pro',
      tierId: flags.tier ?? 'pro-lifetime',
      issuedAt: now,
      permanent,
      leaseUntil: permanent ? null : now + leaseDays * DAY,
      deviceBinding: device,
      deviceLimit: Number(flags.limit ?? 3),
      metadata: {},
    },
    keyId,
    Number(flags['schema-version'] ?? 1),
  );

  const signed = signPayload(privateKey, payload);
  process.stdout.write(signed.licenceKey + '\n');
}

function verifyKey(args) {
  // The licence key is the one argument that is not a flag; the public key is a multi-line PEM.
  const positional = [];
  const flagArgs = [];
  for (let index = 0; index < args.length; index += 1) {
    if (args[index].startsWith('--')) {
      flagArgs.push(args[index], args[index + 1]);
      index += 1;
    } else {
      positional.push(args[index]);
    }
  }
  const flags = parseFlags(flagArgs);
  const licenceKey = positional[0];
  const publicPem = flags['public-key'];
  if (!licenceKey || !publicPem) usage();

  const [payloadPart, signaturePart] = licenceKey.split('.');
  const ok = verifyPayload(createPublicKey(publicPem), payloadPart, signaturePart);
  if (!ok) {
    process.stderr.write('INVALID — the signature does not verify.\n');
    process.exit(1);
  }
  const decoded = JSON.parse(Buffer.from(payloadPart.replace(/-/g, '+').replace(/_/g, '/'), 'base64').toString('utf8'));
  process.stdout.write(JSON.stringify(decoded, null, 2) + '\n');
}

const [command, ...rest] = process.argv.slice(2);
switch (command) {
  case 'keygen':
    keygen(rest[0]);
    break;
  case 'mint':
    mint(rest);
    break;
  case 'verify':
    verifyKey(rest);
    break;
  default:
    usage();
}
