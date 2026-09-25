// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ChatConversation, ChatMessage } from '../lib/store';
import ConversationView from './ConversationView';

let host: HTMLDivElement;
let root: Root;

const dm: ChatConversation = {
  id: 'dm', kind: 'dm', members: ['me', 'peer'], peer_handle: 'peer', peer_display_name: 'Synthetic Peer',
};
const message = (seq: number, sender: string, sentAt: string): ChatMessage => ({
  id: `m${seq}`, conversation_id: 'dm', sender_id: sender, seq, sent_at: sentAt,
  client_msg_id: `c${seq}`, ciphertext_b64: btoa(`body-${seq}`),
});

async function render(messages: ChatMessage[], conversation: ChatConversation = dm, title: string | null = null, unreadAfterSeq: number | null = null, onTailVisibleChange?: (visible: boolean) => void) {
  await act(async () => root.render(
    <ConversationView conversation={conversation} messages={messages} meId="me" meHandle="me"
      loading={false} sending={false} error={null} onSend={async () => {}} title={title} unreadAfterSeq={unreadAfterSeq}
      onTailVisibleChange={onTailVisibleChange} />,
  ));
}

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it('groups one author within five minutes but keeps every sequence marker', async () => {
  await render([
    message(1, 'peer', '2026-09-24T14:00:00Z'),
    message(2, 'peer', '2026-09-24T14:04:59Z'),
    message(3, 'me', '2026-09-24T14:05:30Z'),
    message(4, 'me', '2026-09-24T14:11:00Z'),
  ]);
  const rows = [...host.querySelectorAll('.message-row')];
  expect(rows.map(row => row.classList.contains('message-row--follow'))).toEqual([false, true, false, false]);
  // Follow-ups hide the header visually, never from assistive tech or tooling.
  expect([...host.querySelectorAll('.message-sequence')].map(el => el.textContent)).toEqual(['#1', '#2', '#3', '#4']);
  expect(rows[1].querySelector('.message-meta')?.classList.contains('sr-only')).toBe(true);
  expect(rows[1].querySelector('.message-meta strong')?.textContent).toBe('Synthetic Peer');
  expect([...host.querySelectorAll('p.whitespace-pre-wrap')].map(p => p.textContent)).toEqual(['body-1', 'body-2', 'body-3', 'body-4']);
});

it('starts a new group across a day boundary or missing timestamps', async () => {
  await render([
    message(1, 'peer', '2026-09-24T23:59:00Z'),
    message(2, 'peer', '2026-09-25T00:01:00Z'),
    message(3, 'peer', ''),
    message(4, 'peer', ''),
  ]);
  expect([...host.querySelectorAll('.message-row')].every(row => !row.classList.contains('message-row--follow'))).toBe(true);
});

it('styles the channel hash without changing the literal title text', async () => {
  await render([], { id: 'ch', kind: 'channel', members: [] }, '#general');
  expect(host.querySelector('h2')?.textContent).toBe('#general');
  expect(host.querySelector('h2 .channel-glyph')?.textContent).toBe('#');
});

it('puts one NEW divider before the first unread message from someone else', async () => {
  const history = [
    message(1, 'peer', '2026-09-24T14:00:00Z'),
    message(2, 'me', '2026-09-24T14:01:00Z'),
    message(3, 'peer', '2026-09-24T14:02:00Z'),
    message(4, 'peer', '2026-09-24T14:03:00Z'),
  ];
  await render(history, dm, null, 1);
  const dividers = host.querySelectorAll('.new-divider');
  expect(dividers).toHaveLength(1);
  // Own message 2 is never "new"; the divider sits right above message 3,
  // which also restarts the author header instead of grouping.
  const next = dividers[0].nextElementSibling;
  expect(next?.querySelector('.message-sequence')?.textContent).toBe('#3');
  expect(next?.classList.contains('message-row--follow')).toBe(false);
  expect(dividers[0].getAttribute('aria-label')).toBe('New messages');
});

it('shows no divider without a marker or when only your own messages follow it', async () => {
  const history = [message(1, 'peer', '2026-09-24T14:00:00Z'), message(2, 'me', '2026-09-24T14:01:00Z')];
  await render(history, dm, null, null);
  expect(host.querySelector('.new-divider')).toBeNull();
  await render(history, dm, null, 1);
  expect(host.querySelector('.new-divider')).toBeNull();
});

/** jsdom has no layout: give `.message-history` a browser-like scroll box. */
function stubScrollBox(scrollHeight = 1000, clientHeight = 200, dividerOffset = 500): () => void {
  const tops = new WeakMap<Element, number>();
  const isBox = (el: Element) => el.classList.contains('message-history');
  Object.defineProperty(HTMLElement.prototype, 'scrollHeight', { configurable: true, get(this: HTMLElement) { return isBox(this) ? scrollHeight : 0; } });
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, get(this: HTMLElement) { return isBox(this) ? clientHeight : 0; } });
  Object.defineProperty(HTMLElement.prototype, 'scrollTop', {
    configurable: true,
    get(this: HTMLElement) { return tops.get(this) ?? 0; },
    set(this: HTMLElement, value: number) { tops.set(this, Math.max(0, Math.min(value, isBox(this) ? scrollHeight - clientHeight : 0))); },
  });
  // The NEW divider sits `dividerOffset` into the content; its viewport rect
  // moves with the box's scroll position, as in a real browser.
  const rects = vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
    const box = this.closest<HTMLElement>('.message-history');
    const top = isBox(this) ? 100 : this.classList.contains('new-divider') && box ? 100 + dividerOffset - box.scrollTop : 0;
    return { top, bottom: top, left: 0, right: 0, height: 0, width: 0, x: 0, y: top, toJSON() {} } as DOMRect;
  });
  return () => {
    rects.mockRestore();
    for (const key of ['scrollHeight', 'clientHeight', 'scrollTop']) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[key];
  };
}

it('opens an unread backlog at the NEW divider and reports the tail only once reached', async () => {
  const restore = stubScrollBox();
  try {
    const tail = vi.fn();
    const history = Array.from({ length: 6 }, (_, i) => message(i + 1, i < 2 ? 'me' : 'peer', `2026-09-24T14:0${i}:00Z`));
    await render(history, dm, null, 2, tail);
    const box = host.querySelector<HTMLElement>('.message-history')!;
    expect(box.scrollTop).toBe(484);
    expect(tail).toHaveBeenLastCalledWith(false);
    // More history arriving must not drag a reader away from the divider.
    await render([...history, message(7, 'peer', '2026-09-24T14:07:00Z')], dm, null, 2, tail);
    expect(box.scrollTop).toBe(484);
    await act(async () => { box.scrollTop = 800; box.dispatchEvent(new Event('scroll')); });
    expect(tail).toHaveBeenLastCalledWith(true);
  } finally {
    restore();
  }
});

it('still follows the tail when nothing was unread on open', async () => {
  const restore = stubScrollBox();
  try {
    const tail = vi.fn();
    await render([message(1, 'peer', '2026-09-24T14:00:00Z')], dm, null, null, tail);
    expect(host.querySelector<HTMLElement>('.message-history')!.scrollTop).toBe(800);
    expect(tail).toHaveBeenLastCalledWith(true);
  } finally {
    restore();
  }
});
