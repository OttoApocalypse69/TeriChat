import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';
import { createServer } from 'vite';
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
// UI-only acceptance. Synthetic HTTP fixtures are not backend evidence.
const desktop = fileURLToPath(new URL('../', import.meta.url));
process.chdir(desktop);
const artifact = path.join(desktop, 'evidence', 's3-controls');
mkdirSync(artifact, { recursive: true });
const vite = await createServer({ root: desktop, envFile: false, server: { host: '127.0.0.1', port: 0, strictPort: false }, define: { 'import.meta.env.VITE_API_URL': JSON.stringify('http://synthetic.test') } });
let browser;
try {
  await vite.listen(); browser = await chromium.launch({ channel: 'msedge', headless: true });
  const page = await browser.newPage();
  await page.route('http://synthetic.test/**', async route => {
    const p = new URL(route.request().url()).pathname; let body = [];
    if (p === '/v1/auth/login') body = { token: 'synthetic-token', user_id: 'me', user_handle: 'synthetic', session_id: 'self' };
    if (p === '/v1/conversations') body = [{ id: 'dm', kind: 'dm', members: ['me', 'peer'], peer_handle: 'peer', peer_display_name: 'Synthetic Peer' }];
    if (p === '/v1/workspaces') body = [{ id: 'ws', name: 'Synthetic Workspace', owner_id: 'me', my_role: 'member' }];
    if (p.endsWith('/members')) body = { members: [], next_cursor: null };
    if (p === '/v1/auth/sessions') body = { sessions: [{ id: 'self', device_id: null, is_current: true, created_at: '2026-09-01T00:00:00Z', expires_at: '2026-10-01T00:00:00Z' }], next_cursor: null };
    if (p.endsWith('/stats/me')) body = { workspace_id: 'ws', user_id: 'me', message_count: 43, last_message_at: null };
    if (p.endsWith('/stats/me/channels')) body = { channels: [{ channel_id: 'general', conversation_id: 'channel', name: 'general', message_count: 43, last_message_at: null }], next_cursor: null };
    if (p === '/v1/auth/sessions/self' && route.request().method() === 'DELETE') { await route.fulfill({ status: 204 }); return; }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });
  await page.goto(vite.resolvedUrls.local[0]);
  await page.getByPlaceholder('handle', { exact: true }).fill('synthetic');
  await page.getByPlaceholder('password', { exact: true }).fill('synthetic-password');
  await page.getByRole('button', { name: 'Log in', exact: true }).click();
  await page.getByRole('button', { name: /Synthetic Peer/ }).click();
  const draft = page.getByPlaceholder('Message (demo plaintext → opaque envelope)');
  await draft.fill('Synthetic unsent draft'); const measurements = [];
  for (const width of [1440, 768, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await page.getByRole('button', { name: 'Sessions', exact: true }).click();
    await page.getByRole('heading', { name: 'Account sessions' }).waitFor();
    const size = await page.evaluate(() => ({ width: innerWidth, scrollWidth: document.documentElement.scrollWidth }));
    assert.ok(size.scrollWidth <= size.width, `sessions fit ${width}`); measurements.push(size);
    await page.screenshot({ path: path.join(artifact, `sessions-${width}.png`) });
    await page.getByRole('button', { name: 'Close sessions' }).click();
    assert.equal(await draft.inputValue(), 'Synthetic unsent draft');
    assert.equal(await page.evaluate(() => document.activeElement.textContent), 'Sessions');
  }
  await page.getByRole('button', { name: 'Workspace details', exact: true }).last().click();
  await page.getByRole('button', { name: 'Your activity', exact: true }).click();
  await page.getByText('43 messages across current channels').waitFor();
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  await page.screenshot({ path: path.join(artifact, 'activity-320.png') });
  await page.getByRole('button', { name: 'Sessions', exact: true }).click();
  await page.getByRole('button', { name: 'End this session and log out' }).click();
  await page.getByRole('button', { name: 'Log in', exact: true }).waitFor();
  writeFileSync(path.join(artifact, 'result.json'), JSON.stringify({ result: 'PASS', scope: 'UI-only synthetic HTTP fixtures; no real backend', measurements, draftAndFocus: true, selfRevoke: true }, null, 2));
  console.log('PASS: responsive session/activity controls, draft/focus and self revoke (HTTP fixtures only)');
} finally { await browser?.close(); await vite.close(); }
