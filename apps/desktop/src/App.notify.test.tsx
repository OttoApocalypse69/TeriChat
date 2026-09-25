// @vitest-environment jsdom
// Issue #43 through the real App + GatewayClient: an unfocused DM arrival
// toasts once (even on gateway redelivery), a focused arrival stays silent,
// and denied permission never blocks messaging.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import App from './App';
import { ApiClient, encodeOpaqueText, type MessageBody } from './lib/api';
import type { GatewayOutboxEvent, WsLike } from './lib/gateway';
import { __resetNotificationStateForTests } from './lib/notifications';

const plugin = vi.hoisted(() => ({
  isPermissionGranted: vi.fn(),
  requestPermission: vi.fn(),
  sendNotification: vi.fn(),
  onAction: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-notification', () => plugin);

class TestSocket implements WsLike {
  static instances: TestSocket[] = [];
  onopen: WsLike['onopen'] = null;
  onmessage: WsLike['onmessage'] = null;
  onclose: WsLike['onclose'] = null;
  onerror: WsLike['onerror'] = null;
  sent: string[] = [];
  constructor(_url: string) { TestSocket.instances.push(this); }
  send(data: string) { this.sent.push(data); }
  close() {}
  event(seq: number) {
    const { id, ...rest } = event(seq);
    this.onmessage?.({ data: JSON.stringify({ op: 'event', event: { event_id: id, ...rest } }) });
  }
}

let root: Root;
let host: HTMLDivElement;
const deferred = <T,>() => {
  let resolve!: (v: T) => void;
  let reject!: (e: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const row = (id = 'dm', name = 'Peer') => ({ id, kind: 'dm', members: ['a', 'peer'], peer_handle: name, peer_display_name: name, last_seq: null, last_sent_at: null });
// Peer-sent (not own): the toast path skips self echoes.
const peerMessage = (seq: number): MessageBody => ({ id: `m${seq}`, conversation_id: 'dm', sender_id: 'peer', seq, ciphertext_b64: encodeOpaqueText(`body-${seq}`), client_msg_id: `c${seq}`, sent_at: '2026-09-01T00:00:00Z', deduped: false });
const event = (seq: number): GatewayOutboxEvent => ({ id: `e${seq}`, topic: 'message.created', payload: { conversation_id: 'dm', data: { message_id: `m${seq}`, seq } } });
async function flush(fn: () => void = () => {}) { await act(async () => { fn(); }); }
function input(placeholder: string) { return host.querySelector<HTMLInputElement>(`input[placeholder^="${placeholder}"]`)!; }
async function type(placeholder: string, value: string) {
  await flush(() => {
    const el = input(placeholder);
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
}
async function click(text: string) {
  const button = [...host.querySelectorAll('button')].find(b => b.textContent?.includes(text));
  expect(button, `button ${text}`).toBeTruthy();
  await flush(() => button!.click());
}
async function login(name = 'a') {
  await type('handle', name); await type('password', 'synthetic-password'); await click('Log in');
}
function unfocused(focused: boolean) {
  vi.spyOn(document, 'hasFocus').mockReturnValue(focused);
}

beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  TestSocket.instances = [];
  // Simulate the Tauri shell so the plugin toast path is exercised.
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  __resetNotificationStateForTests();
  plugin.isPermissionGranted.mockReset().mockResolvedValue(true);
  plugin.requestPermission.mockReset().mockResolvedValue('granted');
  plugin.sendNotification.mockReset();
  plugin.onAction.mockReset().mockResolvedValue(undefined);
  vi.stubGlobal('WebSocket', TestSocket);
  vi.spyOn(ApiClient.prototype, 'login').mockImplementation(async handle => ({ token: `token-${handle}`, session_id: handle, user_id: handle, user_handle: handle, expires_at: '' }));
  vi.spyOn(ApiClient.prototype, 'logout').mockResolvedValue({ status: 'ok', session_id: 'a' });
  vi.spyOn(ApiClient.prototype, 'listConversations').mockResolvedValue([row()]);
  vi.spyOn(ApiClient.prototype, 'listWorkspaces').mockResolvedValue([]);
  vi.spyOn(ApiClient.prototype, 'history').mockResolvedValue([]);
  vi.spyOn(ApiClient.prototype, 'listChannels').mockResolvedValue([]);
  vi.spyOn(ApiClient.prototype, 'listAudit').mockResolvedValue([]);
  vi.spyOn(ApiClient.prototype, 'listInvites').mockResolvedValue([]);
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  await flush(() => root.render(<App />));
});
afterEach(async () => {
  await flush(() => root.unmount()); host.remove();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  vi.restoreAllMocks(); vi.unstubAllGlobals();
});

async function readySocket() {
  const socket = TestSocket.instances[0];
  await flush(() => {
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({ op: 'ready', session_id: 's', user_id: 'a' }) });
  });
  return socket;
}

it('unfocused DM arrival toasts sender plus text exactly once, even on redelivery', async () => {
  await login();
  const socket = await readySocket();
  await click('@Peer');
  const history = vi.mocked(ApiClient.prototype.history);
  history.mockClear();
  const pending = deferred<MessageBody[]>();
  history.mockReturnValueOnce(pending.promise).mockResolvedValue([peerMessage(1)]);
  unfocused(false);
  await flush(() => socket.event(1));
  // Redelivery of the same event while history is still pending.
  await flush(() => socket.event(1));
  await flush(() => pending.resolve([peerMessage(1)]));
  await flush();
  expect(plugin.sendNotification).toHaveBeenCalledTimes(1);
  expect(plugin.sendNotification).toHaveBeenCalledWith({ title: 'Peer', body: 'body-1' });
  expect([...host.querySelectorAll('main p.whitespace-pre-wrap')].map(p => p.textContent)).toEqual(['body-1']);
});

it('focused arrival stays silent', async () => {
  await login();
  const socket = await readySocket();
  await click('@Peer');
  const history = vi.mocked(ApiClient.prototype.history);
  history.mockClear();
  history.mockResolvedValue([peerMessage(1)]);
  unfocused(true);
  await flush(() => socket.event(1));
  await flush();
  expect(plugin.sendNotification).not.toHaveBeenCalled();
  expect(plugin.requestPermission).not.toHaveBeenCalled();
  expect([...host.querySelectorAll('main p.whitespace-pre-wrap')].map(p => p.textContent)).toEqual(['body-1']);
});

it('denied permission never errors and never blocks messaging', async () => {
  plugin.isPermissionGranted.mockResolvedValue(false);
  plugin.requestPermission.mockResolvedValue('denied');
  await login();
  const socket = await readySocket();
  await click('@Peer');
  const history = vi.mocked(ApiClient.prototype.history);
  history.mockClear();
  history.mockResolvedValue([peerMessage(1)]);
  unfocused(false);
  await flush(() => socket.event(1));
  await flush();
  expect(plugin.sendNotification).not.toHaveBeenCalled();
  expect([...host.querySelectorAll('main p.whitespace-pre-wrap')].map(p => p.textContent)).toEqual(['body-1']);
});

// A first message for a conversation this client has never listed: the
// history fetch can finish before the list says what kind of row it is.
type Summaries = Awaited<ReturnType<ApiClient['listConversations']>>;
async function unlistedArrival(listed: Summaries[number]) {
  await login();
  const socket = await readySocket();
  unfocused(false);
  const list = deferred<Summaries>();
  vi.mocked(ApiClient.prototype.listConversations).mockReturnValueOnce(list.promise);
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([{ ...peerMessage(1), conversation_id: listed.id }]);
  await flush(() => socket.onmessage?.({ data: JSON.stringify({ op: 'event', event: {
    event_id: `new-${listed.id}`, topic: 'message.created', payload: { conversation_id: listed.id, data: { message_id: 'm1', seq: 1 } },
  } }) }));
  await flush();
  // History is in, the list is not: nothing may be classified yet.
  expect(plugin.sendNotification).not.toHaveBeenCalled();
  await flush(() => list.resolve([row(), listed]));
  await flush();
}

it('never toasts an unlisted group as a DM, even when history beats the list', async () => {
  await unlistedArrival({ id: 'grp', kind: 'group', members: ['a', 'peer', 'u2'], peer_handle: null, peer_display_name: null, last_seq: 1, last_sent_at: null });
  expect(plugin.sendNotification).not.toHaveBeenCalled();
});

it('toasts a brand-new DM with its real peer name once the list resolves it', async () => {
  await unlistedArrival({ id: 'dm-new', kind: 'dm', members: ['a', 'peer'], peer_handle: 'newpeer', peer_display_name: 'New Peer', last_seq: 1, last_sent_at: null });
  expect(plugin.sendNotification).toHaveBeenCalledTimes(1);
  expect(plugin.sendNotification).toHaveBeenCalledWith({ title: 'New Peer', body: 'body-1' });
});

