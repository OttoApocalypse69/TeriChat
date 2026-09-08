// Run only against a disposable local backend: node acceptance/workspaces.mjs http://127.0.0.1:<port>
// Real HTTP/DB and shared React UI; browser mode uses the existing test-only HTTP adapter.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import { createServer } from 'vite';
import react from '@vitejs/plugin-react';

const desktop = fileURLToPath(new URL('../', import.meta.url));
const backend = process.argv[2];
assert.equal(process.argv.length, 3, 'supply one disposable backend URL');
assert.match(backend ?? '', /^http:\/\/127\.0\.0\.1:\d+$/, 'local disposable backend required');
const runId = new Date().toISOString().replace(/[:.]/g, '-');
const artifact = path.join(desktop, '.acceptance', `workspace-${runId}`);
mkdirSync(artifact, { recursive: true });
const report = {
  head: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: desktop, encoding: 'utf8' }).trim(),
  scope: 'workspace-browser-http-adapter', result: 'FAIL', steps: [],
};
const suffix = Date.now().toString(36);
const password = 'Synthetic-workspace-only-8!';
const names = { owner: `owner_${suffix}`, member: `member_${suffix}`, outsider: `outside_${suffix}` };
const workspaceName = `Swarm workspace ${suffix}`;
let vite;
let browser;
const check = name => { report.steps.push(name); console.log(`PASS ${name}`); };
async function request(method, route, token, body, expected = 200) {
  const response = await fetch(backend + route, {
    method, signal: AbortSignal.timeout(10000),
    headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  assert.equal(response.status, expected, `${method} ${route}`);
  const text = await response.text();
  return text ? JSON.parse(text) : null;
}
async function login(page, handle) {
  await page.getByPlaceholder('handle', { exact: true }).fill(handle);
  await page.getByPlaceholder('password', { exact: true }).fill(password);
  await page.getByRole('button', { name: 'Log in', exact: true }).click();
  await page.locator('[title="gateway: connected"]').first().waitFor();
}

try {
  await request('GET', '/ready');
  const accounts = {};
  for (const [key, handle] of Object.entries(names)) {
    await request('POST', '/v1/auth/register', null, {
      handle, email: `${handle}@example.invalid`, display_name: handle, password,
    }, 201);
    accounts[key] = await request('POST', '/v1/auth/login', null, { handle, password });
  }
  vite = await createServer({
    configFile: false, root: desktop, envDir: false, plugins: [react()],
    resolve: { alias: { '@tauri-apps/plugin-http': path.join(desktop, 'acceptance/browser-http.mjs') } },
    define: { 'import.meta.env.VITE_API_URL': JSON.stringify(backend) },
    optimizeDeps: { entries: ['index.html'] },
    server: { host: '127.0.0.1', port: 0, watch: { ignored: ['**/.acceptance/**', '**/evidence/**', '**/target/**'] } },
  });
  await vite.listen();
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  const a = await (await browser.newContext()).newPage();
  const b = await (await browser.newContext()).newPage();
  for (const page of [a, b]) {
    page.setDefaultTimeout(15000);
    await page.route(`${backend}/**`, async route => {
      if (route.request().resourceType() === 'websocket') return route.continue();
      const response = await route.fetch();
      await route.fulfill({ response, headers: { ...response.headers(), 'access-control-allow-origin': '*' } });
    });
    await page.goto(vite.resolvedUrls.local[0]);
  }
  await login(a, names.owner);
  await a.getByLabel('Workspace name', { exact: true }).fill(workspaceName);
  await a.getByRole('form', { name: 'Create workspace', exact: true }).getByRole('button', { name: 'Create', exact: true }).click();
  await a.getByText('Members · you are owner', { exact: true }).waitFor();
  const workspaces = await request('GET', '/v1/workspaces', accounts.owner.token);
  const workspace = workspaces.find(row => row.name === workspaceName);
  assert.ok(workspace, 'UI-created workspace is persisted');
  assert.equal(workspace.my_role, 'owner');
  check('create workspace in UI persists and selects owner workspace');

  await a.getByPlaceholder('handle → add').fill(names.member);
  const addForm = a.getByPlaceholder('handle → add').locator('..');
  await addForm.getByRole('button', { name: 'Add', exact: true }).click();
  await a.locator(`li[data-member-id="${accounts.member.user_id}"]`).waitFor();
  await login(b, names.member);
  await b.getByText('Members · you are member', { exact: true }).waitFor();
  await b.locator(`li[data-member-id="${accounts.owner.user_id}"]`).waitFor();
  await request('GET', `/v1/workspaces/${workspace.id}/audit`, accounts.member.token, undefined, 403);
  check('ordinary member sees authoritative roster without audit permission');

  const first = await request('GET', `/v1/workspaces/${workspace.id}/members?limit=1`, accounts.member.token);
  assert.equal(first.members.length, 1);
  assert.ok(first.next_cursor);
  const second = await request('GET', `/v1/workspaces/${workspace.id}/members?limit=1&after=${first.next_cursor}`, accounts.member.token);
  assert.equal(second.members.length, 1);
  assert.equal(second.next_cursor, null);
  assert.notEqual(first.members[0].user_id, second.members[0].user_id);
  for (const row of [...first.members, ...second.members]) {
    assert.deepEqual(Object.keys(row).sort(), ['display_name', 'handle', 'joined_at', 'role', 'user_id']);
  }
  await request('GET', `/v1/workspaces/${workspace.id}/members`, accounts.outsider.token, undefined, 403);
  check('member pagination has no duplicates or private fields; outsider denied');

  await a.getByPlaceholder('new channel name').fill('general');
  await a.getByPlaceholder('new channel name').locator('..').getByRole('button', { name: 'Add', exact: true }).click();
  await a.getByPlaceholder('Message (demo plaintext → opaque envelope)').fill('bro');
  await a.getByRole('button', { name: 'Send', exact: true }).click();
  await b.reload();
  await login(b, names.member);
  await b.locator('button[title="#general"]').click();
  await b.locator('main p.whitespace-pre-wrap').filter({ hasText: /^bro$/ }).waitFor();
  await b.getByText('Alpha demo: envelopes carry demo plaintext — not end-to-end encrypted.', { exact: true }).waitFor();
  check('created channel and message survive client reload; security state stays honest');
  await a.screenshot({ path: path.join(artifact, 'workspace.png'), fullPage: true });

  const memberRow = a.locator(`li[data-member-id="${accounts.member.user_id}"]`);
  await memberRow.getByRole('combobox').selectOption('moderator');
  await memberRow.getByRole('button', { name: 'Set role', exact: true }).click();
  await memberRow.locator('span').filter({ hasText: /^moderator$/ }).waitFor();
  const updated = await request('GET', `/v1/workspaces/${workspace.id}/members`, accounts.owner.token);
  assert.equal(updated.members.find(row => row.user_id === accounts.member.user_id).role, 'moderator');
  await memberRow.getByRole('button', { name: 'Kick', exact: true }).click();
  await memberRow.waitFor({ state: 'detached' });
  await request('GET', `/v1/workspaces/${workspace.id}/members`, accounts.member.token, undefined, 403);
  check('role update and kick refresh roster; removed member access revoked');
  report.result = 'PASS';
} catch (error) {
  report.error = error.message;
  process.exitCode = 1;
  console.error(error.message);
} finally {
  try { await browser?.close(); await vite?.close(); report.cleanup = 'PASS'; }
  catch (error) { report.cleanup = 'FAIL'; report.result = 'FAIL'; process.exitCode = 1; console.error(error.message); }
  writeFileSync(path.join(artifact, 'result.json'), JSON.stringify(report, null, 2) + '\n');
  console.log(`Evidence: ${path.join(artifact, 'result.json')}`);
}
