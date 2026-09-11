// Full-OS-process restart acceptance for issue #5 (Alpha 0 desktop client).
//
// What prior acceptance proved: browser + WebView2 live messaging, offline
// backlog reconnect, account isolation, RENDERER reload. What it never proved:
// a full OS-process restart of the built exe (terminate the native process
// entirely, relaunch a fresh process, login again, history intact).
//
// This harness owns apps/desktop/** test tooling + evidence only. No product
// code is touched. Method: synthetic loopback backend + isolated Postgres in a
// unique container `unknownchat-restart-*` (exact `docker rm --force --volumes`
// cleanup, never volume prune), test-config exe build, two real native
// windows, login, `bro` + synthetic backlog over the real composer/HTTP/WS
// path, TERMINATE both OS processes (taskkill /T /F), verify absence,
// relaunch FRESH processes from the same exe with clean profiles, login
// again, assert full history with zero gaps / zero duplicates plus composer
// and plaintext notice intact, plus one post-restart live send.
//
// Run from apps/desktop: `node acceptance/restart-exe.mjs`
// Evidence: evidence/client-acceptance/<runId>/result.json (+ screenshots,
// backend/postgres logs, commands.txt). Local only, gitignored, synthetic.
// NEVER stages/commits/pushes, never touches any other worktree exe.
import assert from 'node:assert/strict';
import { spawn, spawnSync, execFileSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { createServer as netServer } from 'node:net';
import { mkdirSync, writeFileSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { createWriteStream } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { chromium } from 'playwright-core';
import { build } from 'vite';
import react from '@vitejs/plugin-react';

const desktop = fileURLToPath(new URL('../', import.meta.url));
const repo = path.resolve(desktop, '../..');
const extra = process.argv.slice(2);
assert.deepEqual(extra, [], 'restart-exe takes no arguments; it always runs the full exe-restart campaign');

const runId = new Date().toISOString().replace(/[:.]/g, '-');
const safe = runId.toLowerCase().replace(/[^a-z0-9-]/g, '-').slice(0, 40);
const container = `unknownchat-restart-${safe}`;
const password = 'Synthetic-restart-only-7!';
const artifact = path.join(desktop, 'evidence', 'client-acceptance', runId);
const runtime = path.join(desktop, '.acceptance');
mkdirSync(artifact, { recursive: true });
mkdirSync(runtime, { recursive: true });
writeFileSync(path.join(runtime, '.env'), '# dedicated synthetic restart acceptance runtime\n');

const env = {};
for (const key of ['PATH', 'Path', 'SYSTEMROOT', 'SystemRoot', 'WINDIR', 'COMSPEC', 'PATHEXT', 'TEMP', 'TMP', 'USERPROFILE', 'LOCALAPPDATA', 'APPDATA']) {
  if (process.env[key] !== undefined) env[key] = process.env[key];
}

const BACKLOG_N = 30; // + `bro` => 31 pre-restart; + 1 post-restart live send => 32
const NOTICE = 'Alpha demo: envelopes carry demo plaintext — not end-to-end encrypted.';
const COMPOSER = 'Message (demo plaintext → opaque envelope)';

const children = [];
const browsers = [];
const profiles = [];
let ownedContainer = false;
const commandLog = [];
const observations = {
  scope: 'exe-restart-campaign',
  mode: 'native-webview2-plugin-http-exe-restart',
  arguments: [],
  runId,
  container,
  fullRestartComplete: false,
  result: 'FAIL',
  steps: [],
  generations: {},
};

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
  p.on('error', (e) => { p.launchError = e; });
  if (wait) {
    await new Promise((resolve, reject) => {
      p.once('error', reject);
      p.once('exit', (code) => (code === 0 ? resolve() : reject(new Error(`${name} exited ${code}; see ${name}.log`))));
    });
  }
  return p;
}
async function until(fn, label, ms = 60000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await fn()) return;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`Timed out: ${label}`);
}
async function freePort() {
  const s = netServer();
  await new Promise((r) => s.listen(0, '127.0.0.1', r));
  const p = s.address().port;
  await new Promise((r) => s.close(r));
  return p;
}
function step(name, detail = {}) {
  observations.steps.push({ name, ...detail });
  console.log(`PASS ${name}`, JSON.stringify(detail));
}

