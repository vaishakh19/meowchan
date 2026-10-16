# Diskwise

A local-first disk space analyzer for Windows 10 and 11. It maps a whole volume with a real
sunburst and treemap, finds the files that are actually eating your disk, detects true duplicates
by content, and explains every cleanup suggestion before anything moves.

Built with **Tauri 2 + React + TypeScript + Vite + Tailwind CSS + D3 + Rust + SQLite**. No Electron.

```bash
npm install            # frontend dependencies
npm run dev            # browser preview (synthetic data, clearly labelled)
npm run app:dev        # the real desktop app (requires the Rust toolchain)
npm run app:build      # MSI + NSIS installers
npm run test:all       # typecheck + frontend tests + licence tests + build
npm run bench          # tree aggregation benchmark, prints real measurements
npm run rust:test      # Rust tests (requires the toolchain)
```

---

## What it does

| Area | What you get |
| --- | --- |
| **Scan** | Any fixed, NTFS, external or user folder. Live progress with files, folders, bytes, elapsed, current directory and ETA. Cancellable at any point. |
| **Visualize** | Genuine D3 sunburst (one ring per folder level) and squarified treemap, plus a virtualized file list. Click to select, double-click to navigate in, centre click to return to root, right-click for a safe context menu, breadcrumb and full keyboard navigation. |
| **Large files** | Filter by minimum size (>100 MB / >500 MB / >1 GB / custom), extension and last-modified window; sort largest first; reveal any result in File Explorer. |
| **Duplicates** | Staged detection: group by exact size → SHA-256 → optional byte-for-byte verification. Never by filename. Inspect groups, compare paths and sizes, select individual copies, exclude folders, see total savings. |
| **Cleanup** | Suggestions for folders *you* configure, each with its exact paths, byte totals and the reason it was flagged. Explicit confirmation, Recycle Bin by default, permanent deletion as a separate clearly-labelled action. |
| **Entitlement** | One-time purchase, lifetime licence, three devices. Resolved and enforced in Rust, cached offline. Basic usefulness never requires payment. |

## Commercial model

One-time purchase. Lifetime entitlement. Three simultaneous device activations. No mandatory
subscription, and nothing in the app silently expires a licence you paid for.

| | |
| --- | --- |
| Price | **14,999 INR**, one time |
| Devices | 3 simultaneous activations, deactivable and transferable |
| Updates | included for the life of the product |
| Offline use | a verified licence keeps working with no network for 30 days |

The one distinction worth stating plainly, because it is the one products usually blur: **the
purchase is permanent; the offline validation lease is not.** A licence that has not been re-checked
in a while says "re-verify when you can — your purchase is not affected", never "expired".

Every price, device limit and entitlement period lives in exactly one file on each side —
`src-tauri/src/config/product.rs` and `supabase/migrations/0002_rls_and_functions.sql` — and both are
validated on load.

## What it will never do

- Delete a file because it was selected or analysed.
- Auto-classify your documents, browser profiles, development dependencies or package caches as junk.
- Offer Windows system files or application data for deletion.
- Preselect every copy of a duplicate group — at least one copy is always kept.
- Upload anything. There is no account, no cloud sync, no background scanning, no full-path analytics,
  no ads, and no telemetry. The only network requests the app ever makes are the ones you trigger by
  activating, deactivating or recovering a licence, and they carry the licence key, a device binding
  and a device name — nothing about your filesystem.
- Trust the frontend as a security boundary. Every paid or destructive capability is gated inside
  Rust, before the operation runs, against an entitlement the backend resolved itself. A modified
  React interface cannot bypass it, and a forged licence cannot unlock a paid command.
- Execute a file it has analysed, or grant the webview any shell-execution permission.
- Pretend to be code-signed. The installers are unsigned; signing is a separate release task.

## Layout

```
┌──────────┬────────────────────────────────────┬──────────┐
│ Sidebar  │  Header (location, breadcrumb,     │ Details  │
│  ~240px  │  totals, counts, last scan)        │  ~280px  │
│          │  Controls (sunburst/treemap/list,  │          │
│ nav      │  sort, group, search, zoom/reset)  │ name     │
│ drives   │  Visualization                     │ path     │
│ progress │                                    │ sizes    │
│ quick    │                                    │ counts   │
│ wins     │                                    │ parent   │
│          │                                    │ dates    │
│          │                                    │ children │
└──────────┴────────────────────────────────────┴──────────┘
```

All three regions scroll independently, so the layout holds from 960px wide and at 100 / 125 / 150%
display scaling.

## Repository map

