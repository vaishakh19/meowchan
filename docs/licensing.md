# Licensing and entitlement

Diskwise is freemium and sold as a **one-time purchase**. Two rules shape everything below:

1. **Basic usefulness never requires payment.** Scanning, visualization, the large-file finder, basic
   duplicate detection and every safety guarantee work forever with no account and no internet
   connection.
2. **A licence check never blocks core scanning.** Every failure path resolves to the free tier with
   a human-readable reason, and the app keeps working.

---

## 1. The commercial model

One purchase, one licence, lifetime entitlement. There is **no mandatory subscription**.

| | |
| --- | --- |
| Product | Diskwise Pro |
| Tier | `pro-lifetime` |
| Price | **14,999 INR** (1,499.00 in the smallest currency unit) |
| Payment | One-time, through Dodo Payments |
| Devices | **3 simultaneous activations** |
| Entitlement | Permanent — it never expires |
| Updates | Included for the life of the product |
| Offline lease | 30 days (see §4) |

Those numbers live in exactly **one** place: `src-tauri/src/config/product.rs` (`DEFAULT_CONFIG`),
with the server-side twin in `supabase/migrations/0002_rls_and_functions.sql`. Nothing else in the
codebase may hardcode a price, a device limit or an entitlement period, and the config is validated
on load so a typo cannot silently grant access. Changing any of them is a reviewed change to those
two files, plus a rebuild and a redeploy.

### Permanent ownership vs. a temporary offline lease

These are two different things and the code keeps them apart deliberately:

- **`permanent: true`** — the *entitlement* never expires. A customer who paid once keeps the paid
  features for the life of the product. Nothing in this system silently ends it.
- **`leaseUntil`** — the *offline validation lease*: how long a previously verified licence keeps
  working with no re-validation. It exists so a refunded or chargebacked licence can be withdrawn
  without requiring an internet connection to keep working.

A lapsed lease on a permanent licence is reported as **"this licence needs to be re-verified; your
purchase is not affected"** — never as an expiry. The distinction is enforced in
`entitlement_from_payload`, `apply_offline_grace` and `Authorization::resolve`, and is asserted by
tests on both sides (`supabase/tests/licence-lifecycle.test.ts`,
`src-tauri/tests/licence_lifecycle.rs`, `src/features/licensing/entitlements.test.ts`).

---

## 2. The free tier is a complete product

Free, with no account and no internet connection:

- Unlimited manual scans of any drive or folder.
- Sunburst, treemap and file-list views with full navigation.
- Large-file finder with size, extension and age filters.
- Duplicate detection for the first **25 groups**, largest first, with real SHA-256 and byte
  verification.
- Cleanup **previews** for the folders you choose — every path, every size, before anything happens.
- Copy path and Reveal in File Explorer.
- Every safety guarantee: explicit confirmation, Recycle Bin by default, revalidation immediately
  before acting, partial-failure reporting.

The group limit is applied **in the backend**, where the report is produced, and the report totals
are recomputed exactly so the UI never shows a size the backend cannot account for. A modified
interface cannot ask for more.

---

## 3. Where entitlement is decided

**In Rust.** `src-tauri/src/authz/` is the only place a paid capability is granted.

```
src-tauri/src/
  config/product.rs     the single commercial configuration, validated on load
  authz/permissions.rs  the complete permission catalogue, free and paid
  authz/mod.rs          authorize() — the one gate every protected command goes through
  licensing/payload.rs  the signed payload: parse, verify, encode
  licensing/verify.rs   entitlement resolution, offline grace
  commands/*.rs         each protected command calls authorize() before doing anything
```

The frontend **reads** the entitlement object so it can render the right UI. It never decides
anything. Specifically:

- `cleanup_execute` calls `authorize(PaidCleanup)` first, then `authorize(PermanentDeletion)` for
  any item that bypasses the Recycle Bin, then `authorize(BulkFileOperations)` past 50 paths — all
  inside the native backend, before a single path is prepared.
- `find_duplicates` applies the free group limit inside the blocking worker that builds the report.
- The executor's approved-roots check does not consult the entitlement at all: it is a filesystem
  safety property, not a commercial one, and it applies to every tier.

