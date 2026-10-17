# Diskora

A local Windows disk space analyzer. Scan a drive or folder, explore its
sunburst/treemap/file list, find large files, identify SHA-256 content duplicates,
and review local scan summaries. No account, paid service, API key, telemetry,
backend server, or administrator access is required.

**Free, and always free:** scanning, filtering, size calculations, large-file
listing and duplicate detection. **Diskora Pro** (one-time $12.99, no
subscription) adds safe file deletion through the Windows Recycle Bin and full
absolute path display with copy. See the [Pro page](PRO-SETUP.md) for merchant
setup and purchase testing.

## Current build status

The current source passed the Windows CI workflow on a Windows Server 2022 runner
(run [39031447966](https://github.com/vaishakh19/meowchan/actions/runs/39031447966)).
CI rebuilt the raw executable and current-user NSIS installer, smoke-tested the real
WebView2 app with real files, and installed/uninstalled the package. The generated
binaries are now in `release/`; their SHA-256 manifest is checked locally and the
CI workflow validates its two-line format on subsequent builds. This does not
replace a manual Windows 10/11 Recycle Bin or Dodo purchase test. See
[VERIFICATION.md](VERIFICATION.md) for exact checks and remaining limits.

## Build and run on Windows x64

Install only:

1. **Node.js 22.12 or newer** (22.22.3 is used in CI), including npm.
2. **Rust via rustup**, using the MSVC host (`x86_64-pc-windows-msvc`). The repository
   pins **Rust 1.99.1** in `rust-toolchain.toml`; rustup installs that version.
3. **Visual Studio 2022 Build Tools**, with **Desktop development with C++**:
   MSVC v143 x64/x86 tools and Windows 10/11 SDK (10.0.19041 or newer).
   The full Visual Studio IDE and unrelated workloads are not needed.

Download the source ZIP and extract it, or clone this branch. In PowerShell at the
repository root:

```powershell
npm ci
npm run app:build
```

The single build command is **`npm run app:build`** after dependency installation.
It typechecks/builds React, compiles Rust for Windows x64, and creates one NSIS
installer. Internet access is needed for initial dependency/tool downloads.

For development on Windows, **`npm run dev`** starts Tauri and Vite with hot reload.
The plain frontend server is available as `npm run frontend:dev`; it is not a disk
scanner and deliberately shows **“Run inside the desktop app”** in a normal browser.
`npm run smoke` exercises the packaged Windows executable and is Windows-only.

Output:

```text
release/Diskora-setup.exe   Recommended distribution: current-user installer
release/Diskora.exe         Raw application executable
release/SHA256SUMS.txt      SHA-256 checksums for the two binaries
```

No Node.js, Rust, SDK, source directory, or dev server is needed to run the result.
The C runtime is linked statically. WebView2 is an **OS runtime dependency**, not
embedded into the raw executable. The installer downloads/installs WebView2 if it
is missing; internet access is needed in that case. On an offline PC, install
Microsoft's Evergreen WebView2 Runtime beforehand, then use the raw executable.
A fully standalone executable without WebView2 would require changing frameworks
or distributing a much larger fixed WebView2 runtime; neither is needed here.

You may copy and rename **the raw executable**. Renaming the installer does not
rename the installed application. Product name, installer name and binary output
are configured separately in `src-tauri/tauri.conf.json`. The legacy identifier
`com.diskwise.app` and database filename `diskwise.sqlite3` are intentionally kept
so existing preferences and scan summaries remain accessible. No old tables are
dropped.

## Code signing and SmartScreen

The distributed executable and installer are unsigned unless a release operator
adds optional Windows code signing. Windows may show SmartScreen or publisher
warnings for unsigned or newly signed builds; this is expected and is not a
substitute for verifying the download. Check the SHA-256 values in
`release/SHA256SUMS.txt` against the binaries. A production signing certificate
and its private key must be stored only in the release system's protected secret
store, never committed to this repository. Signing is not configured in this
source branch.

## Use

Launch Diskora, choose **Scan Drive** or **Scan a folder…**, then explore the
results. Duplicate search requires a completed scan: it groups by exact size,
checks a bounded prefix/suffix SHA-256 sample, then streams a full SHA-256 only
for sample collisions. The UI defaults to a 1 MiB minimum (adjustable down to
1 KiB); files over 5 GiB are not hashed and are counted in the report. Duplicate
scans can be cancelled. They never remove copies. Scanning is also cancellable.
Inaccessible or vanished entries produce a partial result with typed errors, not
invented sizes. Logical sizes can differ from volume usage because of permissions,
hard links, filesystem metadata, compression, sparse files and concurrent changes.
Opening the frontend in a normal browser displays **“Run inside the desktop app”**;
there is no synthetic filesystem or demo scan data.

**Deleting files (Pro):** select items and choose **Delete**. Diskora confirms
the exact name, path, size and type of every item first, then requests a move to
the Windows Recycle Bin through the shell. It never calls a permanent-delete API
or falls back to permanent removal; if a path fails a safety check or Windows
refuses the Recycle Bin operation, that item is reported as failed. Links,
junctions, drive roots, the scanned folder itself and protected system locations
are refused. The scan index refreshes after successful moves.

**Paths (Pro):** the details panel shows the full absolute path of the selected
item, with **Copy path**. Without Pro, paths are shown masked in the middle.

**Diskora Pro** is verified against Dodo Payments on activation, restore and
before deletion. Deletion requires a fresh successful validation; offline grace
never authorizes a destructive action. The entitlement is encrypted with
user-scoped Windows DPAPI before it is stored in SQLite. Network use is limited
to the allowlisted Dodo licensing endpoints; everything else stays on your machine.

Scan trees are pruned for display; the file list pages through the underlying
index. A scan defaults to a one-million-entry safety limit and reports truncation
when reached. New scans persist only summary metadata and aggregate totals; the
per-file index and full trees are not written. A restart requires a rescan to
navigate files. History retains up to the most recent 50 summaries. For safety,
legacy `scan_snapshots` rows from older versions are left untouched but the
current app no longer reads or writes that table.

## Verification

```powershell
npm run test:all
npm run rust:test -- --target x86_64-pc-windows-msvc
```

`.github/workflows/ci.yml` runs native filesystem tests on Windows, builds the
installer, and runs `node scripts/smoke-windows.mjs` against the actual renamed
release executable in a separate directory. That smoke test uses real on-disk
files, real IPC/scanning/hashing and the rendered WebView2 UI. Only the OS folder
picker's answer is replaced during the test. It is never replaced in the shipped
application. Browser opening is deliberately rejected, not filled with demo data.

**Current validation status:** see `VERIFICATION.md` for the exact checks actually
completed on this branch. The Windows workflow is configured to run native tests,
build the raw executable and current-user NSIS installer, inspect runtime imports,
and smoke-test/install/uninstall the app; configuration is not evidence those steps
have passed. No Windows 10/11 desktop run or real Recycle Bin operation has been
verified for this branch. A clean PC, WebView2 installation, accessibility/DPI
checks, and signing/SmartScreen reputation remain release-validation tasks.
