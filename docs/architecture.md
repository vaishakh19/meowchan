# Architecture

Diskwise is three layers with a hard boundary between the UI and the filesystem. The webview can
only reach the filesystem through the command surface in `src-tauri/src/commands/`; there is no
filesystem scope and no shell-execution permission in the capability file.

```
┌─────────────────────────────────────────────────────────────────────┐
│ Webview (React + TypeScript)                                        │
│   src/app, src/components, src/features, src/lib                    │
│   never imports @tauri-apps/* directly                              │
└───────────────────────────┬─────────────────────────────────────────┘
                            │  BackendTransport  (src/lib/transport.ts)
                            │  camelCase in, snake_case out
┌───────────────────────────▼─────────────────────────────────────────┐
│ Tauri command surface  (src-tauri/src/commands/)                    │
│   list_drives, scan_*, list_large_files, file_details,              │
│   reveal_in_explorer, open_path, find_duplicates,                   │
│   duplicates_get, verify_duplicates, cleanup_*,                     │
│   recycle_bin_summary, entitlement_*, license_*,                    │
│   device_activations, device_deactivate, settings_*,                │
│   privacy_report, app_version                                       │
└───────────────────────────┬─────────────────────────────────────────┘
                            │  every protected command calls authorize() first
┌───────────────────────────▼─────────────────────────────────────────┐
│ authz        the one gate: 15 permissions, free and paid            │
│   authorize(auth, permission) -> Ok(()) | Err(customer message)     │
└───────────────────────────┬─────────────────────────────────────────┘
                            │
┌───────────────────────────▼─────────────────────────────────────────┐
│ Rust modules                                                         │
│   platform    Win32 entry points, declared by hand                  │
│   filesystem  long paths, metadata snapshots, classification        │
│   scanner     walker, aggregation, tree pruning, progress           │
│   duplicates  size grouping, hashing, byte verification             │
│   cleanup     rules, findings, preview, execution                   │
│   storage     SQLite: settings, scan history, licence ledger        │
│   licensing   signed payloads, offline entitlement resolution       │
│   config      the centralised commercial configuration              │
└─────────────────────────────────────────────────────────────────────┘
                            │
┌───────────────────────────▼─────────────────────────────────────────┐
│ Supabase (supabase/)  — the licence service, never the product path │
│   migrations  products, entitlement_configs, orders, licences,      │
│               device_activations, webhook_events, refunds,          │
│               revocations, audit_log, recovery_tokens, rate_limits  │
│   functions   create-checkout, dodo-webhook, licence-activate,      │
│               licence-deactivate, licence-recover                   │
└─────────────────────────────────────────────────────────────────────┘
```

The app does not talk to Supabase to *verify* a licence — that is Ed25519, offline, against a public
key compiled into the binary. It talks to Supabase for exactly two things that genuinely need a
server: recording a device activation against an atomic limit, and recovering a lost key. Both are
opt-in user actions.

Nothing depends on a module above it. The layering is what makes each service independently testable:
`cargo test` exercises the walker, the aggregator, the pruner, the duplicate pipeline and the
cleanup executor without a webview, and `vitest` exercises the reducer, the visualizations and the
navigation without Rust.

---

## 1. The scanner

`src-tauri/src/scanner/walker.rs`

A hand-written walker over `std::fs`, because the product needs three things an external crate does
not give together: a work queue with a bounded worker pool, per-directory error capture, and
cancellation checks at entry granularity.

- **Queue.** `Mutex<VecDeque<(PathBuf, usize)>>` + `Condvar`. Workers block while the queue is empty
  but work remains, and exit when it is drained.
- **Concurrency.** `worker_count()` is `available_parallelism()` clamped to `2..=16`. Bounded so a
  pathological tree cannot exhaust handles.
- **Traversal order.** Breadth-first, because progress and ETA are only meaningful once several
  directories have been visited.
- **Links.** `symlink_metadata` is used everywhere, so links are never followed by default. A link is
  recorded and skipped. Following is opt-in per scan.
