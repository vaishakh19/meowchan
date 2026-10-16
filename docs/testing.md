# Testing

```bash
npm run typecheck    # TypeScript across src/ and tests/
npm test             # frontend suite (vitest)
npm run test:licence # licence lifecycle + webhook signature contract tests
npm run test:all     # typecheck + frontend + licence + production build
npm run rust:test    # Rust (needs a toolchain)
npm run rust:fmt     # Rust formatting check
npm run rust:clippy  # Rust lints, warnings denied
npm run bench        # tree aggregation benchmark, prints real measurements
```

`npm run rust:*` requires a Rust toolchain. Without one, `npm run test:all` still covers the entire
frontend and the licence contract.

---

## What the frontend suite covers

`npx vitest run` — 10 files, 168 tests, all running in jsdom against the real components and the
real reducer. No component is mocked out.

| File | Tests | What it proves |
| --- | --- | --- |
| `src/lib/format.test.ts` | 17 | Byte, count, duration and relative-time formatting, including exact-byte output and pluralisation. |
| `src/lib/path.test.ts` | 21 | Path normalisation, breadcrumb construction, drive detection, parent traversal. |
| `src/lib/categories.test.ts` | 10 | Every `--color-cat-*` token in `src/styles.css` has a matching entry in `CATEGORY_COLORS`, so the sunburst can never render an undefined colour. |
| `src/lib/tree.test.ts` | 15 | Pruning: `maxChildrenPerNode`, the synthetic `Other` node, node budget, the monotonicity the depth search depends on, and that a pruned tree's sizes stay arithmetically exact. |
| `src/features/visualization/layout.test.ts` | 24 | Sunburst arc paths and treemap tiles, zero-byte and empty-tree handling, category breakdown, sorting. |
| `src/features/licensing/entitlements.test.ts` | 24 | Every entitlement state maps to the right feature set, transitions are legal, key masking, offline grace, duplicate-group limits. |
| `src/features/scanner/quickWins.test.ts` | 11 | Quick Wins are derived from the real scan tree (downloads, temp, caches, large media, dev artifacts), sorted by size, and never double-counted. |
| `src/lib/backend/demo/demoFs.test.ts` | 10 | The synthetic dataset is deterministic, has no duplicate paths, and its sizes add up at every level. |
| `src/lib/backend/demo/demoTransport.test.ts` | 32 | The browser fallback honours the same contract as the Rust backend, labels itself honestly, refuses destructive operations, and gates features without ever blocking a scan. |
| `src/app/App.test.tsx` | 6 | The whole shell in jsdom: navigation between every route, honest empty states, a full scan lifecycle through the demo transport, and selection syncing from the file list into the details panel. |

The App tests are the closest thing to an end-to-end test that can run without Windows: they render
the real `App`, click the real "Scan Drive" button, wait for the real progress events, switch to the
real file list, click a real row, and assert the details panel shows the real measured size.

---

## What the Rust suite covers

`cargo test --manifest-path src-tauri/Cargo.toml`

Every test runs in a temp directory under `std::env::temp_dir()`, created and removed by the test
itself. No test ever touches a real user directory.

