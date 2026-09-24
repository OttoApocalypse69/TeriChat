import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';
import { createServer } from 'vite';
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

// Presentation/interaction evidence only. All account data is synthetic.
const desktop = fileURLToPath(new URL('../', import.meta.url));
process.chdir(desktop);
const artifact = path.join(desktop, '.acceptance', 'stitch-ui', new Date().toISOString().replaceAll(':', '-'));
mkdirSync(artifact, { recursive: true });
const vite = await createServer({ root: desktop, envFile: false, server: { host: '127.0.0.1', port: 0, strictPort: false }, define: { 'import.meta.env.VITE_API_URL': JSON.stringify('http://synthetic.test') } });
const at = '2026-09-24T14:05:00Z';
const message = (id, sender, text, seq, conversation = 'dm') => ({ id, sender_id: sender, ciphertext_b64: Buffer.from(text).toString('base64'), seq, conversation_id: conversation, sent_at: at, client_msg_id: `client-${id}` });
const histories = {
  dm: [message('m1', 'peer', 'The new workspace is ready. What do you think of the violet accents?', 1), message('m2', 'me', 'This feels like our space now. Let’s keep the conversation going.', 2), message('m3', 'peer', 'Agreed. Clear navigation, a little breathing room, and room for the people who matter.', 3)],
  channel: [message('c1', 'peer', 'Welcome to general. This is where the team gets together.', 1, 'channel')],
};
let browser;
const measurements = [];
try {
  await vite.listen();
  browser = await chromium.launch(process.platform === 'win32' ? { channel: 'msedge', headless: true } : { headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.route('http://synthetic.test/**', async route => {
    const request = route.request();
    const url = new URL(request.url());
    const p = url.pathname;
    let body = [];
    if (p === '/v1/auth/login') body = { token: 'synthetic-token', user_id: 'me', user_handle: 'teriri', session_id: 'self' };
    if (p === '/v1/conversations') body = [{ id: 'dm', kind: 'dm', members: ['me', 'peer'], peer_handle: 'alex', peer_display_name: 'Alex Rivera', last_seq: 3, last_sent_at: at }];
    if (p === '/v1/workspaces') body = [{ id: 'ws', name: 'Design Collective', owner_id: 'me', my_role: 'member' }];
    if (p === '/v1/workspaces/ws/channels') body = [{ id: 'general', workspace_id: 'ws', conversation_id: 'channel', name: 'general', kind: 'text', created_by: 'me', created_at: at }];
    if (p.endsWith('/members')) body = { members: [], next_cursor: null };
    if (p === '/v1/messages' && request.method() === 'GET') body = (histories[url.searchParams.get('conversation_id')] ?? []).filter(row => row.seq > Number(url.searchParams.get('since_seq') ?? 0));
    if (p === '/v1/messages' && request.method() === 'POST') {
      const input = request.postDataJSON();
      const rows = histories[input.conversation_id];
      body = { ...message(`sent-${rows.length}`, 'me', '', rows.length + 1, input.conversation_id), ciphertext_b64: input.ciphertext_b64, client_msg_id: input.client_msg_id };
      rows.push(body);
    }
    if (p === '/v1/auth/sessions') body = { sessions: [
      { id: 'self', device_id: null, is_current: true, created_at: at, expires_at: '2026-10-24T14:05:00Z' },
      { id: 'synthetic-other-session', device_id: 'synthetic-device-41', is_current: false, created_at: at, expires_at: '2026-10-20T14:05:00Z' },
    ], next_cursor: null };
    if (p.endsWith('/stats/me')) body = { workspace_id: 'ws', user_id: 'me', message_count: 43, last_message_at: at };
    if (p.endsWith('/stats/me/channels')) body = { channels: [{ channel_id: 'general', conversation_id: 'channel', name: 'general', message_count: 43, last_message_at: at }], next_cursor: null };
    if (p === '/v1/auth/sessions/self' && request.method() === 'DELETE') { await route.fulfill({ status: 204 }); return; }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });
  const snapshot = async name => {
    const size = await page.evaluate(() => ({ width: innerWidth, scrollWidth: document.documentElement.scrollWidth }));
    assert.ok(size.scrollWidth <= size.width, `${name}: no horizontal overflow`);
    measurements.push({ name, ...size });
    await page.screenshot({ path: path.join(artifact, `${name}.png`) });
  };
  await page.goto(vite.resolvedUrls.local[0]);
  await snapshot('login-desktop');
  await page.setViewportSize({ width: 320, height: 900 });
  await snapshot('login-320');
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByLabel('Handle', { exact: true }).fill('teriri');
  await page.getByLabel('Password', { exact: true }).fill('synthetic-password');
  await page.getByRole('button', { name: 'Log in', exact: true }).click();
  await page.getByRole('button', { name: /Alex Rivera/ }).click();
  await page.getByText('The new workspace is ready.', { exact: false }).waitFor();
  const draft = page.getByRole('textbox', { name: 'Message', exact: true });
  assert.equal(await page.locator('#workspace-details').isVisible(), false, 'details initially closed');
  await snapshot('dm-desktop');
  await page.getByRole('searchbox', { name: 'Filter conversations and channels' }).fill('no-such-name');
  await page.getByText('No matching conversations.').waitFor();
  assert.equal(await page.getByRole('button', { name: /Alex Rivera/ }).count(), 0);
  await page.getByRole('searchbox').fill('alex');
  assert.equal(await page.getByRole('button', { name: /Alex Rivera/ }).count(), 1);
  await page.getByRole('searchbox').fill('');
  await draft.fill('A message sent through the real composer, with synthetic HTTP.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await page.getByText('A message sent through the real composer, with synthetic HTTP.', { exact: true }).waitFor();
  assert.equal(await draft.inputValue(), '');
  await draft.fill('Keep this draft');
  for (const width of [1440, 768, 390, 320]) {
    await page.setViewportSize({ width, height: 1000 });
    await snapshot(`chat-${width}`);
    assert.equal(await page.locator('.plaintext-warning').isVisible(), true);
    await page.getByRole('button', { name: 'Workspace details', exact: true }).last().click();
    assert.equal(await page.locator('#workspace-details').isVisible(), true);
    const activity = page.getByRole('button', { name: 'Your activity', exact: true });
    if (await activity.getAttribute('aria-expanded') !== 'true') await activity.click();
    await page.getByText('43 messages across current channels').waitFor();
    await snapshot(`details-${width}`);
    await page.locator('#workspace-details').getByRole('button', { name: '← Back', exact: true }).click();
    assert.equal(await draft.isVisible(), true);
    assert.equal(await draft.inputValue(), 'Keep this draft');
    await page.getByRole('button', { name: 'Sessions', exact: true }).click();
    await page.getByRole('heading', { name: 'Account sessions', exact: true }).waitFor();
    await snapshot(`sessions-${width}`);
    await page.getByRole('button', { name: 'Close sessions' }).click();
    assert.equal(await draft.inputValue(), 'Keep this draft');
    assert.equal(await page.evaluate(() => document.activeElement.textContent), 'Sessions');
    if (width < 768) {
      await page.getByRole('button', { name: '← Back to conversations', exact: true }).click();
      await snapshot(`navigation-${width}`);
      await page.getByRole('searchbox').fill('unmatched');
      await page.getByRole('button', { name: 'Return to conversation' }).click();
      await page.getByRole('button', { name: '← Back to conversations', exact: true }).click();
      assert.equal(await page.evaluate(() => document.activeElement.getBoundingClientRect().width > 0), true, 'focus visible with filtered selection');
      await page.getByRole('searchbox').fill('');
      await page.getByRole('button', { name: 'Return to conversation' }).click();
      assert.equal(await draft.inputValue(), 'Keep this draft');
    }
  }
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole('button', { name: '#general', exact: true }).click();
  await page.getByText('Welcome to general. This is where the team gets together.', { exact: true }).waitFor();
  assert.equal(await draft.inputValue(), '', 'new conversation resets draft');
  await snapshot('channel-desktop');
  // 1440x900 at 200% browser zoom has a 720x450 CSS viewport.
  await page.setViewportSize({ width: 720, height: 450 });
  await snapshot('zoom-equivalent-200-percent');
  await page.getByRole('button', { name: 'Sessions', exact: true }).click();
  await page.getByRole('button', { name: 'End this session and log out' }).click();
  await page.getByRole('button', { name: 'Log in', exact: true }).waitFor();
  assert.deepEqual(pageErrors, [], 'no uncaught browser errors');
  writeFileSync(path.join(artifact, 'result.json'), JSON.stringify({ result: 'PASS', scope: 'UI-only; synthetic HTTP fixtures, not backend evidence', measurements, checks: ['filter', 'send', 'details controls at all sizes', 'draft retention', 'visible focus', 'session close and self revoke', 'conversation draft reset', '200% equivalent CSS reflow'], pageErrors }, null, 2));
  console.log(`PASS: Stitch UI interaction and responsive acceptance. Artifacts: ${artifact}`);
} catch (error) {
  writeFileSync(path.join(artifact, 'failure.txt'), String(error.stack ?? error));
  console.error(`Evidence retained: ${artifact}`);
  throw error;
} finally { await browser?.close(); await vite.close(); }
