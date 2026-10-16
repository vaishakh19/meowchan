// One packaging path: a current-user NSIS installer, plus the raw executable.
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';

if (process.platform !== 'win32' || process.arch !== 'x64') {
  console.error('Build Diskora on 64-bit Windows with Node.js, Rust MSVC and the C++ Build Tools. See README.md.');
  process.exit(1);
}
const cli = resolve('node_modules/@tauri-apps/cli/tauri.js');
const result = spawnSync(process.execPath, [cli, 'build', '--ci', '--target', 'x86_64-pc-windows-msvc'], { stdio: 'inherit' });
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
const release = resolve('src-tauri/target/x86_64-pc-windows-msvc/release');
const installers = readdirSync(`${release}/bundle/nsis`).filter(name => name.endsWith('-setup.exe'));
if (installers.length !== 1) throw new Error('Expected exactly one NSIS installer. Clean src-tauri/target and rebuild.');
mkdirSync('release', { recursive: true });
copyFileSync(`${release}/Diskora.exe`, 'release/Diskora.exe');
copyFileSync(`${release}/bundle/nsis/${installers[0]}`, 'release/Diskora-setup.exe');
console.log('Built release/Diskora.exe and release/Diskora-setup.exe. The raw executable requires WebView2; use the installer on a fresh PC.');
