// Runs the actual packaged WebView2 application. No browser download or test service.
import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, copyFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import assert from 'node:assert/strict';

const directory = mkdtempSync(join(tmpdir(), 'Diskora smoke '));
const fixture = join(directory, 'files');
mkdirSync(join(fixture, 'nested-文件'), { recursive: true });
writeFileSync(join(fixture, 'a.bin'), Buffer.alloc(1024 * 1024, 1));
writeFileSync(join(fixture, 'b.bin'), Buffer.alloc(1024 * 1024, 1));
writeFileSync(join(fixture, 'c.bin'), Buffer.alloc(1024 * 1024, 2));
writeFileSync(join(fixture, 'nested-文件', 'small.txt'), 'hello world');
const executable = process.argv[2] ? resolve(process.argv[2]) : join(directory, 'Renamed.exe');
if (!process.argv[2]) copyFileSync(resolve('release/Diskora.exe'), executable);
const app = spawn(executable, [], { cwd: directory, env: {
  ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=9222',
}, stdio: ['ignore', 'pipe', 'pipe'] });
let output = '';
for (const stream of [app.stdout, app.stderr]) stream.on('data', data => { output = (output + data).slice(-6000); });
let socket;
const pending = new Map();
const errors = [];
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(action, label, timeout = 60000) {
  const deadline = Date.now() + timeout;
  let error;
  while (Date.now() < deadline) {
    try { const value = await action(); if (value) return value; } catch (cause) { error = cause; }
    if (app.exitCode !== null) throw new Error(`Application exited with ${app.exitCode} while waiting for ${label}: ${output}`);
    await pause(200);
  }
  throw new Error(`Timed out: ${label}${error ? ` (${error.message})` : ''}`);
}
let sequence = 0;
function command(method, params = {}) {
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 10000);
    pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value); }, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const result = await command('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
}
const click = text => evaluate(`(() => {
  const button = [...document.querySelectorAll('button')].find(button => button.textContent.trim() === ${JSON.stringify(text)});
  if (!button || button.disabled) return false;
  button.click(); return true;
})()`);
try {
  const page = await until(async () => {
    const pages = await (await fetch('http://127.0.0.1:9222/json')).json();
    return pages.find(page => page.type === 'page' && page.url.includes('tauri.localhost'));
  }, 'WebView2 debug endpoint');
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  socket.onmessage = event => {
    const message = JSON.parse(event.data);
    if (message.method === 'Runtime.exceptionThrown') errors.push(message.params.exceptionDetails);
    const request = pending.get(message.id);
    if (request) {
      pending.delete(message.id);
      if (message.error) request.reject(new Error(JSON.stringify(message.error)));
      else request.resolve(message.result);
    }
  };
  await command('Runtime.enable');
  await until(() => evaluate(`!!document.querySelector('[data-testid="app-shell"]')`), 'rendered desktop UI');
  // Replace ONLY the native file picker answer, not the scanner, filesystem, events or results.
  // This lets the real UI scan a deterministic on-disk fixture without automating OS dialogs.
  await evaluate(`(() => {
    const originalFetch = window.fetch.bind(window);
    window.fetch = (input, options) => {
      const url = new URL(typeof input === 'string' ? input : input.url);
      if (url.hostname === 'ipc.localhost' && decodeURIComponent(url.pathname.slice(1)) === 'plugin:dialog|open') {
        window.__diskoraPickerIntercepted = true;
        return Promise.resolve(new Response(JSON.stringify(${JSON.stringify(fixture)}), {
          headers: { 'Content-Type': 'application/json', 'Tauri-Response': 'ok' },
        }));
      }
      return originalFetch(input, options);
    };
  })()`);
  await until(() => click('Scan a folder…'), 'folder scan button');
  await until(() => evaluate('window.__diskoraPickerIntercepted === true'), 'intercepted folder picker');
  await until(() => evaluate(`!!document.querySelector('[data-testid="analyzed-size"]')`), 'completed scan in UI');
  // This build has no Pro licence, so the backend must hand out identity tokens and masked paths only.
  // The newest scan is the one that just completed.
  const summary = await evaluate(`window.__TAURI_INTERNALS__.invoke('scan_list').then(rows => rows[0])`);
  assert.ok(summary && String(summary.rootPath).startsWith('ref:'), 'free tier must receive an identity token, not the real root path');
  assert.equal(await evaluate(`window.__TAURI_INTERNALS__.invoke('scan_list').then(rows => JSON.stringify(rows).includes(${JSON.stringify(fixture)}))`), false, 'real path leaked through scan_list');
  assert.equal(summary.totalBytes, 3 * 1024 * 1024 + 11);
  assert.equal(summary.fileCount, 4);
  assert.equal(summary.folderCount, 1);
  assert.equal(summary.status, 'complete');
  await until(() => click('Large Files'), 'large-file navigation');
  await evaluate(`(() => {
    const input = document.querySelector('input[placeholder="e.g. 750 MB"]');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, '1 MB');
    input.dispatchEvent(new Event('input', { bubbles: true }));
  })()`);
  await until(() => evaluate(`document.querySelector('[data-testid="large-files-count"]')?.textContent === '3'`), 'filtered large-file results');
  await until(() => click('Duplicate Finder'), 'duplicate navigation');
  await until(() => click('Find duplicates'), 'duplicate scan button');
  await until(() => evaluate(`document.body.textContent.includes('2 copies ·')`), 'real duplicate report');
  await evaluate(`([...document.querySelectorAll('button')].find(button => button.textContent.includes('2 copies ·'))).click()`);
  await until(() => evaluate(`document.body.textContent.includes('a.bin') && document.body.textContent.includes('b.bin')`), 'duplicate paths');
  assert.equal(await evaluate(`document.querySelector('[data-testid="duplicates-view"]').textContent.includes('c.bin')`), false);
  assert.equal(await evaluate(`document.body.innerText.includes(${JSON.stringify(fixture)})`), false, 'real path leaked into the UI');
  assert.equal(errors.length, 0, JSON.stringify(errors));
  console.log('PASS: renamed executable launched outside the build tree; real UI scanned 4 files including a Unicode folder, reported exact totals and found 2 content-identical copies (not the equal-sized different file).');
} catch (error) {
  console.error(`::error title=Desktop smoke test::${String(error).replaceAll('\n', '%0A')}`);
  if (socket?.readyState === WebSocket.OPEN) {
    try { console.error(`::error title=Desktop contents::${(await evaluate('document.body.innerText')).slice(-2500).replaceAll('\n', '%0A')}`); } catch { /* Keep the original smoke failure as the primary diagnostic. */ }
  }
  throw error;
} finally {
  socket?.close();
  if (app.pid) spawnSync('taskkill', ['/PID', String(app.pid), '/T', '/F'], { stdio: 'ignore' });
  // Windows may retain a WebView2 handle briefly; leave only a temp directory if so.
  try { rmSync(directory, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }); } catch { /* WebView2 may retain handles after taskkill; this directory is temporary. */ }
}
