// Synthetic loopback only. Real HTTP, PostgreSQL and outbox projection; no API fixtures.
// Browser transport evidence, not Tauri/native notification proof.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { randomUUID, createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';
import { chromium } from 'playwright-core';
import { createLogger, createServer } from 'vite';

const desktop = fileURLToPath(new URL('../', import.meta.url));
process.chdir(desktop);
const backend = process.argv[2];
assert.equal(process.argv.length, 3, 'one synthetic loopback backend URL required');
assert.match(backend ?? '', /^http:\/\/127\.0\.0\.1:[1-9]\d{0,4}$/);
const artifact = path.join(desktop, '.acceptance', 's3-controls-real');
mkdirSync(artifact, { recursive: true });
const report = {
  head: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
  tree: execFileSync('git', ['rev-parse', 'HEAD^{tree}'], { encoding: 'utf8' }).trim(),
  lockSha256: createHash('sha256').update(readFileSync('package-lock.json')).digest('hex'),
  scope: 'real browser / loopback API / PostgreSQL / outbox; native not tested',
  result: 'FAIL', steps: [],
};
let phase = 'startup';
let vite, browser, gate;
const check = label => { report.steps.push(label); console.log(`PASS ${label}`); };
async function request(method, route, token, body, expected = 200) {
  const response = await fetch(backend + route, {
    method, redirect: 'error', signal: AbortSignal.timeout(10000),
    headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  assert.equal(response.status, expected, 'unexpected real API status');
  const text = await response.text();
  return text ? JSON.parse(text) : null;
}
async function until(predicate, label) {
  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) { if (await predicate()) return; await sleep(100); }
  throw new Error(label);
}
// A one-shot forwarding barrier holds an actual backend response. It neither
// fabricates status/body nor installs Playwright request interception. Holding
// the response makes close/switch ordering deterministic even on fast runners.
function hold(method, route) {
  assert.ok(!gate, 'one forwarding barrier at a time');
  let release;
  const waiting = new Promise(resolve => { release = resolve; });
  gate = { method, route, waiting, release, reached: false, delivered: false };
  return gate;
}
const barrierPlugin = {
  name: 'real-response-barrier',
  configureServer(server) {
    server.middlewares.use(async (req, res, next) => {
      const active = gate;
      if (!active || active.claimed || req.method !== active.method || req.url !== active.route) return next();
      active.claimed = true;
      try {
        const response = await fetch(backend + req.url, {
          method: req.method, redirect: 'error', signal: AbortSignal.timeout(10000),
          headers: { authorization: req.headers.authorization ?? '' },
        });
        const body = Buffer.from(await response.arrayBuffer());
        active.status = response.status;
        active.reached = true;
        let timer;
        try {
          await Promise.race([active.waiting, new Promise((_, reject) => {
            timer = setTimeout(() => reject(new Error('barrier timeout')), 20000);
          })]);
        } finally { clearTimeout(timer); }
        res.statusCode = response.status;
        if (response.headers.has('content-type')) res.setHeader('content-type', response.headers.get('content-type'));
        res.end(body);
        active.delivered = true;
      } catch { active.failed = true; res.destroy(); }
    });
  },
};
async function release(active, page) {
  const received = page.waitForResponse(r => new URL(r.url()).pathname === active.route && r.request().method() === active.method);
  active.release();
  await (await received).finished();
  await until(() => active.delivered || active.failed, 'forwarding completion');
  assert.ok(!active.failed, 'real forwarding failed');
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  gate = undefined;
}
async function login(page, handle, password) {
  await page.getByPlaceholder('handle', { exact: true }).fill(handle);
  await page.getByPlaceholder('password', { exact: true }).fill(password);
  const received = page.waitForResponse(r => new URL(r.url()).pathname === '/v1/auth/login' && r.request().method() === 'POST');
  await page.getByRole('button', { name: 'Log in', exact: true }).click();
  const response = await received;
  assert.equal(response.status(), 200);
  const session = await response.json(); // bearer kept in memory only
  await page.locator('[title="gateway: connected"]').first().waitFor();
  return session;
}
async function selectWorkspace(page, workspace, role) {
  await page.locator(`button[title="${workspace.name} (${role})"]`).click();
  const toggle = page.getByRole('button', { name: 'Your activity', exact: true });
  if (await toggle.getAttribute('aria-expanded') !== 'true') await toggle.click();
  return page.getByRole('region', { name: 'Your workspace activity', exact: true });
}
async function total(region, count) {
  await region.getByText(`${count} messages across current channels`, { exact: true }).waitFor();
}
try {
  await request('GET', '/ready');
  phase = 'synthetic API setup';
  const suffix = randomUUID().replaceAll('-', '').slice(0, 12);
  const password = `Synthetic-only-${suffix}-8!`;
  const accounts = [];
  for (const label of ['alice', 'bob']) {
    const handle = `${label}_${suffix}`;
    await request('POST', '/v1/auth/register', null, { handle, email: `${handle}@example.invalid`, display_name: handle, password }, 201);
    accounts.push({ handle, ...await request('POST', '/v1/auth/login', null, { handle, password }) });
  }
  const [alice, bob] = accounts;
  const workspace = await request('POST', '/v1/workspaces', alice.token, { name: `Shared-${suffix}` }, 201);
  const other = await request('POST', '/v1/workspaces', alice.token, { name: `Separate-${suffix}` }, 201);
  await request('POST', `/v1/workspaces/${workspace.id}/members`, alice.token, { user_handle: bob.handle, role: 'member' }, 201);
  const channels = [];
  for (const name of ['alpha-sentinel', 'zero-sentinel']) channels.push(await request('POST', `/v1/workspaces/${workspace.id}/channels`, alice.token, { name }, 201));
  const separate = await request('POST', `/v1/workspaces/${other.id}/channels`, alice.token, { name: 'separate-sentinel' }, 201);
  async function send(token, channel) {
    return request('POST', '/v1/messages', token, { conversation_id: channel.conversation_id, client_msg_id: randomUUID(), ciphertext_b64: Buffer.from('Synthetic acceptance only').toString('base64') }, 201);
  }
  await send(alice.token, channels[0]); await send(bob.token, channels[0]);
  await send(alice.token, separate); await send(alice.token, separate); await send(alice.token, separate);
  const statsRoute = id => `/v1/workspaces/${id}/stats/me`;
  await until(async () => (await request('GET', statsRoute(workspace.id), alice.token)).message_count === 1, 'outbox projection');
  await until(async () => (await request('GET', statsRoute(other.id), alice.token)).message_count === 3, 'other projection');
  await until(async () => (await request('GET', statsRoute(workspace.id), bob.token)).message_count === 1, 'bob projection');
  await request('GET', statsRoute(other.id), bob.token, undefined, 403);
  await request('DELETE', `/v1/auth/sessions/${alice.session_id}`, bob.token, undefined, 404);
  check('real synthetic accounts, messages, outbox counts and cross-account denial');

  phase = 'browser login and message';
  const logger = createLogger();
  // Proxy diagnostics may contain gateway query bearers. Preserve a count and
  // a fixed diagnostic instead of emitting raw messages to hosted logs.
  for (const level of ['info', 'warn', 'warnOnce', 'error']) logger[level] = () => {
    report.viteDiagnostics = (report.viteDiagnostics ?? 0) + 1;
    console.log(`Vite ${level}: details excluded from credential-safe evidence`);
  };
  vite = await createServer({
    root: desktop, envFile: false, customLogger: logger, plugins: [barrierPlugin],
    define: { 'import.meta.env.VITE_API_URL': 'window.location.origin' },
    server: { host: '127.0.0.1', port: 0, strictPort: false,
      proxy: { '/v1': { target: backend, ws: true } },
      watch: { ignored: ['**/.acceptance/**', '**/evidence/**', '**/target/**'] } },
  });
  await vite.listen();
  browser = await chromium.launch({ headless: true });
  report.browser = browser.version();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
  await page.goto(vite.resolvedUrls.local[0]);
  const browserAlice = await login(page, alice.handle, password);
  await selectWorkspace(page, workspace, 'owner');
  await page.locator('button[title="#alpha-sentinel"]').click();
  await page.getByPlaceholder('Message (demo plaintext → opaque envelope)').fill('Synthetic browser message');
  const sent = page.waitForResponse(r => new URL(r.url()).pathname === '/v1/messages' && r.request().method() === 'POST');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  assert.equal((await sent).status(), 201);
  await until(async () => (await request('GET', statsRoute(workspace.id), alice.token)).message_count === 2, 'browser message projection');
  let activity = page.getByRole('region', { name: 'Your workspace activity', exact: true });
  await activity.getByRole('button', { name: 'Refresh activity' }).click();
  await total(activity, 2);
  await activity.locator('li').filter({ hasText: '#alpha-sentinel' }).getByText('2 messages', { exact: true }).waitFor();
  await activity.locator('li').filter({ hasText: '#zero-sentinel' }).getByText('0 messages', { exact: true }).waitFor();
  check('browser send persists and caller-private UI reflects real projection');

  phase = 'real pagination';
  const first = await request('GET', `${statsRoute(workspace.id)}/channels?limit=1`, alice.token);
  assert.equal(first.channels.length, 1); assert.ok(first.next_cursor);
  const second = await request('GET', `${statsRoute(workspace.id)}/channels?limit=1&after=${first.next_cursor}`, alice.token);
  assert.equal(second.channels.length, 1); assert.equal(second.next_cursor, null);
  assert.notEqual(first.channels[0].channel_id, second.channels[0].channel_id);
  const sessions1 = await request('GET', '/v1/auth/sessions?limit=1', browserAlice.token);
  assert.equal(sessions1.sessions.length, 1); assert.ok(sessions1.next_cursor);
  const sessions2 = await request('GET', `/v1/auth/sessions?limit=1&after=${sessions1.next_cursor}`, browserAlice.token);
  assert.equal(sessions2.sessions.length, 1); assert.equal(sessions2.next_cursor, null);
  const sessionRows = [...sessions1.sessions, ...sessions2.sessions];
  assert.equal(new Set(sessionRows.map(row => row.id)).size, 2);
  assert.equal(sessionRows.filter(row => row.is_current).length, 1);
  assert.equal(sessionRows.find(row => row.is_current).id, browserAlice.session_id);
  for (const row of sessionRows) assert.deepEqual(Object.keys(row).sort(), ['created_at', 'device_id', 'expires_at', 'id', 'is_current']);
  check('real API keyset pagination and safe session inventory fields');

  phase = 'stale workspace response';
  const staleWorkspace = hold('GET', statsRoute(workspace.id));
  await activity.getByRole('button', { name: 'Refresh activity' }).click();
  await until(() => staleWorkspace.reached, 'workspace response held');
  assert.equal(staleWorkspace.status, 200);
  activity = await selectWorkspace(page, other, 'owner');
  assert.equal(await activity.getByText('#alpha-sentinel', { exact: true }).count(), 0);
  await total(activity, 3);
  await release(staleWorkspace, page);
  await total(activity, 3);
  assert.equal(await activity.getByText('#alpha-sentinel', { exact: true }).count(), 0);
  check('late real workspace response cannot replace destination activity');

  phase = 'other-session revoke';
  await page.getByRole('button', { name: 'Sessions', exact: true }).click();
  const self = page.locator(`li[data-session-id="${browserAlice.session_id}"]`);
  await self.getByText('Current session', { exact: true }).waitFor();
  await self.getByText('No device linked', { exact: true }).waitFor();
  const otherSession = page.locator(`li[data-session-id="${alice.session_id}"]`);
  await otherSession.getByText('Other session', { exact: true }).waitFor();
  await otherSession.getByRole('button', { name: 'End session', exact: true }).click();
  await otherSession.waitFor({ state: 'detached' });
  await request('GET', '/v1/auth/sessions', alice.token, undefined, 401);
  await request('GET', '/v1/auth/sessions', browserAlice.token);
  check('UI revokes another bearer while current bearer remains authorized');

  phase = 'self revoke while controls close';
  const selfRevoke = hold('DELETE', `/v1/auth/sessions/${browserAlice.session_id}`);
  await self.getByRole('button', { name: 'End this session and log out' }).click();
  await until(() => selfRevoke.reached, 'real self DELETE held');
  assert.equal(selfRevoke.status, 204);
  await page.getByRole('button', { name: 'Close sessions', exact: true }).click();
  assert.equal(selfRevoke.delivered, false);
  assert.equal(await page.getByRole('region', { name: 'Account sessions', exact: true }).isVisible(), false);
  await release(selfRevoke, page);
  await page.getByRole('button', { name: 'Log in', exact: true }).waitFor();
  await request('GET', '/v1/auth/sessions', browserAlice.token, undefined, 401);
  check('closing controls during real self DELETE still logs out and denies bearer');

  phase = 'account replacement and late response';
  await login(page, alice.handle, password);
  activity = await selectWorkspace(page, workspace, 'owner');
  await total(activity, 2);
  const staleAccount = hold('GET', statsRoute(workspace.id));
  await activity.getByRole('button', { name: 'Refresh activity' }).click();
  await until(() => staleAccount.reached, 'account response held');
  assert.equal(staleAccount.status, 200);
  await page.getByRole('button', { name: 'Log out', exact: true }).click();
  const browserBob = await login(page, bob.handle, password);
  activity = await selectWorkspace(page, workspace, 'member');
  assert.equal(await activity.getByText('2 messages across current channels', { exact: true }).count(), 0);
  await total(activity, 1);
  await release(staleAccount, page);
  await total(activity, 1);
  assert.equal(await activity.getByText('2 messages across current channels', { exact: true }).count(), 0);
  assert.equal(await page.locator(`button[title="${other.name} (owner)"]`).count(), 0);
  await page.getByRole('button', { name: 'Sessions', exact: true }).click();
  await page.locator(`li[data-session-id="${browserBob.session_id}"]`).waitFor();
  assert.equal(await page.locator(`li[data-session-id="${browserAlice.session_id}"]`).count(), 0);
  check('account replacement rejects late real activity and old session inventory');
  report.result = 'PASS';
} catch {
  // Allowlisted phase only: Playwright errors can include tokens in URLs or DOM.
  report.failurePhase = phase;
  process.exitCode = 1;
  console.error(`FAIL ${phase}; raw credentials, DOM and network traces intentionally excluded`);
} finally {
  gate?.release();
  try { await browser?.close(); await vite?.close(); report.cleanup = 'PASS'; }
  catch { report.cleanup = 'FAIL'; report.result = 'FAIL'; process.exitCode = 1; }
  writeFileSync(path.join(artifact, 'result.json'), JSON.stringify(report, null, 2) + '\n');
}
