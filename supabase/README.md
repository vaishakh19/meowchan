# The Diskwise licence service

Supabase Postgres + Edge Functions. This directory is the **only** place a Diskwise entitlement is
granted, and the only place a payment is proved.

It is not part of the desktop application. The app verifies licences offline with an embedded public
key and calls these functions for exactly two things that genuinely need a server: recording a device
activation against an atomic limit, and recovering a lost key.

Full operational detail is in [`../docs/licensing.md`](../docs/licensing.md). This file is the map.

## Layout

```
migrations/
  0001_licensing_schema.sql   tables, constraints, unique indexes, seed rows
  0002_rls_and_functions.sql  row level security + the atomic device-limit function
functions/
  _shared/
    cors.ts       CORS allow-list, generic error shape, body caps, hashing, logging
    dodo.ts       Standard Webhooks verification + checkout creation + payload parsing
    licence.ts    the licence payload shape and Ed25519 signing
  create-checkout/    builds the product cart from the database, creates a checkout session
  dodo-webhook/       verifies, claims the id, then grants or revokes
  licence-activate/   hash lookup, atomic device limit, mints a signed payload
  licence-deactivate/ releases the calling device's slot only
  licence-recover/    two-step, account-independent recovery
tests/
  licence-lifecycle.test.ts   31 tests, no network, no Supabase, no credentials
```

## Quick start

```bash
# Local Postgres + the Edge Functions runtime
supabase start

# Apply the migrations
supabase db reset

# Set the secrets. None of these are committed.
supabase secrets set LICENCE_SIGNING_PRIVATE_KEY="$(cat key.pem)"
supabase secrets set LICENCE_SIGNING_KEY_ID=k2026-01
supabase secrets set DODO_PAYMENTS_API_KEY=...
supabase secrets set DODO_WEBHOOK_SECRET=whsec_...
supabase secrets set DODO_ENVIRONMENT=test
supabase secrets set DODO_PRODUCT_PRO_LIFETIME=...

# Serve the functions locally
supabase functions serve
```

## Tests

```bash
npm run test:licence
```

31 tests covering the licence payload, the Ed25519 signature, the Standard Webhooks construction,
replay and tampering, the `whsec_` prefix handling, and the device-limit semantics. They run against
the **same source files** the Edge Functions import, so a change to the payload shape fails here
before it fails in production.

The Rust twin, `../src-tauri/tests/licence_lifecycle.rs`, reads the same fixtures
(`../src-tauri/tests/fixtures/licences.json`), which `scripts/make-licence-fixtures.mjs` generates.
CI regenerates them and fails if they are stale.

## Security model, in one paragraph

Row Level Security is enabled on every table and **no policy grants `INSERT`, `UPDATE` or `DELETE`
to `anon` or `authenticated`**. There is no client-readable policy either, because a licence key is a
bearer token and letting a client `SELECT` from `licences` would turn any leaked anon key into a key
dump. The desktop app does not read these tables at all. The `migrations` CI job applies the SQL to a
real Postgres and asserts both properties, so an accidental permissive policy fails the build.

## What is deliberately not implemented

`deliverRecoveryLink` in `functions/licence-recover/index.ts` checks for `RECOVERY_EMAIL_PROVIDER` and,
when it is unset, logs and returns without sending. Recovery is therefore **not functional out of the
box**. Wiring up a mail provider and filling in that one function is a deployment task, documented in
`../docs/licensing.md` rather than faked with a stub that pretends to send email.