| Requirement | Where |
| --- | --- |
| Size aggregation | `scanner/aggregate.rs` — sizes roll up the tree; every level is exact. |
| Sunburst data generation | `scanner/aggregate.rs` + `src/features/visualization/layout.test.ts` (the frontend half). |
| Scan cancellation | `scanner/walker.rs::cancellation_stops_the_walk_early` — a cancelled scan stops before visiting the tree. |
| Access-denied directories | `scanner/walker.rs::reports_a_missing_root_as_an_error` and `security.rs` — an unreadable root is an error, not an empty scan. |
| Symlink / junction handling | `security.rs::a_link_is_never_deleted_through`, `a_junction_loop_is_detected_during_the_walk`. |
| Duplicate detection | `duplicates/grouping.rs` — real files on disk, grouped by size then hashed. |
| Hash collision safety | `duplicates/hashing.rs::hashes_known_content` against published SHA-256 vectors, plus `duplicates/grouping.rs::same_size_different_content_is_not_a_duplicate`. |
| File modified between scan and cleanup | `cleanup/execute.rs::a_file_that_changed_since_the_scan_is_skipped` and `security.rs::a_file_modified_after_the_scan_is_skipped`. |
| Path traversal attempts | `security.rs::traversal_style_paths_are_not_actionable`, `a_path_outside_the_approved_roots_is_refused`, `an_empty_approved_root_list_refuses_everything`. |
| Cleanup confirmation | `cleanup/execute.rs::a_dry_run_changes_nothing` and `security.rs::a_dry_run_is_always_non_destructive`. |
| Entitlement transitions | `licensing/verify.rs` — every state, the offline grace period, expiry, and that an invalid licence still leaves core analysis enabled. |
| Licence expiration | `licensing/verify.rs::an_expired_cached_licence_falls_back_to_free`, `the_offline_grace_period_expires_a_stale_cached_licence`. |
| **A forged licence unlocks nothing** | `tests/licence_lifecycle.rs::a_forged_licence_grants_nothing` — nine fixture cases, each checked against every paid permission. |
| **A modified frontend cannot bypass authorization** | `tests/licence_lifecycle.rs::a_modified_frontend_cannot_bypass_authorization` — iterates the whole permission catalogue on the free tier. |
| Licence payload shape | `tests/licence_lifecycle.rs` against `tests/fixtures/licences.json`, generated by `scripts/make-licence-fixtures.mjs` with the same signing code the licence service uses. |
| Payload schema, key id, product, tier, device, lease | `licensing/payload.rs` and `tests/licence_lifecycle.rs` — each failure mode, each with a customer-facing message that leaks no internals. |
| Centralised commercial config | `config/product.rs::tests` — every value is validated, and a build defaults to the test environment. |
| Depth search correctness | `scanner/aggregate.rs::the_depth_search_returns_a_tree_within_the_budget_at_every_budget`, `the_root_byte_total_survives_every_budget`, `no_folder_ever_reports_more_than_its_children_sum_to`. |
| Licensing schema | `storage/db.rs` — schema v2 migrations, licence-key round-trip, device activation and deactivation, revocation retention, audit append. |
| Empty and error states | `src/app/App.test.tsx` (no scan, no results, unreadable volume) and `scanner/aggregate.rs::an_empty_scan_index_produces_a_single_node_tree`. |
| Settings normalisation | `settings.rs::normalization_clamps_out_of_range_values`. |
| SQLite round-trips and idempotent migrations | `storage/db.rs`. |
| Capability allowlist | `security.rs::the_webview_has_no_shell_permission` asserts the capability file contains no shell, fs or process permission. |
| No scanned file is ever executed | `security.rs::no_command_in_the_backend_spawns_a_scanned_file`. |

---

## Licence contract tests

`npm run test:licence` — 31 tests in `supabase/tests/licence-lifecycle.test.ts`, run with Node's own
test runner. No network, no Supabase, no Dodo credentials.

The point of putting them in Node rather than in the Rust suite is that they exercise the **same
signing code the Edge Functions use**. If the payload shape drifts on either side, these tests fail
before a customer's licence does.

