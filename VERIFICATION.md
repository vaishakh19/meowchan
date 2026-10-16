# Verification — 16 October 2026

**Windows build, executable launch and installer tests passed.** This is a
read-only analyzer, not a destructive cleanup tool or a production-certification
claim.

## Delivered build

- Source compiled: `c9a0a26a038c5857cb62bd8e6d9276e42af98274`.
- Successful [Windows run 38898644274](https://github.com/vaishakh19/meowchan/actions/runs/38898644274).
- Host: GitHub Actions Windows Server 2022 x64, with WebView2 already installed.
- Node 22.22.3; Rust MSVC 1.99.1; Visual Studio 2022 C++ Build Tools/Windows SDK.
- `release/Diskora.exe`: 5,078,528 bytes.
- `release/Diskora-setup.exe`: 1,899,563 bytes (current-user NSIS installer).
- `src-tauri/Cargo.lock` records the dependencies used for this build;
  the build command uses `--locked`. File hashes are in `release/SHA256SUMS.txt`.

## Actually passed

- Clean npm installation; production dependency audit: zero vulnerabilities.
- TypeScript typecheck, 117 frontend/unit tests and Vite production build.
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

Launch testing caught an incorrect benchmark-binary selection and a startup
race; the unused benchmark target was removed, the desktop target is explicit,
and scan controls now wait for native initialization. Windows tests also exposed
and verified fixes for icon resources, extended-path normalization, aggregate
rollups and pruning budgets. No failing tests were hidden or deleted to pass CI.

## Limits and unverified environments

- No clean Windows 10/11 VM test without developer tools or WebView2. The renamed
  copy and PE import checks are useful evidence, not a substitute for that test.
- The installer's missing-WebView2 bootstrap/download path was configured but
  not exercised: CI already had the runtime. A missing runtime requires internet
  access, or prior offline installation of Microsoft's Evergreen WebView2 runtime.
- Binaries are unsigned; no SmartScreen reputation, signing or auto-update work.
- Native folder-dialog interaction, Explorer integration, accessibility, display
  scaling and very large real-volume performance were not manually verified.
- Read-only analysis only: no file deletion/moving, billing, scheduling or export.
  Links are not followed. Display trees are pruned; the file list uses the index.
  Scans default to a one-million-entry cap and report truncation. Duplicate
  search defaults to a 1 MiB minimum (adjustable down to 1 KiB) and skips files
  over 5 GiB. Historical summaries do not retain navigable file indexes.

Source was imported from `vaishakh19/diskora` at `c8a80dd` into the Arena-attached
`vaishakh19/meowchan` repository. The original Diskora repository was not modified.
All work and delivered binaries are on `arena/d1f1769d-meowchan`.