A modified React interface cannot bypass any of this by invoking a Tauri command directly, because
the command itself performs the check against an entitlement the backend resolved from a signed
payload. The webview contributes nothing to that proof.

### Permission catalogue

| Permission | Tier | What it gates |
| --- | --- | --- |
| `CoreAnalysis` | free | Scanning, aggregation, visualization, navigation |
| `FileInspection` | free | Metadata, Reveal in Explorer, open, copy path |
| `PreviewOnly` | free | Cleanup and duplicate previews |
| `LargeFileFinder` | free | The large-file finder, with free-tier limits |
| `BasicDuplicates` | free | Duplicate detection, with the free group limit |
| `PaidCleanup` | **paid** | Executing any cleanup plan |
| `PermanentDeletion` | **paid** | Bypassing the Recycle Bin |
| `DuplicateRemoval` | **paid** | Removing duplicate copies |
| `BulkFileOperations` | **paid** | Plans touching more than 50 paths |
| `ExportReports` | **paid** | CSV / JSON / HTML export |
| `ScheduledScans` | **paid** | Scheduled scans |
| `CustomCleanupRules` | **paid** | Rules beyond the built-in suggestions |
| `AdvancedFilters` | **paid** | Advanced type and age analytics |
| `ScanHistory` | **paid** | Saved scans and historical comparison |
| `UnlimitedDuplicates` | **paid** | Duplicate groups past the free limit |

`PERMISSIONS` in `authz/permissions.rs` is the complete list and the tests iterate it, so a new
capability cannot be added without deciding explicitly whether it is free or paid.

---

## 4. How a licence is verified

### The key format

A licence key is `<base64url(payload JSON)>.<base64url(Ed25519 signature)>`. The payload:

```json
{
  "schemaVersion": 1,
  "keyId": "k2026-01",
  "licenseId": "b1f2...",
  "productId": "diskwise-pro",
  "tierId": "pro-lifetime",
  "issuedAt": 1735689600000,
  "permanent": true,
  "leaseUntil": null,
  "deviceBinding": "dev-1a2b3c4d5e6f7a8b",
  "deviceLimit": 3,
  "metadata": {}
}
```

`schemaVersion` and `keyId` are what make rotation safe: an old licence keeps verifying after a new
key is introduced, and a licence signed by a key this build does not trust is refused with a message
that says to update rather than "invalid".

### Verification order

`payload::verify` runs these checks in this order, and each one fails closed:

1. **Shape** — is it `payload.signature` at all?
2. **Schema** — does this build understand `schemaVersion`?
3. **Key identity** — is `keyId` a trusted, active key? The signature is then checked against the
   key the payload *names*, so a payload cannot pick a weaker key.
4. **Signature** — Ed25519 over the exact payload bytes.
5. **Product** — does `productId` match this build's product?
6. **Tier** — is `tierId` a tier this build sells, and is it still available?
7. **Issued-at** — not more than 5 minutes in the future (clock skew).
8. **Device-limit sanity** — not zero, not absurd.
9. **Device binding** — does `deviceBinding` match this machine?
10. **Lease** — has `leaseUntil` passed?

Every error has a customer-facing `message()` that names no key id, no public key, no signature and
no internal path. A test asserts this for every fixture case.

### What is not a security boundary

The **frontend is not a security boundary**, and neither is anything it sends:

- a browser redirect from the payment provider,
- a query parameter on the return URL,
- an email address,
- a client-supplied order or payment id,
- an unverified webhook body.

None of these can grant an entitlement. Only a signature-verified `payment.succeeded` webhook does
that, and only a locally-verified signature can unlock a paid command.

### Device binding

`DeviceBinding::current(data_dir)` is `dev-` + an FNV-1a hash of
`hostname | OS install date | data-directory volume`.

**A machine identifier is not a secret.** It is a binding, not a credential: knowing it does not let
anyone use a licence, because the licence key is still required and the key is what the signature
covers. It is derived from stable machine properties so a reinstall on the same machine produces the
same binding — which is what makes a reinstall free (see §7).

