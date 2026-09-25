// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it } from 'vitest';
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

async function render(messages: ChatMessage[], conversation: ChatConversation = dm, title: string | null = null, unreadAfterSeq: number | null = null) {
  await act(async () => root.render(
    <ConversationView conversation={conversation} messages={messages} meId="me" meHandle="me"
      loading={false} sending={false} error={null} onSend={async () => {}} title={title} unreadAfterSeq={unreadAfterSeq} />,
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
