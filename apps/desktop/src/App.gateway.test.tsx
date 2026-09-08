// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import App from './App';
import { ApiClient, encodeOpaqueText, type MessageBody } from './lib/api';
import type { GatewayOutboxEvent, WsLike } from './lib/gateway';

// Only the socket boundary is injected: App constructs the real GatewayClient.
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
const message = (seq: number): MessageBody => ({ id: `m${seq}`, conversation_id: 'dm', sender_id: 'a', seq, ciphertext_b64: encodeOpaqueText(`body-${seq}`), client_msg_id: `c${seq}`, sent_at: '2026-09-01T00:00:00Z', deduped: false });
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
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  TestSocket.instances = [];
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
afterEach(async () => { await flush(() => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it.each(['after failure', 'while pending'] as const)(
  'real gateway duplicate retries history %s and renders recovery exactly once', async timing => {
    await login();
    const socket = TestSocket.instances[0];
    await flush(() => {
      socket.onopen?.();
      socket.onmessage?.({ data: JSON.stringify({ op: 'ready', session_id: 's', user_id: 'a' }) });
    });
    expect(JSON.parse(socket.sent[0])).toEqual({ op: 'identify', resume_after: null });
    await click('@Peer');
    const history = vi.mocked(ApiClient.prototype.history);
    history.mockClear();
    const failed = deferred<MessageBody[]>();
    history.mockReturnValueOnce(failed.promise).mockResolvedValue([message(1)]);
    await flush(() => socket.event(1));
    expect(history).toHaveBeenCalledTimes(1);
    expect(host.querySelectorAll('main p.whitespace-pre-wrap')).toHaveLength(0);
    if (timing === 'while pending') {
      // Repeated frames coalesce into one follow-up, never concurrent requests.
      await flush(() => { socket.event(1); socket.event(1); });
      expect(history).toHaveBeenCalledTimes(1);
    }
    await flush(() => failed.reject(new Error('temporary history outage')));
    if (timing === 'after failure') {
      expect(host.textContent).toContain('temporary history outage');
      await flush(() => socket.event(1));
    }
    expect(history).toHaveBeenCalledTimes(2);
    expect(history.mock.calls).toEqual([['dm', 0, 100], ['dm', 0, 100]]);
    expect([...host.querySelectorAll('main p.whitespace-pre-wrap')].map(p => p.textContent)).toEqual(['body-1']);
    expect(host.textContent).not.toContain('temporary history outage');
    await flush(() => { socket.event(1); socket.event(1); });
    expect(history).toHaveBeenCalledTimes(2);
    expect(host.querySelectorAll('main p.whitespace-pre-wrap')).toHaveLength(1);
  },
);
