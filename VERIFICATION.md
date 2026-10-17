# Verification — 17 October 2026

## Status

The frontend checks and production web build listed below passed in the authoring sandbox. **Rust compilation/tests and every Windows-specific check remain unverified for this branch.** The `release/` binaries were inherited from the base revision and are not evidence that the current source builds; do not distribute them as a build of these changes.

## Checks completed locally

- `npm ci` — installed 316 packages; audit reported 0 vulnerabilities. npm emitted one deprecation notice for `whatwg-encoding`.
- `npm run typecheck` — passed.
- `npm test` — passed: **14 files, 129 tests**. Coverage includes UI transport wiring, the browser-only desktop-runtime notice, duplicate byte-verification action, list paging/sorting and tree-pruning logic. Tests that mock IPC are not real filesystem or native integration tests.
- `npm run lint` — passed with **0 errors and 0 warnings**.
- `npm run build` — passed; Vite produced the production frontend. Vite prints a non-fatal `INEFFECTIVE_DYNAMIC_IMPORT` warning because the transport module is already statically imported elsewhere.
- `git diff --check` — passed.

## Not run / not verified

- **Rust:** `cargo`, `rustc` and `rustfmt` are not installed in this environment. No `cargo test`, Rust compile, formatting check or Clippy run was possible. All native changes—including scanner path/reparse checks, tree aggregation, duplicate hashing/cancellation/report filtering, SQLite migration, DPAPI and deletion guards—must be treated as uncompiled here.
- **Windows:** `npm run app:build` and `npm run smoke` are Windows-only and were not run. No current-branch raw executable, NSIS installer or refreshed `SHA256SUMS.txt` was produced. WebView2 behavior, current-user install/uninstall, PE imports, real folder scans, junction handling and Recycle Bin behavior have not been verified on Windows 10/11.
- **Dodo Payments:** no real or test purchase, activation, restore or online/offline validation was exercised. The licensing code and merchant setup remain unverified against Dodo endpoints.
- No manual accessibility, DPI/scaling, large-volume performance or SmartScreen/signing validation was performed.

The Windows workflow in `.github/workflows/ci.yml` is configured to run native tests, package the installer/raw executable, inspect runtime imports, smoke-test the actual desktop UI, and test install/uninstall. A configured workflow is not a passed check; consult the branch’s GitHub Actions run before treating any of those steps as verified.

## Release follow-up

Before release, require a successful Windows CI run for this exact source revision, then manually test on a Windows 10/11 machine: scan a real folder and drive, check partial/error/cancellation behavior, detect and byte-verify duplicate files, confirm deletion guards and Recycle Bin recovery, and exercise a Dodo test purchase/restore. Review the generated checksums and installer, and decide whether to sign the binaries. See `README.md` and `PRO-SETUP.md` for build and merchant instructions.
