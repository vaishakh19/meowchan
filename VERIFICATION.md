# Verification — 16 October 2026

**The previously delivered Windows build is verified and unchanged.**
The new Pro/deletion work on this branch is **frontend-verified only**:
the Rust backend was written and line-reviewed but could **not** be
compiled in this environment (no Rust toolchain; see below). Do not
ship the new code until `npm run app:build` and `cargo test` pass on
Windows.

## What this environment could run (all passed)

- `npm ci` — clean install, zero vulnerabilities.
- `npm run typecheck` — TypeScript strict mode, clean.
- `npm test` — **128 vitest tests** (11 files), including new suites:
  - `DeleteConfirmDialog.test.tsx` — locked-Pro refusal, confirmation
    content (name/path/size), cancel deletes nothing, confirm invokes
    `delete_items` then refreshes the scan, backend failure surfaces
    honestly.
  - `ProView.test.tsx` — pricing screen ($12.99, pay once), locked /
    purchased / unconfigured states, activation IPC, restore, and
    failed activation unlocking nothing.
  - `App.test.tsx` — updated: asserts the upgrade path exists while
    Pro stays locked (the old "no billing controls" assertion encoded
    the pre-Pro scope).
- `npm run build` — Vite production build succeeds.

## What this environment could NOT run

- **`cargo build` / `cargo test`** — no Rust toolchain exists here and
  rustup cannot download one (network allowlist). The Rust changes are
  review-verified only. A compile blocker found in review
  (`delete_items` holding `Arc<ScanIndex>` while removal needs
  `&mut self`) was fixed by converting `ScanEntry.index` to
  `Arc<RwLock<ScanIndex>>` across `state.rs`, `scan.rs`, `files.rs`,
  `duplicates.rs`, `large_files.rs` and `delete.rs`; readers take read
  guards, deletion takes the write guard. Further review fixes: the
  Pro gate in `delete_items` now uses `require_pro` (a `Locked`
  decision must never reach the shell call), protected-location checks
  run before the scan-boundary check so escape attempts into
  `C:\Windows` are reported as protected, and subtree size is captured
  before the index overlay marks the path removed.
- **`npm run app:build`** — needs Windows + the MSVC toolchain. The
  `release/` binaries (Diskora.exe 5,078,528 B; Diskora-setup.exe
  1,899,563 B) are the **previous verified build** and are preserved
  untouched as the current deliverable.

## Previously verified (the delivered release/ binaries, commit c9a0a26a)

- Successful [Windows run 38898644274](https://github.com/vaishakh19/meowchan/actions/runs/38898644274).
- Host: GitHub Actions Windows Server 2022 x64, WebView2 installed.
- Node 22.22.3; Rust MSVC 1.99.1; Visual Studio 2022 C++ Build Tools/Windows SDK.
- `release/Diskora.exe`: 5,078,528 bytes; `release/Diskora-setup.exe`:
  1,899,563 bytes (current-user NSIS installer).
- Hashes in `release/SHA256SUMS.txt`; build used `--locked`.

### Actually passed on Windows (previous build)

- Clean npm installation; production dependency audit: zero vulnerabilities.
- TypeScript typecheck, frontend unit tests and Vite production build.
- Windows native unit and filesystem integration tests: real Unicode paths,
  exact byte/file/folder totals, duplicate contents, missing roots, pruning
  budgets, settings preservation and exclusion path boundaries.
- Release compilation and NSIS packaging using `npm run app:build`.
- `dumpbin /dependents` check: no external VCRUNTIME, MSVCP, sqlite3 or
  WebView2Loader DLL import. WebView2 itself remains a required OS runtime.
- Actual release executable copied alone into a separate temporary directory,
  renamed `Renamed.exe`, and launched there.
- Real rendered WebView2 UI scanned a disk fixture containing four files and
  a Unicode-named subfolder: **3,145,739 bytes, 4 files, 1 folder**.
- Large Files UI filtered at 1 MiB and showed the three expected files.
- Duplicate Finder UI found the two content-identical files, not the same-sized
  different-content file; no uncaught JavaScript exceptions were observed.
- Silent installer installation into a path containing spaces; the installed
  executable passed the same real UI tests; silent uninstall removed it.

The UI test replaces **only the OS folder picker's answer** with the fixture
path. The scanner, native IPC, progress, filesystem reads, SHA-256 hashing and
rendered results are real. The shipped app contains no test interception.
Frontend IPC mocks/layout fixtures are unit tests, not filesystem verification.

## New in this branch (unverified on Windows so far)

- **Safe deletion (Pro):** Rust `delete_items` command — Pro gate
  re-verified server-side in the backend, lexical guards (drive root,
  scan root, outside-scan, UNC, protected system locations, `..`
  normalisation), reparse-point refusal (symlinks/junctions never
  followed), NTFS/ReFS-only (Recycle Bin requirement), COM
  `IFileOperation` delete with `FOF_ALLOWUNDO`, post-operation
  existence proof, per-item success/failure reporting, and an
  in-memory removal overlay so scan results refresh without a rescan.
- **Full paths (Pro):** details panel and menus show the complete
  absolute path behind Pro; masked mid-ellipsis otherwise.
- **Licensing:** Dodo Payments public activate/validate endpoints from
  Rust (no backend, no merchant secret shipped), 7-day offline grace,
  SQLite-persisted entitlement, restore-purchases flow, product-id
  binding check. See `PRO-SETUP.md` for the merchant steps.
- **UI:** Pro badge, Upgrade to Pro button, Pro page with $12.99
  one-time pricing, success/locked states, deletion confirmation
  dialog, honest privacy disclosures.

## Limits and unverified environments

- **The Rust code has never been compiled.** First `cargo test` and
  `npm run app:build` on Windows may surface compile errors; the
  review fixes above reduced, but did not eliminate, that risk.
- Deletion, licence activation and restore have **not** been exercised
  against a real Dodo Payments test purchase or a real Recycle Bin.
- No clean Windows 10/11 VM test without developer tools or WebView2.
- Binaries are unsigned; no SmartScreen reputation, signing or auto-update work.
- Native folder-dialog interaction, Explorer integration, accessibility, display
  scaling and very large real-volume performance were not manually verified.
- Read-only analysis remains the default experience; only the two Pro
  capabilities mutate (deletion) or use the network (licence checks).
  Links are not followed. Display trees are pruned; the file list uses the index.
  Scans default to a one-million-entry cap and report truncation. Duplicate
  search defaults to a 1 MiB minimum (adjustable down to 1 KiB) and skips files
  over 5 GiB. Historical summaries do not retain navigable file indexes.

Source was imported from `vaishakh19/diskora` at `c8a80dd` into the Arena-attached
`vaishakh19/meowchan` repository. The original Diskora repository was not modified.
All work and delivered binaries are on `arena/d1f1769d-meowchan`.
