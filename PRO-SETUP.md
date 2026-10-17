# Diskora Pro — merchant setup (Dodo Payments)

Diskora Pro is a **one-time $12.99 purchase**. It unlocks safe file deletion
(Recycle Bin), reveal in File Explorer, full absolute path display and copy, and
CSV export with full paths. Scanning and every view stay free.
The integration uses Dodo Payments' **public** license endpoints, so
there is **no backend server to deploy and no API secret to ship**.
The merchant API key never leaves the Dodo Payments dashboard.

## 0. Free vs Pro, enforced in the backend

| Capability | Free | Pro |
| --- | --- | --- |
| Scan, sunburst, treemap, file list, large files, duplicates | Yes | Yes |
| Real paths reach the webview | Never | Yes |
| Masked display paths (`C:\...\Videos\clip.mp4`) | Yes | Not needed |
| Copy full path | Locked | Yes |
| Reveal in File Explorer, open a file | Locked | Yes |
| Delete to Recycle Bin (after confirmation) | Locked | Yes |
| CSV export (full paths) | Locked | Yes |

Enforcement lives in Rust, not only in the UI:

- `disclosure.rs` rewrites every outbound payload (command results, progress
  events) and redacts absolute paths in error messages while no valid
  entitlement is active. Free payloads contain identity tokens (`ref:…`) and
  masked display strings, never a real path.
- `reveal_in_explorer`, `open_path` and `export_scan_csv` refuse without an
  active entitlement. `delete_items` re-validates with Dodo before every run.
- The entitlement state is published after each verification and at startup. A
  change of tier re-reads the open scan, so no stale identity survives a switch.

## 1. Create the product in the Dodo Payments dashboard

1. Sign in at <https://dashboard.dodopayments.com> (create a merchant
   account if you do not have one).
2. **Products → Create product.** One-time price, **$12.99**.
3. Enable **license keys** for the product (one-time products support
   them). Set the activation limit per key — **2** is a sensible default
   (desktop + spare). Keys are delivered to the customer by email and on
   the checkout success page.
4. Create a **payment link** (Checkout → Payment links, or copy the
   product's hosted checkout URL). This is the static link the desktop
   app opens; it is not a secret.

## 2. Build Diskora with the configuration

The values are compiled in at build time — they are not runtime settings,
so a shipped binary cannot be silently re-pointed:

```powershell
$env:DISKORA_DODO_PRODUCT_ID   = "pdt_..."   # the product id from the dashboard
$env:DISKORA_DODO_CHECKOUT_URL = "https://..." # the hosted payment link
# Optional, defaults below are correct for production:
$env:DISKORA_DODO_API_BASE     = "https://live.dodopayments.com"
$env:DISKORA_DODO_ENVIRONMENT  = "live"      # or "test"
npm run app:build
```

Without both required variables, or when either URL is outside the allowlist,
the build compiles but Diskora Pro shows an explicit not-configured notice and
delete stays locked. The API base must be exactly the matching Dodo host
(`live.dodopayments.com` or `test.dodopayments.com`). Checkout links must use
`checkout.dodopayments.com` or `test.checkout.dodopayments.com` over HTTPS. The
checkout URL is opened by the native shell; the webview has no network permission.

## 3. Test purchases before going live

Dodo Payments has a test mode. Build with
`DISKORA_DODO_ENVIRONMENT=test` (the API base then defaults to
`https://test.dodopayments.com`) and use the dashboard's test payment
method:

1. Open **Diskora Pro → Upgrade to Pro**; the test checkout page opens.
2. Complete the test purchase; Dodo emails a test license key.
3. Paste the key into the Pro page's **Activate** box. The app calls
   Dodo's public activate endpoint, checks the response belongs to the
   configured product, and persists the entitlement locally.
4. Confirm full paths unlock (details panel, Copy path, Reveal, Delete and CSV
   export), then restart the app. A stored license is re-checked once in the
   background at startup, on the Pro page, and before every deletion.
5. Test **Restore purchases** and the offline behaviour (disable
   networking: a previously verified purchase is shown as active for up
   to 7 days, clearly labelled as offline; deletion remains disabled
   until Dodo Payments can verify the purchase again).

## 4. How verification works (and why no backend is needed)

- **Activation** — `POST {api_base}/licenses/activate` with the pasted
  key and a device label. These are Dodo's *public* endpoints: they
  authenticate with the license key itself, not a merchant API key, so
  they are safe to call from a desktop executable. Activation enforces
  the product's activation limit server-side.
- **Validation** — `POST {api_base}/licenses/validate` with the key and
  the stored activation instance id. The response's `valid` flag is the
  only thing Diskora trusts; a local file can never unlock Pro on its
  own.
- **Product binding** — activation responses are checked against the
  compiled-in product id, so a key bought for a different product
  cannot unlock Diskora Pro.
- **Persistence** — the entitlement (key, instance id, product id,
  timestamps) is stored in the app's local SQLite database
  (`pro_entitlement` table). Windows user-scoped DPAPI encrypts the whole
  entitlement before it reaches SQLite. Older plaintext rows are migrated
  to DPAPI on first read. The stored value is an input to the next
  verification, never proof by itself.
- **Grace period** — if Dodo is unreachable, a purchase verified within
  the last 7 days is shown as active, labelled "offline". After 7 days
  offline, Pro locks until a connection returns. Destructive deletion
  always requires a successful fresh Dodo validation; offline grace
  never authorizes it.

Because activation/validation are public endpoints, the smallest secure
integration is direct Rust → Dodo HTTPS. A separate licensing backend
would add a machine to deploy and a secret to protect without adding
any verification strength. Do **not** add the merchant API key to the
app or the executable — checkout-session creation and dashboard
operations stay in the Dodo dashboard.

## 5. License states the app reports

| State (`pro.state`) | Shown as | What it means | How to resolve |
| --- | --- | --- | --- |
| `not-configured` | Notice on the Pro page | This build has no product id or checkout URL | Rebuild with the variables in section 2 |
| `not-purchased` | Free, Upgrade to Pro | No license is stored on this device | Buy, then Activate or Restore |
| `active` | Pro badge | Verified with Dodo (or verified recently, see below) | None |
| `offline` | Pro, with an offline notice | Dodo was unreachable; a purchase verified within 7 days is trusted | Reconnect. Deletion still needs a live check |
| `unverified` | Free, "needs a fresh check" | Offline for more than 7 days | Connect and press Restore purchases |
| `invalid` | Free, "no longer valid" | Dodo says the key is revoked, refunded, expired or over its activation limit. The stored key is removed | Check the key, or contact support |

A deletion attempt while offline fails with a message to reconnect. It is never
allowed on grace alone.

## 6. Fulfilment notes

- Customers receive their key from Dodo (email + success page). The
  app's "paste your key" activation is therefore the primary flow;
  a static payment link cannot hand a key to a desktop app
  automatically.
- Refunds and revocations take effect on the next successful server
  validation: opening the Pro page, Restore purchases, startup (background
  re-check), or a deletion attempt (deletion always re-validates first).
  Startup shows the cached state at once, so an offline start is never
  blocked.
- To change the price or product, create the new product in the
  dashboard and rebuild with the new `DISKORA_DODO_PRODUCT_ID` and
  `DISKORA_DODO_CHECKOUT_URL`.
