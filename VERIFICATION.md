# Verification — 17 October 2026

## Windows CI result

The Windows workflow passed on a `windows-2022` runner for source commit `a5ca309`; the CI job then delivered its generated binaries to the session branch as commit `4cb8e81`. See [run 39031447966](https://github.com/vaishakh19/meowchan/actions/runs/39031447966).

Passed steps included:

- Native Rust tests targeting `x86_64-pc-windows-msvc`.
- `npm ci`, `npm run typecheck`, `npm run lint` and `npm test` (**14 files, 129 tests**).
- `npm run app:build`, which produced `release/Diskora.exe` and the current-user NSIS installer.
- PE runtime-import inspection.
- Smoke test of the actual WebView2 application with real on-disk files.
- Installer install/run/uninstall checks on the CI runner.

The generated files in `release/` are **7,279,104 bytes** (`Diskora.exe`) and **2,761,707 bytes** (`Diskora-setup.exe`). `sha256sum -c release/SHA256SUMS.txt` passes locally for the current files. The initial CI checksum writer joined both entries onto one line; that was corrected in the follow-up workflow change, which also validates the manifest has exactly one correct line per binary. That follow-up check is pending its next CI run.

The first CI attempt caught an ownership error in the new duplicate-report test; it was fixed in `a5ca309`. The passing run above is the subsequent run.

## Checks completed in the authoring sandbox

- `npm ci` — installed 316 packages; audit reported 0 vulnerabilities. npm emitted one deprecation notice for `whatwg-encoding`.
- `npm run typecheck` — passed.
- `npm test` — passed: **14 files, 129 tests**. UI tests that mock IPC are not real filesystem or native integration tests.
- `npm run lint` — passed with **0 errors and 0 warnings**.
- `npm run build` — passed; Vite produced the production frontend. Vite prints a non-fatal `INEFFECTIVE_DYNAMIC_IMPORT` warning because the transport module is already statically imported elsewhere.
- `git diff --check` — passed.

## Still not verified

- The CI host is Windows Server 2022, not a clean Windows 10/11 desktop. No manual Win10/11 compatibility, accessibility, DPI/scaling, offline WebView2 or SmartScreen/signing validation was performed.
- No real/test Dodo Payments purchase, activation, restore or online/offline validation was exercised.
- The CI smoke test does not perform a real Pro deletion. Recycle Bin placement/recovery and the deletion confirmation/safety path still need a manual Windows test.
- The authoring sandbox does not have `cargo`, `rustc` or `rustfmt`; native verification came from the Windows CI run, not a local Rust build.

Before release, manually test on Windows 10/11: scan real folders/drives, exercise cancellation and partial/error cases, detect and byte-verify duplicates, confirm deletion guards and Recycle Bin recovery, and test a Dodo test purchase/restore. Review the generated checksums and installer, and decide whether to sign the binaries. See `README.md` and `PRO-SETUP.md` for build and merchant instructions.