```
src/
  app/            application shell: state, layout, title bar, item menu
  components/ui/  primitives, feedback, virtual list, storage ring, context menu
  features/
    scanner/      analyze view, large files, history, quick wins
    visualization/ sunburst, treemap, file list, layout math
    duplicates/   duplicate finder
    cleanup/      cleanup assistant
    licensing/    entitlement rules and the licensing screen
    settings/     settings and privacy screens
  lib/            format, categories, paths, ids, tree pruning, transport seam
  lib/backend/    tauriTransport (the real backend) + demo (browser preview only)
  types/          the shared wire contract
src-tauri/
  src/
    commands/     the narrowly-scoped Tauri command surface
    scanner/      walker, aggregation, tree pruning, progress counters
    filesystem/   long paths, metadata snapshots, category classification
    duplicates/   size grouping, hashing, byte verification
    cleanup/      rules, findings, preview and execution
    storage/      SQLite: settings, scan history, licence ledger
    licensing/    signed payloads, offline entitlement resolution
    authz/        the one gate every protected command goes through
    config/       the centralised commercial configuration
    platform/     Win32 entry points, declared by hand
  capabilities/   the single capability the webview gets
  icons/          generated by `npm run icons`
  tests/          licence lifecycle integration test + shared fixtures
supabase/
  migrations/     the licensing schema and the atomic device-limit function
  functions/      checkout, webhook, activation, deactivation, recovery
  tests/          the licence + webhook contract suite (31 tests)
docs/             architecture, testing, installer, privacy, licensing
.github/          Windows CI: typecheck, tests, clippy, fmt, migrations, installers
scripts/          icon generator, licence keys, benchmark
```

## The transport seam

`src/lib/transport.ts` defines every operation the app can perform. Exactly two implementations
exist:

- **`tauriTransport`** — the product. Talks to Rust through narrowly-scoped Tauri commands and
  events. Selected when `window.__TAURI_INTERNALS__` is present.
- **`demo`** — a browser-only fallback that serves a deterministic synthetic filesystem so the
  interface can be reviewed and tested without Windows. It is clearly labelled in the UI
  ("Browser preview · synthetic data"), reports `canMutate: false`, refuses every destructive
  operation, and is never selected inside a packaged build.

Nothing else in `src/` imports `@tauri-apps/*`.

## Documentation

- [`docs/architecture.md`](docs/architecture.md) — how the scanner, pruning, duplicates, cleanup,
  storage and entitlement layers fit together, and why.
- [`docs/testing.md`](docs/testing.md) — what is tested, how, and what is *not* yet verified.
- [`docs/installer.md`](docs/installer.md) — building the MSI and NSIS installers, and the signing
  situation.
- [`docs/privacy.md`](docs/privacy.md) — exactly what is stored locally and what leaves the device.
- [`docs/licensing.md`](docs/licensing.md) — the entitlement model, the payment provider, device
  limits and recovery, key rotation, and what has *not* been executed.
- [`supabase/README.md`](supabase/README.md) — the licence service: schema, functions, security model.

## Verification status

Honest summary of what has and has not been proven in this repository:

| Claim | Status |
| --- | --- |
| Frontend typechecks (`tsc --noEmit`) | ✅ verified here |
| Frontend test suite (173 tests) | ✅ verified here |
| Production build (`vite build`) | ✅ verified here |
| Licence + webhook contract suite (31 tests) | ✅ verified here — runs in plain Node, no network |
| Browser preview runs and drives a full scan lifecycle | ✅ verified here (jsdom) |
| Tree aggregation benchmark over 250 000 generated files | ✅ measured here — see [`docs/testing.md`](docs/testing.md) |
| Rust compiles, `cargo test`, `cargo clippy`, `cargo fmt` | ⚠️ **not verified here** — no Rust toolchain is obtainable in this sandbox. Gated on GitHub Actions `windows-latest`. |
| Rust licence-lifecycle integration test | ⚠️ **not verified here** — same reason. It reads the same fixtures the Node suite passes. |
| Rust aggregation benchmark (`cargo run --bin bench`) | ⚠️ **not verified here** — same reason. |
| MSI / NSIS installers | ⚠️ **not verified here** — produced by `npm run app:build` on Windows. |
| Supabase migrations applied to a real Postgres | ⚠️ **not verified here** — no network access to Supabase. Gated on the CI `migrations` job, which also asserts the device limit is enforced and no client policy exists. |
| Edge Functions executed | ⚠️ **not verified here** — no Supabase project and no Dodo credentials. Their shared signing and webhook-verification code *is* covered by the 31 contract tests. |
| A real Dodo test payment yielding exactly one entitlement | ⚠️ **not verified here** — requires credentials. Documented as a manual step in [`docs/licensing.md`](docs/licensing.md). |
| Code signing | ❌ intentionally not done. Signing is a separate release task. |
| Licence recovery email delivery | ❌ **not implemented.** `deliverRecoveryLink` is a documented seam; no mail provider is configured. |

See [`docs/testing.md`](docs/testing.md) for the full picture and the exact commands.

## Licence

MIT.