- **Cycle prevention.** A directory is only queued once, keyed on its canonical path. A repeat is
  recorded as a `symlink-cycle` error.
- **Errors.** Permission denials, missing paths, long paths and vanished files are collected into
  `WalkError { path, kind, message }` and surfaced in the UI. Nothing is silently skipped.
- **Long paths.** Every path is normalised to the `\\?\` form before it touches the filesystem
  (`src-tauri/src/filesystem/long_path.rs`), so trees deeper than `MAX_PATH` work.
- **Cancellation.** An `Arc<AtomicBool>` checked on every entry, so a cancel stops within one
  directory read.
- **Memory.** One small `FileRecord` per file and one `DirectoryRecord` per directory. No extended
  attributes, no ACLs, no alternate data streams are retained.

### Aggregation

`src-tauri/src/scanner/aggregate.rs`

`ScanIndex::from_run` rolls the flat lists up:

1. files roll into their parent directory,
2. directories roll up from the deepest to the shallowest, so a single pass suffices,
3. an adjacency list is built once, each child list pre-sorted by size descending.

That last point matters: tree construction is then linear in the number of directories rather than
quadratic, which is the difference between a usable scan and an unusable one at a million entries.

### Tree pruning — the "don't ship millions of nodes" contract

The scan index stays in Rust. The UI only ever receives a *pruned* tree:

- `maxChildrenPerNode` (default 100) children per folder. The remainder is collapsed into one
  synthetic `Other (n items)` node whose bytes, file count and folder count are exactly the sum of
  what was dropped, so the parent's total stays arithmetically exact.
- A global node budget (default 150 000) enforced by reducing the depth limit until the tree fits.
- `ScanResult.prunedNodes` reports how many nodes were dropped.

The frontend half of this contract lives in `src/lib/tree.ts` and the Rust half in
`scanner/aggregate.rs::prune_tree`. They are deliberately identical; `src/lib/tree.test.ts` pins the
behaviour on the TypeScript side and `scanner/aggregate.rs`'s tests pin it on the Rust side. If you
change one, change both.

Because the tree is pruned, the file list does not page through it. `scan_children(scanId, path,
offset, limit)` serves real children from the index, and the synthetic `Other` node is filtered out
— it is an aggregate, not a filesystem entry, and every command refuses to act on it.

### Progress

`scanner/progress.rs` holds counters in atomics. The emitter thread reads a snapshot roughly every
100 ms and publishes `scan://progress`. ETA is only emitted once more than 5% of the expected total
has been analyzed, and `percent` is `null` when there is no honest expectation (for example when
scanning a single folder rather than a whole volume).

---

## 2. Visualization

`src/features/visualization/`

- **Sunburst** (`Sunburst.tsx`) — `d3.hierarchy().sum()` → `d3.partition()` → `d3.arc()`. Arcs are
  memoized so a selection change does not re-render the whole chart. Angle-based pruning and a
  `maxVisibleArcs` cap keep the DOM node count bounded on huge trees. Click selects, double-click
  navigates in, the centre button returns to root, and the chart is keyboard navigable with
  accessible labels.
- **Treemap** (`Treemap.tsx`) — `d3.treemap()` with squarify, depth-clamped and a `minArea` floor so
  sub-pixel blocks are not drawn.
- **File list** (`FileList.tsx`) — a custom fixed-row-height `VirtualList`, so a folder with 200 000
  entries renders the same as one with 20. Sort, group and search all operate on the page the
  backend served.
- **Layout math** (`layout.ts`) — pure functions, unit-tested: sunburst arc paths, treemap tiles,
  category breakdown, node counting and sorting. This is where the "accurate relative sizes" and
  "graceful zero-byte handling" requirements are actually enforced.

---

## 3. Duplicates

`src-tauri/src/duplicates/`

Three stages, in order of cost:

1. **Group by exact size.** No I/O. On a real volume this removes the overwhelming majority of
   candidates.
2. **Hash the survivors.** SHA-256 by default, streamed with a reused 1 MiB buffer so a 40 GB file
   costs 1 MiB of RAM. Hashing runs on the shared rayon pool.