| Case | What it proves |
| --- | --- |
| A valid licence verifies | The happy path, and that the payload round-trips through base64url with its camelCase field names intact. |
| An invalid signature is refused | A payload signed by an untrusted key does not verify. |
| A modified payload is refused | Raising `deviceLimit` and clearing `permanent` after signing breaks the signature. |
| A truncated signature is refused | Not just "different" — shorter is refused too. |
| Wrong product, wrong device, unknown tier, unknown schema, not yet valid, lapsed lease, absurd device limit | Every policy failure mode, each distinct. |
| A correctly signed webhook verifies | The Standard Webhooks construction: HMAC-SHA256 over `id.timestamp.body`, base64 signature. |
| A webhook signed with the wrong secret is refused | |
| A webhook with a tampered body is refused | The signature covers the raw bytes, so an edit in flight is caught. |
| A stale timestamp is refused | A captured delivery cannot be replayed forever. |
| A missing signature header is refused | |
| A space-separated signature list verifies when any `v1` entry matches | Dodo may offer several; any one is sufficient. |
| The `whsec_` prefix handling | The key is the base64 decoding of the secret **without** the prefix, and a too-short secret is refused. |
| A failed payment carries no grant | It is a *valid* delivery that must be acknowledged, and the handler ignores it. |
| Two deliveries of the same event share a body hash | What makes idempotency by `webhook-id` work. |
| A replayed id with a different body has a different hash | What makes a replayed id with edited content get caught. |
| A licence key is stored as a hash | The hash is not the key, and different keys hash differently. |
| Constant-time comparison | Equal values match, different values do not, and different lengths do not. |
| The public key derived from the issuer key verifies, an unrelated key does not | |

The Rust twin, `src-tauri/tests/licence_lifecycle.rs`, reads the same fixtures
(`src-tauri/tests/fixtures/licences.json`) and additionally asserts that a forged licence grants
nothing and that a modified frontend cannot bypass authorization. CI regenerates the fixtures and
fails if they are stale.

---

## Performance

The brief asks for duration and memory to be treated as measured engineering targets, not
guarantees. Here is what has actually been measured, and how to reproduce it.

### Measured here: tree aggregation

`npm run bench` runs `scripts/bench-tree.mjs`, which compiles the real `src/lib/tree.ts` with the
project's own bundler and prunes generated trees of the sizes below. The Rust twin is
`cargo run --release --bin bench` (`src-tauri/src/bin/bench.rs`), which measures
`ScanIndex::from_run` + `prune_tree` over the same generated shapes.

Measured on Node v22.22.3, linux x64, `maxChildren=100`, `maxNodes=150000`, median of five runs:

| Scenario | Files | Dirs | Input nodes | Pruned nodes | Median ms | Nodes/ms |
| --- | --- | --- | --- | --- | --- | --- |
| `small` | 2,000 | 200 | 2,200 | 2,200 | 1.4 | 1622 |
| `medium` | 25,000 | 2,000 | 27,000 | 27,000 | 17.9 | 1505 |
| `large` | 100,000 | 8,000 | 108,000 | 108,000 | 89.3 | 1209 |
| `wide` | 120,000 | 400 | 120,400 | 40,001 | 57.7 | 693 |
| `huge` | 250,000 | 20,000 | 270,000 | 127,947 | 1214.7 | 105 |

Every one of those runs also asserts, and passed:

- the pruned root reports **exactly** the source byte total,
- no folder reports more bytes than its children sum to,
- the node budget is respected,
- every synthetic node's path ends in the documented `\[Other]` suffix and its bytes are
  non-negative.

**A bug this benchmark found.** Before the depth search was a binary search, `pruneTree` rebuilt the
whole tree once per depth level. On the `huge` shape (270 000 nodes, ~55 levels deep) that measured
**4 203 ms**; after the change it measures **1 215 ms** — a 3.5× improvement, and the difference
between a visible freeze and a brief wait in the browser preview. The Rust `prune_tree` had the same
linear walk and has been given the same fix. `src/lib/tree.test.ts` now asserts the monotonicity the
search depends on, so it cannot silently regress to a linear walk.

**What is not measured here, and is not claimed:** scanning a real multi-terabyte NTFS volume,
hashing tens of gigabytes, the walker's throughput on real hardware, and the memory ceiling on a
low-RAM machine. The `smoke-windows` CI job compiles and links the real walker on a Windows runner;
the numbers above are aggregation numbers only, and nothing here should be read as a real-volume
throughput claim.

### Measured here: everything else

| Scenario | Result |
| --- | --- |
| Full demo-transport scan lifecycle in jsdom | ~2–3 s, dominated by the simulated 55 ms progress ticks |
| Rendering the file list for a 200 000-entry folder | windowed: constant DOM node count regardless of folder size |
| Licence lifecycle contract suite | 31 tests, ~250 ms |