The OS install date is read from the `InstallDate` registry value via `reg query`, rather than
pulling in a registry crate that could not be compile-checked in every environment.

---

## 5. The backend

Supabase Postgres + Edge Functions. Everything in `supabase/`.

### Schema

| Table | Written by | Purpose |
| --- | --- | --- |
| `products` | service role | The product being sold |
| `entitlement_configs` | service role | Price, currency, permanence, device limit, lease — the server-side twin of the Rust config |
| `orders` | service role | One row per payment, unique on `provider_payment_id` |
| `licences` | service role | One row per paid order, unique on `order_id`, key stored as SHA-256 |
| `device_activations` | service role | The authoritative device ledger |
| `webhook_events` | service role | Idempotency, keyed on `webhook-id` |
| `refunds` | service role | Provider refund references |
| `revocations` | service role | Every revocation with its reason |
| `audit_log` | service role | Append-only; ids, event names, short notes — never paths or keys |
| `recovery_tokens` | service role | Hashed, single-use, short-lived recovery tokens |
| `rate_limits` | service role | Fixed-window counters |

Row Level Security is enabled on every table and **no policy grants `INSERT`, `UPDATE` or `DELETE`
to `anon` or `authenticated`**. The desktop application does not read these tables directly at all:
it verifies licences offline with an embedded public key and calls Edge Functions that authenticate
with the service-role key. A licence key is a bearer token, so letting a client `SELECT` from
`licences` would turn any leaked anon key into a key dump.

### Edge Functions

| Function | Called by | Does |
| --- | --- | --- |
| `create-checkout` | the web | Builds the product cart from `entitlement_configs` and creates a Dodo checkout session. Grants nothing. |
| `dodo-webhook` | Dodo Payments | Verifies the signature, claims the id, then grants or revokes. The only granting path. |
| `licence-activate` | the desktop app | Looks the key up by hash, enforces the device limit atomically, mints and returns a signed payload. |
| `licence-deactivate` | the desktop app | Releases the slot for the calling device only. |
| `licence-recover` | the web / desktop | Two-step, account-independent recovery (§7). |

### Dodo Payments integration

Dodo follows the **Standard Webhooks** specification. The details that are easy to get wrong, all
verified against the current official documentation:

- Headers: `webhook-id`, `webhook-signature`, `webhook-timestamp`.
- The signed content is `"{webhook-id}.{webhook-timestamp}.{rawBody}"`, joined by periods.
- The HMAC-SHA256 key is the **base64 decoding** of the webhook secret **with the `whsec_` prefix
  stripped**. Passing the raw `whsec_...` string produces signatures that never verify.
- `webhook-signature` is a space-separated list of `v1,<signature>` entries; any one verifying is
  sufficient.
- The raw body must be the exact bytes Dodo sent. Re-serialising parsed JSON changes the key
  whitespace and breaks the signature, so every handler reads `request.text()` once.
- Dodo retries until it receives a 2xx, so every handler must be idempotent.
- The documentation is explicit that **the browser redirect is not the source of truth** — the
  webhook is.

Checkout creation is `POST https://test.dodopayments.com/checkouts` (test) or
`https://live.dodopayments.com/checkouts` (live), with `Authorization: Bearer $DODO_PAYMENTS_API_KEY`
and a body of `{product_cart, customer, return_url, metadata}`. The response is
`{session_id, checkout_url}`.

### Idempotency and duplicate delivery

Every delivery is claimed by its `webhook-id` **before** any work happens:

1. `claim_webhook` inserts the id; a conflict means this exact delivery has been seen.
2. A duplicate is acknowledged with `2xx {received: true, duplicate: true}` and does nothing.
3. A delivery that reuses an id with a **different** body has a different `body_hash` and is refused,
   not silently ignored.
4. The response goes out **before** processing, because Dodo's timeout is shorter than the work and a
   timeout causes another retry.

On the granting path, `orders.provider_payment_id` is unique and `licences.order_id` is unique, so a
replayed `payment.succeeded` — even with a fresh webhook id — cannot create a second order or a
second licence for the same payment.