3. **Verify byte-for-byte** when the user asks, comparing in lockstep and reporting the first
   differing offset.

Filenames are never used as evidence. Groups are ordered by reclaimable space. Nothing is
preselected: exactly one copy per group is flagged as the *suggested* original (oldest, then
shallowest path), and the UI disables the checkbox that would select every copy. The backend
recomputes `reclaimableBytes` from the copies that remain, so the invariant holds even if the
frontend is wrong.

---

## 4. Cleanup

`src-tauri/src/cleanup/`

Rules → findings → preview → execute.

- **Findings** (`rules.rs`) explain themselves: title, reason, exact paths, file and folder counts,
  byte totals, oldest and newest modification timestamps. Protected categories (system, applications,
  documents, games, development) can only ever be `informational` — the checkbox is disabled and no
  action is offered.
- **Preview** (`execute.rs::preview_plan`) simulates every path and reports what would happen,
  changing nothing.
- **Execute** (`execute.rs::execute_plan`) revalidates each path immediately before acting:
  - is it on the never-touch list? refuse,
  - is it inside an approved root (a folder the user configured)? if not, refuse,
  - is it a link? refuse — never delete through a link,
  - is it a synthetic `Other` node? refuse,
  - does it still exist? if not, report `missing`,
  - has it changed size since the scan? if so, report `changed` and skip it,
  - otherwise act: Recycle Bin by default (`trash::delete`), permanent deletion only when the caller
    explicitly asks for it.
- Failures and partial completion are reported path by path. Nothing is swallowed.

---

## 5. Storage

`src-tauri/src/storage/db.rs`

One SQLite database in the app's own data directory, WAL mode, `PRAGMA user_version` migrations.
Eleven tables: `settings`, `scan_history`, `entitlement_cache`, `entitlement_events`,
`cleanup_rules`, `license_key`, `device_activations`, `license_records`, `webhook_events`,
`refunds`-shaped audit, and `license_audit`.

No file contents. No full-path analytics. Delete the database and Diskwise starts from a clean
slate; scanning works identically.

Migrations are versioned with `PRAGMA user_version` and run in order, so an upgrade from schema v1 to
v2 adds the licensing tables without losing anything. `storage/db.rs` tests the round-trips,
including that a revoked licence record is *kept* rather than deleted, and that a deactivated device
keeps its row for the audit trail.

---

## 6. Entitlement

`src-tauri/src/licensing/` and `src-tauri/src/authz/`

A licence key is `<base64url payload>.<base64url signature>`. The payload is versioned JSON
carrying the schema version, the signing key id, the licence id, the product and tier, the issue
time, a `permanent` flag, an optional offline-lease expiry, the device binding and the device limit.
The signature is Ed25519 over the payload bytes, verified against a **public** key compiled into the
binary. The matching private key lives in the licence service and never enters this repository, the
frontend bundle or the packaged app.

- Verification is offline. No network call is required to scan, and none is made.
- Any failure — malformed, bad signature, unknown key, wrong product, wrong device, lapsed lease —
  resolves to the free entitlement with a customer-facing reason. Scanning is never blocked.
- **`permanent` and `leaseUntil` are different fields.** A one-time purchase is permanent and never
  expires. The lease is only how long a previously verified licence keeps working without
  re-validation, which is what lets a refunded licence be withdrawn offline. A lapsed lease on a
  permanent licence says "re-verify when you can", never "you no longer own this".
- The frontend reads the entitlement; it never decides it. There is no pricing logic in `src/`.

### The authorization layer

`src-tauri/src/authz/` is the single place a paid capability is granted, and it is deliberately
separate from the licensing module so the two questions never get conflated:

- `licensing` answers *"is this licence valid, and what does it entitle the holder to?"*
- `authz` answers *"may this operation proceed?"*

`Permission` is an enum of fifteen capabilities, split five free / ten paid. `PERMISSIONS` is the
complete list and the tests iterate it, so a new capability cannot be added without deciding
explicitly which side it belongs on.

