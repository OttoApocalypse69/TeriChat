// Synthetic local-only acceptance. No existing DB, dotenv file, or browser profile is used.
import assert from 'node:assert/strict';
import { spawn, spawnSync, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer as httpServer, request } from 'node:http';
import { createServer as netServer } from 'node:net';
import { mkdirSync, writeFileSync, createWriteStream, readFileSync, readdirSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { chromium } from 'playwright-core';
import { createServer, build } from 'vite';
import react from '@vitejs/plugin-react';
import { validateHistory, validateFullAcceptance } from './report.mjs';
import { verifyChatLayout } from './chat-layout.mjs';

const desktop = fileURLToPath(new URL('../', import.meta.url));
const repo = path.resolve(desktop, '../..');
const args = process.argv.slice(2);
assert.ok(args.every(arg => ['--native', '--probe-only', '--ui-probe'].includes(arg)), 'unknown acceptance argument');
assert.equal(new Set(args).size, args.length, 'duplicate acceptance argument');
assert.ok(!(args.includes('--probe-only') && args.includes('--ui-probe')), 'choose one probe scope');
const native = args.includes('--native');
const scope = args.includes('--probe-only') ? 'backend-probe' : args.includes('--ui-probe') ? 'ui-probe' : 'full-campaign';
const runId = new Date().toISOString().replace(/[:.]/g, '-');
const artifact = path.join(desktop, 'evidence', 'client-acceptance', runId);
const runtime = path.join(desktop, '.acceptance');
mkdirSync(artifact, { recursive: true });
mkdirSync(runtime, { recursive: true });
// Stop dotenvy's upward search without reading any existing .env.
writeFileSync(path.join(runtime, '.env'), '# dedicated synthetic acceptance runtime\n');
const env = {};
// Build/OS plumbing only; never inherit DB URLs, auth tokens, signing keys, or VITE variables.
for (const key of ['PATH', 'Path', 'SYSTEMROOT', 'SystemRoot', 'WINDIR', 'COMSPEC', 'PATHEXT', 'TEMP', 'TMP', 'USERPROFILE', 'LOCALAPPDATA', 'APPDATA']) {
  if (process.env[key] !== undefined) env[key] = process.env[key];
}
const container = 'unknownchat-client-acceptance';
const password = 'Synthetic-acceptance-only-5!';
const children = [];
const browsers = [];
const profiles = [];
let ownedContainer = false;
let vite;
let asynchronousError;
let proxy;
const sockets = new Map();
const blocked = new Set();
let denyHistory = null;
let phase = 'startup';
let activeClient = 'unknown';
const clientLabels = new Map();
const observations = { arguments: args, scope, fullCampaignComplete: false, mode: native ? 'native-webview2-plugin-http' : 'browser-test-only-fetch-adapter', steps: [], history: [], frames: { a: [], b: [] } };
const commandLog = [];
function command(exe, args, options = {}) {
  commandLog.push([exe, ...args].join(' '));
  return execFileSync(exe, args, { cwd: runtime, env, encoding: 'utf8', timeout: 120000, ...options }).trim();
}
async function child(exe, args, name, options = {}, wait = false) {
  commandLog.push([exe, ...args].join(' '));
  const log = createWriteStream(path.join(artifact, `${name}.log`));
  const p = spawn(exe, args, { cwd: runtime, env, windowsHide: true, ...options, stdio: ['ignore', 'pipe', 'pipe'] });
  children.push(p);
  p.stdout.pipe(log); p.stderr.pipe(log);
  p.on('error', e => { p.launchError = e; });
  if (wait) await new Promise((resolve, reject) => {
    p.once('error', reject);
    p.once('exit', code => code === 0 ? resolve() : reject(new Error(`${name} exited ${code}; see ${name}.log`)));
  });
  return p;
}
async function until(fn, label, ms = 30000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (asynchronousError) throw asynchronousError;
    if (await fn()) return;
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error(`Timed out: ${label}`);
}
async function port() {
  const s = netServer(); await new Promise(r => s.listen(0, '127.0.0.1', r));
  const p = s.address().port; await new Promise(r => s.close(r)); return p;
}
function step(name, detail = {}) { observations.steps.push({ name, ...detail }); console.log(`PASS ${name}`, JSON.stringify(detail)); }
function drop(token) { for (const s of sockets.get(token) ?? []) s.destroy(); }
async function api(base, method, route, body, token) {
  const response = await fetch(base + route, { method, headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) }, body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(10000) });
  assert.ok(response.ok, `${method} ${route}: ${response.status}`);
  const text = await response.text(); return text ? JSON.parse(text) : undefined;
}
async function login(page, handle) {
  await page.getByPlaceholder('handle', { exact: true }).fill(handle);
  await page.getByPlaceholder('password', { exact: true }).fill(password);
  await page.getByRole('button', { name: 'Log in', exact: true }).click();
  await page.locator('[title="gateway: connected"]').first().waitFor();
}
async function openDm(page, handle) {
  await page.getByPlaceholder('peer handle → open DM').fill(handle);
  await page.getByRole('button', { name: 'Open DM', exact: true }).click();
  await page.locator('main').getByRole('heading', { name: handle, exact: true }).waitFor();
  await page.getByPlaceholder('Message (demo plaintext → opaque envelope)').waitFor();
}
async function send(page, text) {
  await page.getByPlaceholder('Message (demo plaintext → opaque envelope)').fill(text);
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await until(async () => await page.getByPlaceholder('Message (demo plaintext → opaque envelope)').inputValue() === '', 'send acknowledged');
}
async function bodies(page, expected) {
  await until(async () => (await page.locator('main p.whitespace-pre-wrap').allTextContents()).length === expected.length, `render ${expected.length} messages`);
  assert.deepEqual(await page.locator('main p.whitespace-pre-wrap').allTextContents(), expected);
  const seqs = await page.locator('main .message-sequence').allTextContents();
  assert.deepEqual(seqs.map(s => Number(s.match(/^#(\d+)/)?.[1])), expected.map((_, i) => i + 1));
}
function observe(page, key) {
  page.on('websocket', ws => {
    ws.on('framesent', ({ payload }) => {
      try { const f = JSON.parse(payload.toString()); if (f.op === 'identify') observations.frames[key].push({ identify_resume_after: f.resume_after }); } catch {}
    });
    ws.on('framereceived', ({ payload }) => {
      try { const f = JSON.parse(payload.toString()); if (f.op === 'event') observations.frames[key].push({ event_id: f.event.event_id, conversation_id: f.event.payload.conversation_id }); } catch {}
    });
  });
}
try {
  // Fail rather than commandeer an existing container with the reserved name.
  observations.head = command('git', ['rev-parse', 'HEAD'], { cwd: repo });
  const sourceFiles = readdirSync(path.join(desktop, 'src'), { recursive: true }).filter(name => /\.(tsx?|css)$/.test(name)).sort();
  observations.sourceSha256 = Object.fromEntries(sourceFiles.map(name => [name, createHash('sha256').update(readFileSync(path.join(desktop, 'src', name))).digest('hex')]));
  observations.layoutHarnessSha256 = createHash('sha256').update(readFileSync(path.join(desktop, 'acceptance/chat-layout.mjs'))).digest('hex');
  observations.harnessSha256 = createHash('sha256').update(readFileSync(fileURLToPath(import.meta.url))).digest('hex');
  assert.equal(command('docker', ['ps', '-a', '--filter', `name=^/${container}$`, '--format', '{{.ID}}']), '', 'reserved container already exists');
  await child('cargo', ['build', '--locked', '-p', 'terichat-server', '--manifest-path', path.join(repo, 'Cargo.toml'), '--target-dir', path.join(runtime, 'server-target')], 'server-build', {}, true);
  command('docker', ['run', '--detach', '--name', container, '--label', `unknownchat.acceptance=${runId}`, '--publish', '127.0.0.1::5432', '--env', 'POSTGRES_USER=acceptance', '--env', 'POSTGRES_PASSWORD=synthetic-local-db-only', '--env', 'POSTGRES_DB=client_acceptance', 'postgres:16-alpine']);
  ownedContainer = true;
  // The image's temporary init server accepts Unix sockets before its restart.
  // TCP readiness excludes that phase and prevents a backend connect/EOF race.
  await until(() => { try { return command('docker', ['exec', container, 'pg_isready', '-h', '127.0.0.1', '-U', 'acceptance', '-d', 'client_acceptance']).includes('accepting connections'); } catch { return false; } }, 'PostgreSQL ready');
  const mapping = command('docker', ['port', container, '5432/tcp']);
  assert.match(mapping, /^127\.0\.0\.1:\d+$/);
  const serverPort = await port();
  const backend = `http://127.0.0.1:${serverPort}`;
  const server = await child(path.join(runtime, 'server-target/debug/terichat-server.exe'), [], 'backend', { env: { ...env, DATABASE_URL: `postgres://acceptance:synthetic-local-db-only@${mapping}/client_acceptance`, BIND_ADDR: '127.0.0.1', PORT: String(serverPort), RUST_LOG: 'info' } });
  observations.startup = { backend, databaseBinding: mapping, pid: server.pid };
  await until(async () => {
    if (server.launchError) throw server.launchError;
    if (server.exitCode !== null) throw new Error(`Backend exited ${server.exitCode}; see backend.log`);
    try {
      const response = await fetch(backend + '/ready', { signal: AbortSignal.timeout(2000) });
      observations.startup.readiness = { status: response.status, body: await response.text() };
      return response.ok;
    } catch (error) { observations.startup.lastConnectionError = error.message; return false; }
  }, 'backend readiness');
  step('isolated backend startup and readiness', observations.startup);

  if (!process.argv.includes('--probe-only')) {
  // A loopback fault proxy forwards actual bytes to the actual backend. It can
  // disconnect one session or fail history, never synthesize successful API data.
  proxy = httpServer((req, res) => {
    const token = req.headers.authorization?.replace(/^Bearer /, '');
    const url = new URL(req.url, backend);
    const isHistory = req.method === 'GET' && url.pathname === '/v1/messages';
    if (token && !clientLabels.has(token)) clientLabels.set(token, activeClient);
    const observation = isHistory ? { client: clientLabels.get(token) ?? 'unknown', phase, conversation: url.searchParams.get('conversation_id'), since: Number(url.searchParams.get('since_seq')), limit: Number(url.searchParams.get('limit')), status: null, size: null } : null;
    if (observation) observations.history.push(observation);
    if (blocked.has(token) || (isHistory && denyHistory?.token === token && denyHistory.id === url.searchParams.get('conversation_id'))) {
      if (observation) observation.status = 503;
      res.writeHead(503, { 'content-type': 'application/json' }); res.end('{"error":{"code":"acceptance_fault","message":"synthetic history outage"}}'); return;
    }
    const upstream = request(url, { method: req.method, headers: req.headers }, incoming => {
      if (observation) {
        observation.status = incoming.statusCode;
        const chunks = [];
        incoming.on('data', chunk => chunks.push(chunk));
        incoming.on('end', () => {
          try {
            const body = JSON.parse(Buffer.concat(chunks).toString());
            if (Array.isArray(body)) observation.size = body.length;
          } catch { /* Invalid responses cannot satisfy the pagination validator. */ }
        });
      }
      res.writeHead(incoming.statusCode, incoming.headers); incoming.pipe(res);
    });
    upstream.on('error', () => { res.writeHead(502); res.end(); }); req.pipe(upstream);
  });
  proxy.on('upgrade', (req, socket, head) => {
    const token = new URL(req.url, backend).searchParams.get('token');
    if (blocked.has(token)) { socket.destroy(); return; }
    if (!clientLabels.has(token)) clientLabels.set(token, activeClient);
    if (!sockets.has(token)) sockets.set(token, new Set());
    sockets.get(token).add(socket); socket.on('close', () => sockets.get(token).delete(socket));
    const upstream = request(new URL(req.url, backend), { headers: req.headers });
    upstream.on('upgrade', (res, remote, remoteHead) => {
      socket.write(`HTTP/1.1 ${res.statusCode} Switching Protocols\r\n${Object.entries(res.headers).map(([k, v]) => `${k}: ${v}`).join('\r\n')}\r\n\r\n`);
      if (remoteHead.length) socket.write(remoteHead); if (head.length) remote.write(head);
      socket.pipe(remote); remote.pipe(socket);
      socket.on('close', () => remote.destroy()); remote.on('error', () => socket.destroy()); socket.on('error', () => remote.destroy());
    });
    upstream.on('error', () => socket.destroy()); upstream.end();
  });
  await new Promise(r => proxy.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${proxy.address().port}`;
  const accounts = {};
  for (const handle of ['acceptance_a', 'acceptance_b', 'acceptance_quiet']) {
    await api(backend, 'POST', '/v1/auth/register', { handle, email: `${handle}@example.invalid`, display_name: handle, password });
    accounts[handle] = await api(backend, 'POST', '/v1/auth/login', { handle, password });
  }
  let pages;
  if (native) {
    const dist = path.join(runtime, 'native-dist');
    await build({ configFile: false, root: desktop, envDir: false, plugins: [react()], define: { 'import.meta.env.VITE_API_URL': JSON.stringify(base) }, build: { outDir: dist, emptyOutDir: true } });
    const config = path.join(runtime, 'tauri-acceptance.json');
    writeFileSync(config, JSON.stringify({ identifier: 'com.terichat.acceptance', build: { beforeBuildCommand: '', frontendDist: '../.acceptance/native-dist' }, app: { security: { capabilities: [{ identifier: 'acceptance', windows: ['main'], permissions: ['http:default', { identifier: 'http:allow-fetch', allow: [{ url: base }] }] }] } } }));
    await child(process.execPath, [path.join(desktop, 'node_modules/@tauri-apps/cli/tauri.js'), 'build', '--debug', '--no-bundle', '--config', config, '--', '--locked'], 'native-build', { cwd: desktop, env: { ...env, CARGO_TARGET_DIR: path.join(runtime, 'native-target') } }, true);
    pages = [];
    for (const key of ['a', 'b']) {
      const debugPort = await port();
      const profile = path.join(runtime, `webview-${runId}-${key}`);
      profiles.push(profile);
      const app = await child(path.join(runtime, 'native-target/debug/terichat-desktop.exe'), [], `native-${key}`, { env: { ...env, WEBVIEW2_USER_DATA_FOLDER: profile, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debugPort} --remote-debugging-address=127.0.0.1` } });
      await until(async () => { if (app.launchError) throw app.launchError; try { return (await fetch(`http://127.0.0.1:${debugPort}/json/version`)).ok; } catch { return false; } }, `native ${key} CDP`);
      const browser = await chromium.connectOverCDP(`http://127.0.0.1:${debugPort}`); browsers.push(browser);
      await until(() => browser.contexts().some(c => c.pages().length), 'native page');
      pages.push(browser.contexts()[0].pages()[0]);
      const window = JSON.parse(command('powershell.exe', ['-NoProfile', '-Command', `Get-Process -Id ${app.pid} | Select-Object Id,MainWindowHandle,MainWindowTitle | ConvertTo-Json -Compress`]));
      assert.notEqual(window.MainWindowHandle, 0, 'live native window handle');
      step(`native ${key} window`, window);
    }
    observations.nativeExecutableSha256 = createHash('sha256').update(readFileSync(path.join(runtime, 'native-target/debug/terichat-desktop.exe'))).digest('hex');
  } else {
    const adapter = path.join(desktop, 'acceptance/browser-http.mjs');
    const generated = [runtime, path.join(desktop, 'evidence/client-acceptance')];
    // Do not traverse Rust outputs or WebView profiles. Keep source watching on.
    const ignored = candidate => generated.some(dir => path.resolve(candidate) === dir || path.resolve(candidate).startsWith(dir + path.sep));
    vite = await createServer({ configFile: false, root: desktop, envDir: false, plugins: [react()], resolve: { alias: { '@tauri-apps/plugin-http': adapter } }, define: { 'import.meta.env.VITE_API_URL': JSON.stringify(base) }, optimizeDeps: { entries: ['index.html'] }, server: { host: '127.0.0.1', port: 0, strictPort: false, watch: { ignored } } });
    vite.watcher.on('error', error => { asynchronousError = error; });
    await vite.listen();
    const browser = await chromium.launch({ channel: 'msedge', headless: true }); browsers.push(browser);
    pages = [await (await browser.newContext()).newPage(), await (await browser.newContext()).newPage()];
    // Browser-only CORS bridge: forward HTTP to the same real loopback proxy;
    // the Tauri mode above uses the unmodified Rust plugin and capability.
    for (const page of pages) await page.route(`${base}/**`, async route => {
      if (route.request().resourceType() === 'websocket') return route.continue();
      const response = await route.fetch(); await route.fulfill({ response, headers: { ...response.headers(), 'access-control-allow-origin': '*' } });
    });
    for (const page of pages) await page.goto(vite.resolvedUrls.local[0]);
    await until(() => Object.keys(vite.watcher.getWatched()).some(dir => path.resolve(dir) === path.join(desktop, 'src')), 'source watcher ready');
    assert.ok(Object.keys(vite.watcher.getWatched()).every(dir => !ignored(dir)), 'generated runtime is excluded from watcher');
    step('source watching enabled; generated runtime/profile paths excluded');
  }
  const [a, b] = pages;
  for (const [i, page] of pages.entries()) {
    await page.getByPlaceholder('handle', { exact: true }).waitFor();
    await page.screenshot({ path: path.join(artifact, `login-${i}.png`) });
  }
  if (!process.argv.includes('--ui-probe')) {
  observe(a, 'a'); observe(b, 'b');
  phase = 'live';
  activeClient = 'a'; await login(a, 'acceptance_a');
  activeClient = 'b'; await login(b, 'acceptance_b');
  // Capture session identity only in memory; never persist bearer tokens.
  await until(() => sockets.size >= 2, 'two gateway sessions');
  const tokenB = [...sockets.keys()].find(token => clientLabels.get(token) === 'b');
  assert.ok(tokenB, 'B gateway session identified');
  await openDm(a, 'acceptance_b'); await openDm(b, 'acceptance_a');
  await send(a, 'bro'); await bodies(b, ['bro']);
  assert.ok(observations.frames.b.some(f => f.event_id), 'B received a real gateway event');
  assert.ok(await b.getByText('Alpha demo: envelopes carry demo plaintext — not end-to-end encrypted.', { exact: true }).isVisible());
  step('two clients login; A sends bro; B receives live; plaintext warning visible');
  const dm = (await api(backend, 'GET', '/v1/conversations', undefined, accounts.acceptance_a.token)).find(c => c.peer_handle === 'acceptance_b');
  observations.backlogConversation = dm.id;
  phase = 'offline-backlog';
  blocked.add(tokenB); drop(tokenB);
  await b.locator('[title="gateway: reconnecting"]').first().waitFor();
  const expected = ['bro'];
  // Every backlog message uses the actual client composer, HTTP client, and backend.
  for (let i = 1; i <= 125; i++) { const text = `offline-${String(i).padStart(3, '0')}`; await send(a, text); expected.push(text); }
  assert.deepEqual(await b.locator('main p.whitespace-pre-wrap').allTextContents(), ['bro']);
  phase = 'backlog-reconnect';
  blocked.delete(tokenB);
  await b.locator('[title="gateway: connected"]').first().waitFor();
  await bodies(b, expected);
  validateHistory(observations.history, dm.id);
  phase = 'second-reconnect';
  await b.screenshot({ path: path.join(artifact, 'restored.png') });
  step('125-message offline backlog restored in sequence with no gaps or duplicates', { total: expected.length });
  const identifies = observations.frames.b.filter(f => 'identify_resume_after' in f).length;
  drop(tokenB);
  await until(() => observations.frames.b.filter(f => 'identify_resume_after' in f).length > identifies, 'second reconnect identify frame');
  assert.ok(observations.frames.b.filter(f => 'identify_resume_after' in f).at(-1).identify_resume_after, 'resume cursor is non-null');
  await b.locator('[title="gateway: connected"]').first().waitFor();
  await bodies(b, expected);
  step('second reconnect preserves exact message sequence');

  // Quiet DM: advance event cursor while HTTP history is unavailable. Recovery
  // must happen on reconnect even with no new event/message to trigger a fetch.
  phase = 'quiet-recovery';
  const quiet = await api(backend, 'POST', '/v1/conversations/dm', { peer_handle: 'acceptance_quiet' }, accounts.acceptance_b.token);
  await openDm(b, 'acceptance_quiet');
  await until(async () => await b.getByText('Loading history…', { exact: true }).count() === 0, 'quiet history initially drained');
  denyHistory = { token: tokenB, id: quiet.id };
  await api(backend, 'POST', '/v1/messages', { conversation_id: quiet.id, client_msg_id: crypto.randomUUID(), ciphertext_b64: Buffer.from('quiet-recovery').toString('base64') }, accounts.acceptance_quiet.token);
  await b.getByText('synthetic history outage', { exact: true }).waitFor();
  await until(() => observations.frames.b.some(f => f.conversation_id === quiet.id), 'quiet gateway event');
  assert.equal(await b.locator('main p.whitespace-pre-wrap').count(), 0, 'failed quiet history is not already rendered');
  const quietFrames = observations.frames.b.filter(f => f.conversation_id === quiet.id).length;
  denyHistory = null; drop(tokenB);
  await bodies(b, ['quiet-recovery']);
  assert.equal(observations.frames.b.filter(f => f.conversation_id === quiet.id).length, quietFrames, 'quiet recovery needs no replayed event');
  step('quiet conversation recovers failed history on reconnect without a new message');
  phase = 'account-switch';
  await b.getByRole('button', { name: 'Log out', exact: true }).click();
  await login(b, 'acceptance_a');
  assert.ok(!(await b.locator('body').innerText()).includes('quiet-recovery'));
  assert.ok(!(await b.locator('body').innerText()).includes('acceptance_quiet'));
  await openDm(b, 'acceptance_b'); await bodies(b, expected);
  step('account switch clears other account private conversation and restores own history');
  phase = 'renderer-reload';
  // Additional real synthetic fixtures for the three-pane/responsive UI checks.
  const layoutWorkspace = await api(backend, 'POST', '/v1/workspaces', { name: 'Acceptance Studio' }, accounts.acceptance_b.token);
  await api(backend, 'POST', `/v1/workspaces/${layoutWorkspace.id}/channels`, { name: 'general' }, accounts.acceptance_b.token);
  await b.reload(); await login(b, 'acceptance_b'); await openDm(b, 'acceptance_a'); await bodies(b, expected);
  step('full renderer reload/login restores all history without duplicates');
  await b.screenshot({ path: path.join(artifact, 'final.png') });
  observations.chatLayout = await verifyChatLayout(b, artifact, send, native, text => send(a, text));
  step('responsive chat navigation, wrapping, composer and draft preservation');
  observations.fullCampaignComplete = true;
  } else step(`${native ? 'native' : 'browser'} login DOM and screenshot verified`);
  }
  observations.result = observations.fullCampaignComplete ? 'PASS' : 'PROBE_PASS';
} catch (error) {
  observations.result = 'FAIL'; observations.error = error.stack;
  console.error(error.stack); process.exitCode = 1;
} finally {
  const cleanupErrors = [];
  const cleanup = async fn => { try { await fn(); } catch (error) { cleanupErrors.push(error.message); process.exitCode = 1; } };
  // Drain browser-adapter requests before disposing their request contexts.
  // A failing assertion can otherwise race route.fetch and abort all cleanup.
  for (const browser of browsers) {
    for (const context of browser.contexts()) for (const page of context.pages()) {
      await cleanup(() => page.unrouteAll({ behavior: 'wait' }));
    }
    await browser.close().catch(() => {});
  }
  for (const p of children.reverse()) if (p.exitCode === null) {
    await cleanup(async () => {
      command('taskkill.exe', ['/PID', String(p.pid), '/T', '/F']);
      await until(() => p.exitCode !== null || p.signalCode !== null, 'owned process exit', 10000);
    });
  }
  for (const profile of profiles) await cleanup(() => rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 }));
  for (const set of sockets.values()) for (const socket of set) socket.destroy();
  if (vite) await cleanup(() => vite.close());
  if (proxy) await cleanup(async () => { proxy.closeAllConnections(); await new Promise(r => proxy.close(r)); });
  if (ownedContainer) {
    await cleanup(() => {
    const label = command('docker', ['inspect', '--format', '{{index .Config.Labels "unknownchat.acceptance"}}', container]);
    assert.equal(label, runId, 'cleanup ownership label');
    const volumes = JSON.parse(command('docker', ['inspect', '--format', '{{json .Mounts}}', container])).filter(m => m.Type === 'volume').map(m => m.Name);
    const logs = spawnSync('docker', ['logs', container], { env, encoding: 'utf8', timeout: 10000 });
    writeFileSync(path.join(artifact, 'postgres.log'), logs.stdout + logs.stderr);
    command('docker', ['rm', '--force', '--volumes', container]);
    observations.containerRemoved = command('docker', ['ps', '-a', '--filter', `name=^/${container}$`, '--format', '{{.ID}}']) === '';
    const remaining = command('docker', ['volume', 'ls', '--format', '{{.Name}}']).split('\n');
    observations.volumesRemoved = volumes.every(v => !remaining.includes(v));
    assert.ok(observations.containerRemoved && observations.volumesRemoved, 'owned Docker resources removed');
    });
  }
  observations.cleanupErrors = cleanupErrors;
  if (cleanupErrors.length) observations.result = 'FAIL';
  if (observations.result === 'PASS') {
    try { validateFullAcceptance(observations); } catch (error) {
      observations.result = 'FAIL'; observations.error = error.stack; process.exitCode = 1;
    }
  }
  writeFileSync(path.join(artifact, 'commands.txt'), commandLog.join('\n') + '\n');
  writeFileSync(path.join(artifact, 'result.json'), JSON.stringify(observations, null, 2));
  console.log(`Evidence: ${artifact}`);
}