### Test mode cannot grant a production entitlement

Three independent guards:

1. The shipped `DEFAULT_CONFIG` has `environment: Test`. A build straight off the default branch
   mints nothing, and a CI job asserts the trusted key is still the documented all-zero placeholder.
2. `create-checkout` reads `DODO_ENVIRONMENT` and never accepts it from the request.
3. The webhook handler checks the payment's `environment` metadata: a non-live payment is recorded
   in `orders` for the audit trail and **no licence is issued**.

Switching to live mode is a deliberate, reviewed release action — see §9.

---

## 6. Refunds, chargebacks and revocation

| Event | Effect |
| --- | --- |
| `refund.succeeded` | A `refunds` row, the order moves to `refunded`, the licence moves to `refunded` with `status_reason = 'refund'`, and a `revocations` row records why. |
| `dispute.accepted` / `dispute.lost` | Same, with `source = 'dispute'`. |
| `dispute.opened` / `challenged` / `expired` / `won` / `cancelled` | Recorded in `webhook_events`, acknowledged, and **deliberately ignored**: a challenge is not a loss. |
| `payment.failed` / `processing` / `cancelled` / `charge_failed` | Acknowledged and ignored. No entitlement. |
| `refund.failed` | Acknowledged and ignored: the refund did not happen. |

A revoked licence is **kept, never deleted**, and `revocations` preserves the reason. The desktop app
refuses activation for any status other than `active`, and the offline lease bounds how long a
previously-activated licence keeps working after a revocation.

---

## 7. Device limit, transfer and recovery

### The limit is enforced atomically, in the database

`activate_device()` in `supabase/migrations/0002_rls_and_functions.sql` takes `SELECT ... FOR UPDATE`
on the licence row, counts the active devices, and only then inserts. Two concurrent activations
queue behind each other, so the count cannot be read twice at the same value. A check-then-insert in
application code would not be safe, and is not what this does.

The CI `migrations` job seeds a licence with a limit of one, activates it twice against a real
Postgres, and asserts the second activation raises `check_violation`.

### Reinstall, hardware change and transfer

| Situation | Binding | Outcome |
| --- | --- | --- |
| Reinstall on the same machine | unchanged | The activation row is **refreshed**. No slot consumed. |
| Replace the machine | new | A new activation. Must be within the limit. |
| Move the licence deliberately | new | Deactivate the old device first (§8), then activate the new one. |

Recovery is **account-independent**: a customer who lost their key does not need to remember which
email they used, and does not need to pay again.

1. `POST licence-recover {action: "request", email, orderRef?}` — a single-use, 30-minute recovery
   token is created; only its SHA-256 is stored. The response is identical whether or not the email
   matches an order, so the endpoint cannot be used to enumerate customers.
2. The link is delivered out of band. The transport is behind one function,
   `deliverRecoveryLink`, which is **not implemented in this repository** — there is no mail
   credential here and the code does not pretend to send anything. See §9.
3. `POST licence-recover {action: "claim", token, deviceBinding}` — the token is consumed with a
   conditional update (so a double-submit cannot mint twice), the device is activated through the
   same atomic path, and a fresh signed payload is returned.

The token is rate-limited, capped at 10 attempts, and a revoked licence is never recoverable.

---

## 8. Using the app

**Activate.** Licensing → paste the key → Activate. The key is verified entirely offline. Nothing is
uploaded: no key, no machine identifier, no file paths.

**See your devices.** Licensing → Devices. This is a local mirror of the service's ledger, so it
works offline and shows exactly what the app can see. The service remains authoritative.

**Deactivate this device.** Licensing → Deactivate. Releases the slot locally and asks the service to
release it too. Only the device you are using can be deactivated from here — releasing somebody
else's device would be a way to defeat the limit, and that goes through the account page.

**Re-verify.** Connect to the internet and re-activate. This refreshes the lease. It is never
required to keep scanning.

---

## 9. Operations

### Generating a signing key

```bash
node scripts/licence-keys.mjs keygen k2026-01
```

