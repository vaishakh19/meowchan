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

## Run the verified Windows build

The delivered ZIP includes source and the tested Windows x64 build. On Windows
10/11, open **`release/Diskora-setup.exe`** and then launch **Diskora** from the
Start menu. No development tools are needed to run it. Alternatively, launch
`release/Diskora.exe` directly if WebView2 is already installed.

These binaries are **unsigned**; Windows may show an unknown-publisher/SmartScreen
warning. Check the download's origin before choosing to run it. See
`release/SHA256SUMS.txt` for the delivered file hashes.

## Build on Windows x64

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

Output:

```text
release/Diskora-setup.exe   Recommended distribution: current-user installer
release/Diskora.exe         Raw application executable
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

## Use

Launch Diskora, choose **Scan Drive** or **Scan a folder…**, then explore the
results. Duplicate search requires a completed scan and reads file contents for
hashing. The UI defaults to a 1 MiB minimum (adjustable down to 1 KiB); files
over 5 GiB are skipped to bound hashing work. It never removes copies. Scanning is cancellable. Inaccessible or
vanished entries produce a partial result with reported errors, not invented
sizes. Logical sizes can differ from volume usage because of permissions,
hard links, filesystem metadata, compression, sparse files and concurrent changes.

**Deleting files (Pro):** select items and choose **Delete**. Diskora confirms
the exact name, path, size and type of every item first, then moves it to the
Windows Recycle Bin through the shell — always recoverable, never permanent,
never following links or junctions, never touching drive roots, the scanned
folder itself, or protected system locations. A failed item is reported per
item; the scan results refresh immediately.

**Paths (Pro):** the details panel shows the full absolute path of the selected
item, with **Copy path**. Without Pro, paths are shown masked in the middle.

**Diskora Pro** is verified against Dodo Payments on every activation, restore
and deletion — a licence is a server-checked purchase, not a local flag.
Purchases can be restored after a reinstall. Network use is limited to these
licence checks; everything else stays on your machine.

Scan trees are pruned for display; the file list pages through the underlying
index. A scan defaults to a one-million-entry safety limit and reports truncation
when reached. Only summary totals are saved; a restart requires a rescan to
navigate files. History contains the most recent 50 summaries in its UI.

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

**Release status:** see `VERIFICATION.md` for tests actually completed and remaining
limitations. Do not treat configured CI steps as proof they have passed. Installer
installation/uninstallation passed on Windows Server 2022 CI. A clean Windows
10/11 machine without WebView2, interactive accessibility/DPI checks, and
signing/SmartScreen reputation still require separate release validation.