`authorize()` returns `Ok(())` or a customer-facing message, and fails closed for every failure mode
including "the authorization has not been resolved yet". The commands that call it:

| Command | Permissions checked, in order |
| --- | --- |
| `cleanup_execute` | `PaidCleanup` → `PermanentDeletion` (any item bypassing the Recycle Bin) → `BulkFileOperations` (past 50 paths) |
| `cleanup_preview` | `PreviewOnly` |
| `cleanup_scan` | `CoreAnalysis` |
| `find_duplicates` | `BasicDuplicates`, and the free group limit is applied inside the worker |
| `duplicates_get` | `BasicDuplicates` |
| `file_details` | `FileInspection` |
| `list_large_files` | `LargeFileFinder` |
| `scan_start`, `scan_get` | `CoreAnalysis` |

Duplicate *removal* is not a separate command: it goes through `cleanup_execute` with a plan, so it
is gated by `PaidCleanup` exactly like any other deletion. That is intentional — one destructive
path, one set of safety checks, one authorization gate.

**A modified React interface cannot bypass any of this.** The webview contributes nothing to the
proof: the command reads the entitlement the backend resolved from a signed payload, so hiding a
paywall in the UI, or invoking the command directly from the devtools console, changes nothing.
`src-tauri/tests/licence_lifecycle.rs::a_modified_frontend_cannot_bypass_authorization` asserts this
by walking the whole catalogue on the free tier.

### The commercial configuration

`src-tauri/src/config/product.rs` holds `DEFAULT_CONFIG`: the product id, the tiers with their
prices and currencies, the device limit, the offline lease length, the free duplicate group limit and
the trusted signing keys with their key ids. It is the **only** place these numbers exist, it is
validated on load so a typo cannot silently grant access, and it defaults to the test environment so
a build off the default branch cannot mint a real entitlement. `docs/licensing.md` §11 lists every
value and where it lives.

---

## 7. Platform layer

`src-tauri/src/platform/`

The Win32 entry points Diskwise uses are declared by hand rather than pulled from a binding crate,
so the exact set of OS calls is explicit and reviewable:

| Function | Used for |
| --- | --- |
| `GetLogicalDrives`, `GetDriveTypeW` | volume enumeration |
| `GetDiskFreeSpaceExW` | capacity and free space |
| `GetVolumeInformationW` | volume label and filesystem name |
| `GetCompressedFileSizeW` | on-disk (allocated) size, including sparse and compressed files |
| `SHQueryRecycleBinW` | Recycle Bin contents |
| `ShellExecuteW` | "open with the default application" |

On non-Windows targets a portable fallback provides the same functions with honest "not available"
answers, which is what lets the Rust unit tests run on Linux CI runners.

"Reveal in File Explorer" uses `explorer.exe /select,<path>` with a validated argument. It is a Rust
command, not a permission granted to the webview.

---

## 8. Capability allowlist

`src-tauri/capabilities/default.json` grants the webview exactly:

```
core:default
core:window:allow-minimize, allow-toggle-maximize, allow-close, allow-start-dragging,
             allow-is-maximized, allow-set-focus
core:event:default
dialog:allow-open, allow-message, allow-ask, allow-confirm
opener:allow-open-path, allow-open-url
```

No `shell`, no `fs`, no `process`, no `allow-execute`. Every filesystem action is a narrowly-scoped
Rust command instead, and `src-tauri/src/security.rs` asserts the capability file stays that way.

The clipboard uses the web Clipboard API on the secure `http://tauri.localhost` origin.

---

## 9. Known limits

- The Rust code has not been compiled in the environment this repository was authored in; there is
  no Rust toolchain obtainable there. CI on `windows-latest` is the compile and test gate. See
  [`testing.md`](testing.md).
- The Windows Recycle Bin query reports the whole machine's bin rather than per-drive.
- `blake3` is offered in the TypeScript contract but the Rust backend currently implements SHA-256
  only; the option is accepted and mapped to SHA-256 rather than silently failing.
- Scheduled scans and exportable reports are Pro features with a defined entitlement boundary but no
  implementation behind them yet.