The script prints the **public** key as base64 (paste it into
`CommercialConfig::trusted_keys` in `src-tauri/src/config/product.rs`) and never prints the private
key. A lost private key is rotated, not recovered.

Store the private key in the licence service's secret store:

```bash
supabase secrets set LICENCE_SIGNING_PRIVATE_KEY="$(cat key.pem)"
```

### Secrets

| Secret | Where | Committed? |
| --- | --- | --- |
| `LICENCE_SIGNING_PRIVATE_KEY` | Supabase secret | **Never** |
| `DODO_PAYMENTS_API_KEY` | Supabase secret | **Never** |
| `DODO_WEBHOOK_SECRET` | Supabase secret | **Never** |
| `SUPABASE_SERVICE_ROLE_KEY` | Supabase secret | **Never** |
| `LICENCE_SIGNING_KEY_ID` | Supabase secret | Yes, it is not a secret |
| `DODO_ENVIRONMENT` | Supabase secret | Yes |
| `RECOVERY_EMAIL_PROVIDER` | Supabase secret | Yes |

None of these appear in the app, the frontend bundle, the installer, this repository, the logs or
any crash report. The desktop build carries **only the public key**, which is not a secret. CI greps
the built bundle and the tracked code for the shapes these secrets take and fails if it finds one.

### Configuring recovery delivery

`deliverRecoveryLink` in `supabase/functions/licence-recover/index.ts` is the single place the
recovery email is sent. It currently checks for `RECOVERY_EMAIL_PROVIDER` and, when it is unset,
logs and returns without sending — which means recovery is **not functional out of the box**. Setting
up a mail provider and filling in that function is a deployment task, documented here rather than
faked.

### Creating a Dodo test product

1. Sign in to the Dodo Payments dashboard and switch to **Test mode**.
2. Create a one-time product, note its `product_id`.
3. Set `DODO_PRODUCT_PRO_LIFETIME` to that id in the Supabase secrets.
4. Add a webhook endpoint pointing at `https://<project>.functions.supabase.co/dodo-webhook` and
   subscribe it to the events listed in §6.
5. Copy the signing secret into `DODO_WEBHOOK_SECRET`. It arrives as `whsec_...`; the code strips the
   prefix and base64-decodes the rest.

### Making a test purchase

Use the test-mode checkout, complete the payment, and watch `dodo-webhook` in the Supabase logs. A
`payment.succeeded` in test mode records an order and issues **no licence** — that is the guard in
§5 working. To exercise the granting path locally, mint a licence with
`node scripts/licence-keys.mjs mint --device <binding>` using a throwaway key pair, and verify it
with `node scripts/licence-keys.mjs verify`.

### Switching to live mode

This is the one change that turns test payments into real entitlements. Do all of it, in order:

1. Generate a **new** signing key pair. Do not reuse the test key.
2. Put the new public key in `CommercialConfig::trusted_keys` with a new `key_id` and
   `active: true`, keep the old key with `active: false` so already-issued licences keep verifying.
3. Set `LICENCE_SIGNING_PRIVATE_KEY` and `LICENCE_SIGNING_KEY_ID` to the new pair.
4. Set `environment: Live` in `DEFAULT_CONFIG`.
5. Set `DODO_ENVIRONMENT=live` and point `DODO_PRODUCT_PRO_LIFETIME` at the live product.
6. Deploy the Edge Functions, then the desktop build.
7. Run a real purchase and confirm exactly one licence row appears.

A build with `environment: Test` can never mint a real entitlement, so step 4 is the only step that
can go wrong silently. It is a code change, which means it is reviewed.

### Key rotation and compromise recovery

**Rotation.** Add the new key with `active: true` and a new `key_id`; set the old one to
`active: false`. Both verify; new licences carry the new `key_id`. Because `key_id` travels in the
payload, a licence signed by a retired key is told apart from one signed by an untrusted key.

**Compromise.** If the private key leaks, an attacker can mint licences that verify offline. There is
no way to un-sign them, and it would be dishonest to claim otherwise:

