# Privacy

Diskwise is local-first. There is no account, no mandatory cloud, no automatic uploads and no
background scanning. This page states exactly what is stored and what leaves the device; the running
app shows the same list, read from the backend rather than from marketing copy
(`privacy_report` command, rendered in the Privacy screen).

## Stored locally

One SQLite database at `%APPDATA%\com.diskwise.app\diskwise.sqlite3`:

| Table | Contents |
| --- | --- |
| `settings` | Theme, default scan root, children-per-folder, link following, cleanup confirmation preference, Recycle Bin preference, age threshold, cache and temp folders, never-touch paths, crash-reporting opt-in, duplicate group limit. |
| `scan_history` | One row per scan: root path, root name, start and finish timestamps, duration, status, total bytes, file count, folder count, error count, whether it was truncated, drive id. **No file names, no individual paths.** |
| `entitlement_cache` | The current licence and its entitlement state, plus when it was last verified. |
| `entitlement_events` | A short log of entitlement transitions with a reason. |
| `cleanup_rules` | The cleanup rules the user configured. |
| `license_key` | The licence key you activated, and the device binding it was activated for. It is stored as-is because it is the bearer token the offline verifier needs. It is a credential: anyone with it can use the licence on another machine until the device limit catches them. |
| `device_activations` | A local mirror of the licence service's device ledger, so the Devices screen works offline. Device name and binding only. |
| `license_records` | The licence record the service last reported: product, tier, permanence, lease, device limit, and a revocation reason if it was revoked. |
| `license_audit` | Activation, deactivation and revocation events. **No file paths and no file names.** |
| `webhook_events`, `refunds`, `revocations` | Declared for schema completeness. The service owns them; the app never writes them. |

### What is never stored

- **A hash of the licence key, on the server.** The Supabase `licences` table stores a SHA-256 of the
  key, not the key. A database leak does not hand out working licences. The desktop app is the one
  place the key exists in plaintext, because it has to present it verbatim when re-verifying.
- **Any scanned file name, path or content.** Not in the database, not in a log, not in a crash
  report.
- **Any part of your directory tree.** Scan results are in memory only and the tree sent to the
  webview is pruned.
- **A private signing key, a payment API key, a webhook secret or a Supabase service-role key.** None
  of them exist in the app, the bundle, the installer or this repository. CI greps for their shapes.

Scan results themselves live in memory only. They are never written to disk, and the tree sent to
the webview is pruned — see [`architecture.md`](architecture.md#tree-pruning--the-dont-ship-millions-of-nodes-contract).

Deleting the database is safe. Diskwise starts from a clean slate and scanning behaves identically.

## Leaves the device

**Nothing, by default.** The app makes no network request during a scan, a duplicate search or a
cleanup.

Two exceptions, both opt-in and both off unless you turn them on:

1. **Crash reporting** (Settings → Privacy). When enabled, a crash sends a stack trace and the app
   version. No file paths, no file names, no scan results, no machine identifier beyond what the
   transport itself carries.
2. **Licence activation and recovery.** These are the only paths that make a network request, and
   only when you explicitly activate, deactivate or recover a licence. They send:
   - the licence key you typed (hashed for lookup, and never stored in plaintext by the service),
   - a device binding — a hash of your hostname, OS install date and data-directory volume,
   - a device name, which defaults to your computer name and which you can see in the app.

   That is the complete list. No file names, no paths, no scan results, no directory contents, no
   email address beyond the one you typed into recovery.

Entitlement verification itself is **offline and stays offline**. A licence you have already
activated keeps working with no network at all for the length of its offline lease, and scanning,
previews and the large-file finder never require a connection in any state.

There is no analytics on full paths. There is no telemetry about which folders you scan. There are
no ads and no sponsored results, and no code path that could produce either.

### The machine identifier is not a secret

`DeviceBinding::current()` derives `dev-` + a hash of `hostname | OS install date | data-directory
volume`. It is a **binding, not a credential**: knowing it does not let anyone use your licence,
because the licence key is still required and the key is what the signature covers. It exists so a
reinstall on the same machine does not consume a second device slot.

## What Diskwise will not do

- Scan in the background. A scan runs while you are looking at it and stops when you cancel it.
- Execute a file it has analysed.
- Elevate privileges. Scanning a folder you can already read needs no administrator rights, and the
  app never asks for them.
- Delete a file because it was selected or analysed.
- Classify your documents, browser profiles, development dependencies or package caches as junk.
- Offer Windows system files or application data for deletion.

## Permissions the webview has

`src-tauri/capabilities/default.json` grants the webview:

```
core:default
core:window:allow-minimize, allow-toggle-maximize, allow-close, allow-start-dragging,
             allow-is-maximized, allow-set-focus
core:event:default
dialog:allow-open, allow-message, allow-ask, allow-confirm
opener:allow-open-path, allow-open-url
```

That is the complete list. There is no `shell` permission, no filesystem scope and no ability to
spawn a process. Every filesystem action — reading metadata, revealing in Explorer, opening a file,
moving something to the Recycle Bin — is a narrowly-scoped Rust command that validates its input
first. `src-tauri/src/security.rs` contains a test that fails if a dangerous permission is ever added
to the capability file.

## Verifying this yourself

The claims above are testable rather than aspirational:

- `src-tauri/src/security.rs::the_webview_has_no_shell_permission` asserts the capability file's
  contents.
- `src-tauri/src/security.rs::no_command_in_the_backend_spawns_a_scanned_file` asserts the only
  process the backend ever starts is `explorer.exe` with a fixed argument shape.
- The Privacy screen in the running app reads the real database and data-directory paths from the
  backend, so the list cannot drift from reality.
- `.github/workflows/ci.yml` greps the built `dist/` bundle and the tracked code for the shapes a
  private key, a provider API key or a webhook secret would take, and fails the build if it finds
  one.
- The `licence` CI job asserts the trusted public key shipped in
  `src-tauri/src/config/product.rs` is still the documented all-zero placeholder, so a build straight
  off the default branch cannot mint a real entitlement.
- The `audit` CI job scans the tracked code for credential-shaped strings.