// Evidence dump of one server history page: seq order, idempotency keys,
// content hash (never plaintext). Lets a re-verifier check history without
// re-trusting the live asserts.
function dumpHistory(rows) {
  return rows.map((m) => ({
    seq: m.seq,
    client_msg_id: m.client_msg_id,
    contentHash: createHash('sha256').update(String(m.ciphertext_b64 ?? m.body ?? m.snippet ?? m.client_msg_id)).digest('hex').slice(0, 16),
  }));
}
async function api(base, method, route, body, token) {
  const response = await fetch(base + route, {
    method,
    headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(15000),
  });
  assert.ok(response.ok, `${method} ${route}: ${response.status}`);
  const text = await response.text();
  return text ? JSON.parse(text) : undefined;
}
async function login(page, handle) {
  await page.getByPlaceholder('handle', { exact: true }).fill(handle);
  await page.getByPlaceholder('password', { exact: true }).fill(password);
  await page.getByRole('button', { name: 'Log in', exact: true }).click();
  await page.locator('[title="gateway: connected"]').first().waitFor({ timeout: 30000 });
}
async function openDm(page, handle) {
  await page.getByPlaceholder('peer handle → open DM').fill(handle);
  await page.getByRole('button', { name: 'Open DM', exact: true }).click();
  await page.locator('main').getByText(handle, { exact: true }).waitFor({ timeout: 30000 });
  await page.getByPlaceholder(COMPOSER).waitFor({ timeout: 30000 });
}
async function send(page, text) {
  await page.getByPlaceholder(COMPOSER).fill(text);
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await until(async () => (await page.getByPlaceholder(COMPOSER).inputValue()) === '', 'send acknowledged');
}
async function bodies(page, expected) {
  await until(
    async () => (await page.locator('main p.whitespace-pre-wrap').allTextContents()).length === expected.length,
    `render ${expected.length} messages`,
    60000,
  );
  assert.deepEqual(await page.locator('main p.whitespace-pre-wrap').allTextContents(), expected);
  const seqs = await page.locator('main p.text-\\[10px\\]').allTextContents();
  assert.deepEqual(
    seqs.map((s) => Number(s.match(/^#(\d+)/)?.[1])),
    expected.map((_, i) => i + 1),
  );
}
function procAlive(pid) {
  try {
    command('powershell.exe', ['-NoProfile', '-Command', `Get-Process -Id ${pid} -ErrorAction Stop | Out-Null`]);
    return true;
  } catch { return false; }
}
async function launchGeneration(tag, exePath, debugPorts) {
  const pages = [];
  const procs = [];
  for (const key of ['a', 'b']) {
    const profile = path.join(runtime, `restart-${runId}-${tag}-${key}`);
    profiles.push(profile);
    const app = await child(
      exePath, [],
      `restart-${tag}-${key}`,
      { env: { ...env, WEBVIEW2_USER_DATA_FOLDER: profile, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debugPorts[key]} --remote-debugging-address=127.0.0.1` } },
    );
    procs.push(app);
    await until(async () => {
      if (app.launchError) throw app.launchError;
      if (app.exitCode !== null) throw new Error(`restart ${tag}/${key} exited ${app.exitCode}; see restart-${tag}-${key}.log`);
      try { return (await fetch(`http://127.0.0.1:${debugPorts[key]}/json/version`, { signal: AbortSignal.timeout(2000) })).ok; }
      catch { return false; }
    }, `restart ${tag}/${key} CDP`);
    const browser = await chromium.connectOverCDP(`http://127.0.0.1:${debugPorts[key]}`);
    browsers.push(browser);
    await until(() => browser.contexts().some((c) => c.pages().length), `restart ${tag}/${key} page`);
    const page = browser.contexts()[0].pages()[0];
    pages.push(page);
    const window = JSON.parse(command('powershell.exe', ['-NoProfile', '-Command', `Get-Process -Id ${app.pid} | Select-Object Id,MainWindowHandle,MainWindowTitle | ConvertTo-Json -Compress`]));
    assert.notEqual(window.MainWindowHandle, 0, `live native window handle (${tag}/${key})`);
    step(`restart ${tag}/${key} window`, window);
  }
  return { pages, procs };
}

let server = null;
try {
  observations.head = command('git', ['rev-parse', 'HEAD'], { cwd: repo });
  const sourceFiles = readdirSync(path.join(desktop, 'src'), { recursive: true }).filter((n) => /\.(tsx?|css)$/.test(n)).sort();
  observations.sourceSha256 = Object.fromEntries(
    sourceFiles.map((n) => [n, createHash('sha256').update(readFileSync(path.join(desktop, 'src', n))).digest('hex')]),
  );
  observations.harnessSha256 = createHash('sha256').update(readFileSync(fileURLToPath(import.meta.url))).digest('hex');

  assert.equal(command('docker', ['ps', '-a', '--filter', `name=^/${container}$`, '--format', '{{.ID}}']), '', 'reserved restart container already exists');
  await child('cargo', ['build', '--locked', '-p', 'terichat-server', '--manifest-path', path.join(repo, 'Cargo.toml'), '--target-dir', path.join(runtime, 'restart-server-target')], 'restart-server-build', {}, true);
  command('docker', ['run', '--detach', '--name', container, '--label', `unknownchat.restart=${runId}`, '--publish', '127.0.0.1::5432', '--env', 'POSTGRES_USER=restart', '--env', 'POSTGRES_PASSWORD=synthetic-local-db-only', '--env', 'POSTGRES_DB=restart_acceptance', 'postgres:16-alpine']);
  ownedContainer = true;
  await until(() => {
    try { return command('docker', ['exec', container, 'pg_isready', '-h', '127.0.0.1', '-U', 'restart', '-d', 'restart_acceptance']).includes('accepting connections'); }
    catch { return false; }
  }, 'PostgreSQL ready (TCP 127.0.0.1)');
  const mapping = command('docker', ['port', container, '5432/tcp']);
  assert.match(mapping, /^127\.0\.0\.1:\d+$/);
  const serverPort = await freePort();
  const backend = `http://127.0.0.1:${serverPort}`;
  server = await child(
    path.join(runtime, 'restart-server-target/debug/terichat-server.exe'), [],
    'restart-backend',
    { env: { ...env, DATABASE_URL: `postgres://restart:synthetic-local-db-only@${mapping}/restart_acceptance`, BIND_ADDR: '127.0.0.1', PORT: String(serverPort), RUST_LOG: 'info' } },
  );
  observations.startup = { backend, databaseBinding: mapping, pid: server.pid };
  await until(async () => {
    if (server.launchError) throw server.launchError;
    if (server.exitCode !== null) throw new Error(`Backend exited ${server.exitCode}; see restart-backend.log`);
    try {
      const response = await fetch(`${backend}/ready`, { signal: AbortSignal.timeout(2000) });
      observations.startup.readiness = { status: response.status, body: await response.text() };
      return response.ok;
    } catch (error) { observations.startup.lastConnectionError = error.message; return false; }
  }, 'backend readiness');
  step('restart: isolated backend startup and readiness', observations.startup);

  const accounts = {};
  for (const handle of ['restart_a', 'restart_b']) {
    await api(backend, 'POST', '/v1/auth/register', { handle, email: `${handle}@example.invalid`, display_name: handle, password });
    accounts[handle] = await api(backend, 'POST', '/v1/auth/login', { handle, password });
  }

  // Test-config exe: separate dist + separate Cargo target dir; exact loopback capability only.
  const dist = path.join(runtime, 'restart-dist');
  await build({ configFile: false, root: desktop, envDir: false, plugins: [react()], define: { 'import.meta.env.VITE_API_URL': JSON.stringify(backend) }, build: { outDir: dist, emptyOutDir: true } });
  const markerHtml = readFileSync(path.join(dist, 'index.html'), 'utf8');
  assert.ok(markerHtml.length > 0, 'restart dist built');
  const markerJs = readdirSync(path.join(dist, 'assets')).filter((f) => f.endsWith('.js'));
  assert.ok(markerJs.length > 0, 'restart dist has a JS bundle');
  const markerSrc = markerJs.map((f) => readFileSync(path.join(dist, 'assets', f), 'utf8')).join('\n');
  assert.ok(markerSrc.includes(backend), 'restart bundle points at the synthetic backend');
  const tauriConfig = path.join(runtime, 'tauri-restart-acceptance.json');
  writeFileSync(tauriConfig, JSON.stringify({
    identifier: 'com.terichat.restart-acceptance',
    build: { beforeBuildCommand: '', frontendDist: '../.acceptance/restart-dist' },
    app: { security: { capabilities: [{ identifier: 'restart-acceptance', windows: ['main'], permissions: ['http:default', { identifier: 'http:allow-fetch', allow: [{ url: backend }] }] }] } } },
  ));
  await child(process.execPath, [path.join(desktop, 'node_modules/@tauri-apps/cli/tauri.js'), 'build', '--debug', '--no-bundle', '--config', tauriConfig, '--', '--locked'], 'restart-native-build', { cwd: desktop, env: { ...env, CARGO_TARGET_DIR: path.join(runtime, 'restart-target') } }, true);
  const exePath = path.join(runtime, 'restart-target/debug/terichat-desktop.exe');
  observations.exePath = exePath;
  observations.exeSha256 = createHash('sha256').update(readFileSync(exePath)).digest('hex');
  assert.match(observations.exeSha256, /^[a-f0-9]{64}$/);
  step('restart: test-config exe built', { exeSha256: observations.exeSha256 });

  // Generation 1: two real windows, login, bro + backlog.
  const gen1Ports = { a: await freePort(), b: await freePort() };
  const gen1 = await launchGeneration('gen1', exePath, gen1Ports);
  const [a1, b1] = gen1.pages;
  for (const [i, page] of gen1.pages.entries()) {
    await page.getByPlaceholder('handle', { exact: true }).waitFor({ timeout: 30000 });
    await page.screenshot({ path: path.join(artifact, `restart-gen1-login-${i}.png`) });
  }
  await login(a1, 'restart_a');
  await login(b1, 'restart_b');
  await openDm(a1, 'restart_b');
  await openDm(b1, 'restart_a');
  await send(a1, 'bro');
  await bodies(b1, ['bro']);
  assert.ok(await b1.getByText(NOTICE, { exact: true }).isVisible(), 'plaintext notice visible pre-restart');
  step('restart gen1: two logins; A sends bro; B receives live; plaintext notice visible');
  const dm = (await api(backend, 'GET', '/v1/conversations', undefined, accounts.restart_a.token)).find((c) => c.peer_handle === 'restart_b');
  assert.ok(dm?.id, 'DM conversation found');
  observations.conversationId = dm.id;
  const expected = ['bro'];
  for (let i = 1; i <= BACKLOG_N; i++) {
    const text = `restart-${String(i).padStart(3, '0')}`;
    await send(a1, text);
    expected.push(text);
  }
  await bodies(b1, expected);
  observations.preRestartTotal = expected.length;
  await b1.screenshot({ path: path.join(artifact, 'restart-pre-restart.png') });
  const preHistory = await api(backend, 'GET', `/v1/messages?conversation_id=${dm.id}&since_seq=0&limit=100`, undefined, accounts.restart_b.token);
  assert.equal(preHistory.length, expected.length, 'pre-restart server history length');
  assert.deepEqual(preHistory.map((m) => m.seq), expected.map((_, i) => i + 1), 'pre-restart seqs contiguous');
  assert.equal(new Set(preHistory.map((m) => m.client_msg_id)).size, preHistory.length, 'pre-restart no duplicate client ids');
  // Server-truth dump persisted into evidence: seq order, idempotency keys,
  // and a content hash (never plaintext) so a re-verifier need not re-trust
  // the harness asserts.
  observations.preHistory = dumpHistory(preHistory);
  step('restart gen1: backlog complete with contiguous seqs', { total: expected.length });

  // FULL OS-PROCESS RESTART: terminate both native processes entirely.
  const gen1Pids = gen1.procs.map((p) => p.pid);
  observations.generations.gen1 = {
    pids: gen1Pids,
    debugPorts: gen1Ports,
  };
  for (const p of gen1.procs) {
    const out = command('taskkill.exe', ['/PID', String(p.pid), '/T', '/F']);
    observations[`taskkill-gen1-${p.pid}`] = out;
  }
  for (const p of gen1.procs) {
    await until(() => p.exitCode !== null || p.signalCode !== null, `gen1 pid ${p.pid} exit`, 15000);
  }
  await new Promise((r) => setTimeout(r, 1500));
  for (const pid of gen1Pids) assert.equal(procAlive(pid), false, `gen1 pid ${pid} fully gone`);
  // Close gen1 CDP sessions (browser endpoints died with the processes).
  for (const browser of browsers.splice(0, browsers.length)) { try { await browser.close(); } catch {} }
  // Delete gen1 WebView profiles only after their processes are gone.
  const gen1Profiles = profiles.splice(0, profiles.length);
  for (const profile of gen1Profiles) rmSync(profile, { recursive: true, force: true, maxRetries: 30, retryDelay: 200 });
  step('restart: both gen1 OS processes terminated and absent; profiles removed', { pids: gen1Pids });

  // Generation 2: FRESH processes from the same exe, clean profiles, login again.
  const gen2Ports = { a: await freePort(), b: await freePort() };
  const gen2 = await launchGeneration('gen2', exePath, gen2Ports);
  const [a2, b2] = gen2.pages;
  const gen2Pids = gen2.procs.map((p) => p.pid);
  observations.generations.gen2 = { pids: gen2Pids, debugPorts: gen2Ports };
  for (const pid of gen2Pids) assert.ok(!gen1Pids.includes(pid), `fresh pid ${pid} differs from gen1`);
  await login(a2, 'restart_a');
  await login(b2, 'restart_b');
  await openDm(a2, 'restart_b');
  await openDm(b2, 'restart_a');
  await bodies(b2, expected);
  await bodies(a2, expected);
  observations.postRestartTotal = expected.length;
  await b2.screenshot({ path: path.join(artifact, 'restart-post-restart.png') });
  step('restart gen2: fresh processes login; full history restored with zero gaps/duplicates', { total: expected.length });

  // Post-restart integrity: composer + plaintext notice + server truth + live send.
  assert.ok(await b2.getByPlaceholder(COMPOSER).isVisible(), 'composer visible post-restart');
  assert.ok(await b2.getByPlaceholder(COMPOSER).isEnabled(), 'composer enabled post-restart');
  assert.ok(await a2.getByPlaceholder(COMPOSER).isVisible(), 'composer visible post-restart (A)');
  assert.ok(await b2.getByText(NOTICE, { exact: true }).isVisible(), 'plaintext notice intact post-restart');
  assert.ok(await a2.getByText(NOTICE, { exact: true }).isVisible(), 'plaintext notice intact post-restart (A)');
  const postHistory = await api(backend, 'GET', `/v1/messages?conversation_id=${dm.id}&since_seq=0&limit=100`, undefined, accounts.restart_b.token);
  assert.equal(postHistory.length, expected.length, 'post-restart server history length');
  assert.deepEqual(postHistory.map((m) => m.seq), expected.map((_, i) => i + 1), 'post-restart seqs contiguous');
  assert.equal(new Set(postHistory.map((m) => m.client_msg_id)).size, postHistory.length, 'post-restart no duplicates');
  observations.postHistory = dumpHistory(postHistory);
  step('restart gen2: composer and plaintext notice intact; server history contiguous');
  await send(b2, 'restart-back');
  expected.push('restart-back');
  await bodies(a2, expected);
  await bodies(b2, expected);
  const finalHistory = await api(backend, 'GET', `/v1/messages?conversation_id=${dm.id}&since_seq=0&limit=100`, undefined, accounts.restart_a.token);
  assert.equal(finalHistory.length, expected.length, 'final history length after post-restart send');
  assert.deepEqual(finalHistory.map((m) => m.seq), expected.map((_, i) => i + 1), 'final seqs contiguous');
  observations.finalHistory = dumpHistory(finalHistory);
  await b2.screenshot({ path: path.join(artifact, 'restart-final.png') });
  step('restart gen2: post-restart live send received both ways', { total: expected.length });

  observations.expectedBodies = expected;
  observations.fullRestartComplete = true;
  observations.result = 'PASS';
} catch (error) {
  observations.result = 'FAIL';
  observations.error = error.stack;
  console.error(error.stack);
  process.exitCode = 1;
} finally {
  const cleanupErrors = [];
  const cleanup = async (fn) => { try { await fn(); } catch (error) { cleanupErrors.push(error.message); process.exitCode = 1; } };
  for (const browser of browsers.splice(0, browsers.length)) await cleanup(() => browser.close().catch(() => {}));
  for (const p of children.reverse()) {
    if (p.exitCode === null && p.signalCode === null) {
      await cleanup(async () => {
        command('taskkill.exe', ['/PID', String(p.pid), '/T', '/F']);
        await until(() => p.exitCode !== null || p.signalCode !== null, 'owned process exit', 15000);
      });
    }
  }
  for (const profile of profiles.splice(0, profiles.length)) {
    await cleanup(() => rmSync(profile, { recursive: true, force: true, maxRetries: 30, retryDelay: 200 }));
  }
  if (ownedContainer) {
    await cleanup(() => {
      const label = command('docker', ['inspect', '--format', '{{index .Config.Labels "unknownchat.restart"}}', container]);
      assert.equal(label, runId, 'cleanup ownership label');
      const volumes = JSON.parse(command('docker', ['inspect', '--format', '{{json .Mounts}}', container])).filter((m) => m.Type === 'volume').map((m) => m.Name);
      const logs = spawnSync('docker', ['logs', container], { env, encoding: 'utf8', timeout: 15000 });
      writeFileSync(path.join(artifact, 'postgres.log'), (logs.stdout ?? '') + (logs.stderr ?? ''));
      command('docker', ['rm', '--force', '--volumes', container]);
      observations.containerRemoved = command('docker', ['ps', '-a', '--filter', `name=^/${container}$`, '--format', '{{.ID}}']) === '';
      const remaining = command('docker', ['volume', 'ls', '--format', '{{.Name}}']).split('\n');
      observations.volumesRemoved = volumes.every((v) => !remaining.includes(v));
      assert.ok(observations.containerRemoved && observations.volumesRemoved, 'owned Docker resources removed');
    });
  }
  observations.cleanupErrors = cleanupErrors;
  if (cleanupErrors.length) { observations.result = 'FAIL'; observations.fullRestartComplete = false; }
  writeFileSync(path.join(artifact, 'commands.txt'), `${commandLog.join('\n')}\n`);
  writeFileSync(path.join(artifact, 'result.json'), JSON.stringify(observations, null, 2));
  console.log(`Evidence: ${artifact}`);
}