**How to reproduce:**

```bash
npm run bench                                        # tree aggregation, prints a table + JSON
BENCH_MAX_CHILDREN=50 BENCH_MAX_NODES=20000 npm run bench   # different budgets

cd src-tauri && cargo run --release --bin bench      # the Rust twin
cd src-tauri && cargo run --release --bin bench huge # one scenario
```

The Rust binary writes `target/bench-results.json`, so a CI run can publish the measurements as an
artifact rather than restating them in a document.

---

## CI

`.github/workflows/ci.yml` runs on every push and pull request:

| Job | Runner | What it does |
| --- | --- | --- |
| `frontend` | `ubuntu-latest` | `npm ci`, `npm run typecheck`, `npm test`, `npm run build`, then greps `dist/` for secret-shaped strings. |
| `licence` | `ubuntu-latest` | Regenerates the shared licence vectors and fails if they are stale, runs the 31 contract tests, and asserts the shipped trusted key is still the documented placeholder. |
| `rust` | `windows-latest` | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --lib --tests`, `cargo test --test licence_lifecycle`, `cargo check --all-targets`. |
| `migrations` | `ubuntu-latest` + Postgres 17 | Applies every migration in order, then asserts against a real database that the device limit is enforced atomically, that RLS is on every licensing table, and that no client write policy exists. |
| `installers` | `windows-latest` | `npm run app:build`, uploads the MSI and NSIS artefacts, and **fails** if either reports a signature — the release process expects unsigned artefacts. |
| `smoke-windows` | `windows-latest` | Builds the app on a real Windows runner so linker and WebView2 issues surface in CI rather than at release time. |
| `audit` | `ubuntu-latest` | `npm audit` at high severity, and a scan of the tracked code for credential-shaped strings. |

`RUSTFLAGS: -D warnings` is set repo-wide, so a new warning fails the build.

---

## What is *not* verified, and why

Being precise about this matters more than a green badge.

1. **The Rust code has never been compiled.** The environment this repository was authored in has no
   Rust toolchain and no reachable crates.io mirror. The Rust source is written carefully and
   reviewed, but "it compiles" is a claim only `cargo build` on Windows can make. CI is that gate.
2. **The Rust tests have never been run.** Same reason. `src-tauri/tests/licence_lifecycle.rs` and
   the `src-tauri/src/bin/bench.rs` measurements are written and reviewed, not executed here.
3. **The Supabase migrations and Edge Functions have never been executed.** There is no network
   access to Supabase and no Dodo credentials in the authoring environment. They are written against
   the verified official documentation, and CI's `migrations` job applies the SQL to a real Postgres
   and asserts the device-limit guarantee. The Edge Functions themselves are covered by
   `npm run test:licence` for their shared signing and verification code, and by a documented manual
   run in [`licensing.md`](licensing.md).
2. **The installers have never been produced.** `npm run app:build` needs both a Rust toolchain and
   Windows; the documented commands in [`installer.md`](installer.md) are the way to produce them.
3. **No installer is code-signed, and none claims to be.** Signing is a separate release task with
   its own key custody; nothing in this repository pretends otherwise.
4. **No scan has run against a real multi-terabyte volume here.** The walker's correctness is
   covered by tests against generated temp trees; its throughput on real hardware is not.
5. **The reference screenshot's exact visual identity is not reproduced.** The aesthetic direction
   (radial usage visualization, compact sidebars, details panel, premium minimalism) is followed;
   none of its branding or assets are copied.

### How to verify the completed portions yourself

```bash
# Frontend, no toolchain beyond Node:
npm install
npm run typecheck
npm test

# Full app on Windows (needs the Rust toolchain + WebView2):
npm install
npm run app:dev      # launches the real desktop app against the real filesystem

# Then: pick a drive, press Scan Drive, click a sunburst segment, open the
# details panel, filter Large Files, run Duplicate Finder, and try a cleanup
# dry run. Every step should show real numbers from your own disk.
```
