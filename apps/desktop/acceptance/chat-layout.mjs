import assert from 'node:assert/strict';
import path from 'node:path';
import { writeFileSync } from 'node:fs';

// Run against the mounted app and real synthetic backend after the full campaign.
export async function verifyChatLayout(page, artifact, send, native, incoming) {
  const draft = page.getByPlaceholder('Message (demo plaintext → opaque envelope)');
  const navigation = page.getByRole('complementary', { name: 'Conversations and workspaces' });
  const main = page.getByRole('main', { name: 'Conversation' });
  const long = 'Synthetic wrapping check: ' + 'unbroken'.repeat(64);
  const measurements = [];
  const hiddenIncoming = [];
  const mountedDraft = await draft.elementHandle();
  async function receiveWhileHidden(label) {
    const history = page.locator('.message-history');
    assert.equal(await history.evaluate(el => el.clientHeight), 0, `${label}: history has no layout box`);
    for (let i = 0; i < 4; i++) {
      const text = `Synthetic hidden ${label} ${i}`;
      await incoming(text);
      await page.getByText(text, { exact: true }).waitFor({ state: 'attached' });
    }
    // Flush the receiving renderer's effects before reopening; no subsequent send.
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  }
  async function returnedToTail(label, navigationReturn = true) {
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const size = await page.locator('.message-history').evaluate(el => {
      const rect = el.getBoundingClientRect();
      const tail = [...el.querySelectorAll('.message-bubble')].at(-1).getBoundingClientRect();
      return { height: el.clientHeight, scrollHeight: el.scrollHeight, scrollTop: el.scrollTop,
        gap: el.scrollHeight - el.clientHeight - el.scrollTop, tailTop: tail.top, top: rect.top, tailBottom: tail.bottom, bottom: rect.bottom };
    });
    hiddenIncoming.push({ label, ...size });
    writeFileSync(path.join(artifact, 'hidden-incoming.json'), JSON.stringify(hiddenIncoming, null, 2));
    await page.screenshot({ path: path.join(artifact, `hidden-incoming-${label}.png`) });
    assert.ok(size.scrollHeight > size.height, `${label}: actual scrollable history`);
    assert.ok(size.gap <= 1 && size.tailTop >= size.top && size.tailBottom <= size.bottom, `${label}: incoming tail visible after return without another message (gap ${size.gap})`);
    assert.ok(await draft.evaluate((el, original) => el === original, mountedDraft), `${label}: composer not remounted`);
    assert.equal(await draft.inputValue(), 'Synthetic unsent draft');
    if (navigationReturn) assert.ok(await main.evaluate(el => el === document.activeElement), `${label}: navigation focus remains on main`);
    await draft.focus();
    await draft.press('End');
    await draft.press('!');
    assert.equal(await draft.inputValue(), 'Synthetic unsent draft!', `${label}: draft focus usable`);
    await draft.press('Backspace');
  }
  await send(page, long);
  async function fits(label) {
    const sizes = await page.evaluate(() => ({ width: innerWidth, scroll: document.documentElement.scrollWidth,
      composer: document.querySelector('main input').getBoundingClientRect().toJSON(),
      send: document.querySelector('main button[type=submit]').getBoundingClientRect().toJSON(),
      height: innerHeight,
      panes: [...document.querySelectorAll('.chat-navigation, .chat-main, .workspace-details')].map(el => ({
        name: el.getAttribute('aria-label'), display: getComputedStyle(el).display,
        rect: el.getBoundingClientRect().toJSON(),
      })),
      messages: [...document.querySelectorAll('main p.whitespace-pre-wrap')].every(p => p.scrollWidth <= p.clientWidth + 1),
    }));
    measurements.push({ label, ...sizes });
    writeFileSync(path.join(artifact, 'layout-measurements.json'), JSON.stringify(measurements, null, 2));
    assert.ok(sizes.scroll <= sizes.width, `${label}: no horizontal page overflow`);
    assert.ok(sizes.messages, `${label}: long messages wrap`);
    assert.ok(sizes.composer.width > 100 && sizes.composer.x >= 0, `${label}: usable composer`);
    assert.ok(sizes.send.right <= sizes.width && sizes.send.bottom <= sizes.height, `${label}: send in viewport`);
    return sizes;
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  await draft.fill('Synthetic unsent draft');
  const desktop = await fits('desktop');
  assert.ok(await navigation.isVisible());
  await page.screenshot({ path: path.join(artifact, 'chat-desktop.png') });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: path.join(artifact, 'chat-phone.png') });
  assert.ok(!await navigation.isVisible(), 'phone shows one pane');
  assert.ok(await main.isVisible());
  const phone = await fits('phone');
  await page.getByRole('button', { name: 'Back to conversations' }).click();
  assert.ok(await navigation.isVisible());
  assert.ok(!await main.isVisible(), 'hidden composer cannot receive keyboard focus');
  await page.screenshot({ path: path.join(artifact, 'chat-phone-navigation.png') });
  await receiveWhileHidden('navigation');
  await navigation.locator('button[aria-current="page"]').click();
  assert.ok(await main.evaluate(el => el === document.activeElement), 'focus follows conversation navigation');
  await returnedToTail('navigation');
  assert.equal(await draft.inputValue(), 'Synthetic unsent draft');
  await main.getByRole('button', { name: 'Workspace details', exact: true }).click();
  const details = page.getByRole('complementary', { name: 'Workspace details' });
  assert.ok(await details.isVisible());
  assert.ok(!await main.isVisible() && !await navigation.isVisible(), 'phone details is the only pane');
  await page.screenshot({ path: path.join(artifact, 'chat-phone-details.png') });
  await receiveWhileHidden('details');
  await details.getByRole('button', { name: '← Back', exact: true }).click();
  await returnedToTail('details');
  assert.equal(await draft.inputValue(), 'Synthetic unsent draft');
  await page.getByRole('button', { name: 'Back to conversations' }).click();
  const mountedHistory = await page.locator('.message-history').elementHandle();
  await receiveWhileHidden('breakpoint');
  const receivedCount = await page.locator('.message-bubble').count();
  // CSS alone reveals the still-inactive pane; do not click navigation or send.
  await page.setViewportSize({ width: 1440, height: 900 });
  await returnedToTail('breakpoint', false);
  assert.equal(await page.locator('.message-bubble').count(), receivedCount, 'breakpoint: no extra message');
  assert.ok(await page.locator('.message-history').evaluate((el, original) => el === original, mountedHistory), 'breakpoint: history DOM retained');
  // A consumed follow must not pull a reader back to the tail on later resizes.
  await page.locator('.message-history').evaluate(el => { el.scrollTop = 0; });
  await page.setViewportSize({ width: 1300, height: 850 });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  assert.equal(await page.locator('.message-history').evaluate(el => el.scrollTop), 0, 'resize preserves reading old messages');
  await navigation.locator('button[aria-current="page"]').click();
  await page.setViewportSize({ width: 320, height: 568 });
  const narrow = await fits('320px phone');
  await page.screenshot({ path: path.join(artifact, 'chat-narrow.png') });
  await page.setViewportSize({ width: 390, height: 400 });
  await fits('short viewport');
  await page.setViewportSize({ width: 768, height: 700 });
  await fits('tablet');
  await page.setViewportSize({ width: 1440, height: 900 });
  assert.equal(await draft.inputValue(), 'Synthetic unsent draft', 'resize retains mounted draft');
  await draft.fill('');
  return { desktop, phone, narrow, hiddenIncoming, viewportMode: native ? 'WebView2 CDP emulation (not physical phone)' : 'browser viewport', screenshots: ['chat-desktop.png', 'chat-phone.png', 'chat-phone-navigation.png', 'chat-phone-details.png', 'chat-narrow.png'] };
}