1. Set the compromised key to `active: false` and ship a build. Existing licences fall back to free
   with "update Diskwise".
2. Issue replacement licences under a new key to every affected customer, using the recovery flow —
   no one pays twice.
3. Audit `audit_log` and `device_activations` for activations that do not correspond to real
   customers.

**Honest limitation.** Local signature verification is **not** DRM. It stops a casual edit of a
settings file and it makes forging a licence require the private key. It does not prevent copying the
app, patching the binary, or reverse engineering it, and it is not represented as doing so.

### Backup and disaster recovery

- **Postgres** — Supabase's daily backups plus Point-in-Time Recovery. `licences` and
  `device_activations` are the only irreplaceable state: everything else can be rebuilt from the
  payment provider's records.
- **The signing key** — stored in the secret store only. Losing it means rotating, not recovering.
- **The desktop app** — if the service is unreachable, activation and recovery fail with "activation
  is unavailable right now". Previously activated customers are unaffected for the length of their
  offline lease, and scanning never stops.

---

## 10. Testing

| Test | Where | Runs |
| --- | --- | --- |
| Valid licence verifies | `supabase/tests/licence-lifecycle.test.ts`, `src-tauri/tests/licence_lifecycle.rs` | local + CI |
| Invalid signature refused | both | local + CI |
| Modified payload refused | both | local + CI |
| Wrong product / wrong device / unknown tier / unknown schema refused | both | local + CI |
| Lapsed lease refused, live lease accepted | both | local + CI |
| Absurd device limit refused | both | local + CI |
| Malformed and empty keys refused | both | local + CI |
| Webhook signature verification | `supabase/tests/licence-lifecycle.test.ts` | local + CI |
| Webhook replay with a stale timestamp refused | same | local + CI |
| Webhook replay with an edited body refused | same | local + CI |
| `whsec_` prefix handling | same | local + CI |
| Duplicate delivery cannot create a second licence | `webhook_events` + unique constraints; CI `migrations` job | CI |
| Device limit enforced atomically | CI `migrations` job against a real Postgres | CI |
| No client write policy exists | CI `migrations` job | CI |
| RLS enabled on every licensing table | CI `migrations` job | CI |
| A forged licence grants nothing | `src-tauri/tests/licence_lifecycle.rs` | CI |
| A modified frontend cannot bypass authorization | `src-tauri/tests/licence_lifecycle.rs` | CI |
| A permanent licence with a lapsed lease stays paid | all three suites | local + CI |
| No secret in the bundle or the repo | CI `frontend` and `audit` jobs | CI |
| The shipped trusted key is the placeholder | CI `licence` job | CI |

**What has not been run.** The Rust tests have not been executed in the environment this was written
in — there is no Rust toolchain and no crates.io access there, so `cargo test`,
`cargo clippy` and `cargo fmt` are gated on GitHub Actions on `windows-latest`. The Supabase
migrations and Edge Functions have not been executed against a live project: there is no network
access to Supabase and no Dodo credentials. Those paths are covered by CI (`migrations`,
`licence`) and by a documented manual run in §9, and this document does not claim otherwise.

---

## 11. Where the numbers live

| Value | Location |
| --- | --- |
| Product id | `DEFAULT_CONFIG.product_id` and `products.id` |
| Tier id, name, price, currency, permanence, update eligibility | `DEFAULT_CONFIG.tiers[*]` and `entitlement_configs` |
| Device limit | `DEFAULT_CONFIG.device_limit`, `entitlement_configs.device_limit`, and the signed payload |
| Offline lease days | `DEFAULT_CONFIG.offline_lease_days` and `entitlement_configs.offline_lease_days` |
| Free duplicate group limit | `DEFAULT_CONFIG.free_duplicate_group_limit` |
| Trusted signing keys | `DEFAULT_CONFIG.trusted_keys` |
| Payload schema version | `DEFAULT_CONFIG.license_schema_version` and `licences.schema_version` |
| Environment | `DEFAULT_CONFIG.environment` and `DODO_ENVIRONMENT` |

Nothing else may contain any of these as a literal. If you find one, it is a bug.
