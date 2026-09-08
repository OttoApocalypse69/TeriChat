// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import App from './App';
import { ApiClient, encodeOpaqueText, type MessageBody } from './lib/api';
import type { GatewayOutboxEvent, GatewayStatus } from './lib/gateway';

const gateways = vi.hoisted(() => [] as { onEvent: (e: GatewayOutboxEvent) => void; onStatus: (s: GatewayStatus) => void }[]);
vi.mock('./lib/gateway', () => ({ GatewayClient: class {
  constructor(options: typeof gateways[number]) { gateways.push(options); }
  connect() {}
  close() {}
} }));
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
  gateways.length = 0;
  vi.spyOn(ApiClient.prototype, 'login').mockImplementation(async handle => ({ token: `token-${handle}`, session_id: handle, user_id: handle, user_handle: handle, expires_at: '' }));
  vi.spyOn(ApiClient.prototype, 'logout').mockResolvedValue({ status: 'ok', session_id: 'a' });
  vi.spyOn(ApiClient.prototype, 'listConversations').mockResolvedValue([row()]);
  vi.spyOn(ApiClient.prototype, 'listWorkspaces').mockResolvedValue([]);
  vi.spyOn(ApiClient.prototype, 'history').mockResolvedValue([]);
  vi.spyOn(ApiClient.prototype, 'listChannels').mockResolvedValue([]);
  vi.spyOn(ApiClient.prototype, 'listAudit').mockResolvedValue([]);
  vi.spyOn(ApiClient.prototype, 'listMembers').mockResolvedValue({ members: [], next_cursor: null });
  vi.spyOn(ApiClient.prototype, 'listInvites').mockResolvedValue([]);
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  await flush(() => root.render(<App />));
});
afterEach(async () => { await flush(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

it('does not publish a deferred conversation list from the previous account', async () => {
  const old = deferred<ReturnType<typeof row>[]>();
  vi.mocked(ApiClient.prototype.listConversations).mockReturnValueOnce(old.promise);
  await login(); await click('Log out'); await login('b');
  await flush(() => old.resolve([row('private', 'OLD-ACCOUNT-PRIVATE')]));
  expect(host.textContent).not.toContain('OLD-ACCOUNT-PRIVATE');
});

it('does not chain an old create-DM response into requests after logout/login', async () => {
  const old = deferred<ReturnType<typeof row>>();
  vi.spyOn(ApiClient.prototype, 'createDm').mockReturnValue(old.promise);
  await login(); await type('peer handle', 'old-peer'); await click('Open DM');
  await click('Log out'); await login('b');
  const calls = vi.mocked(ApiClient.prototype.listConversations).mock.calls.length;
  await flush(() => old.resolve(row('private', 'OLD-ACCOUNT-PRIVATE')));
  expect(host.textContent).not.toContain('OLD-ACCOUNT-PRIVATE');
  expect(ApiClient.prototype.listConversations).toHaveBeenCalledTimes(calls);
});

const workspace = (name = 'Current Workspace') => ({ id: 'shared-ws', name, owner_id: 'a', my_role: 'owner', created_at: '', updated_at: '' });
const channel = (name = 'OLD-ACCOUNT-PRIVATE') => ({ id: 'old-channel', workspace_id: 'shared-ws', conversation_id: 'old-channel-conv', name, kind: 'text', created_by: 'a', created_at: '' });
async function submitInput(placeholder: string, value: string) {
  await type(placeholder, value);
  await flush(() => input(placeholder).form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
}

it.each(['conversations', 'workspaces', 'channels', 'history', 'gateway-history', 'send', 'create-dm', 'join', 'create-channel', 'leave'] as const)(
  'isolates late %s success AND failure from the next account', async kind => {
    // Exercise each callback twice: once with a successful response and once
    // with a rejection. Both must leave the new account's DOM unchanged.
    for (const outcome of ['success', 'failure']) {
      const old = deferred<never>();
      let result: unknown;
      vi.mocked(ApiClient.prototype.listWorkspaces).mockResolvedValue([workspace()]);
      vi.mocked(ApiClient.prototype.listChannels).mockResolvedValue([]);
      if (kind === 'conversations') {
        vi.mocked(ApiClient.prototype.listConversations).mockReturnValueOnce(old.promise);
        result = [row('private', 'OLD-ACCOUNT-PRIVATE')];
      }
      if (kind === 'workspaces') {
        vi.mocked(ApiClient.prototype.listWorkspaces).mockReturnValueOnce(old.promise);
        result = [workspace('OLD-ACCOUNT-PRIVATE')];
      }
      if (kind === 'channels') {
        vi.mocked(ApiClient.prototype.listChannels).mockReturnValueOnce(old.promise);
        result = [channel()];
      }
      await login('a');
      if (kind === 'history' || kind === 'gateway-history') {
        vi.mocked(ApiClient.prototype.history).mockReturnValueOnce(old.promise);
        if (kind === 'history') await click('@Peer');
        else await flush(() => gateways[gateways.length - 1].onEvent(event(1)));
        result = [message(1)];
      }
      if (kind === 'send') {
        await click('@Peer');
        vi.spyOn(ApiClient.prototype, 'sendMessage').mockReturnValueOnce(old.promise);
        await submitInput('Message', 'old-message'); result = message(1);
      }
      if (kind === 'create-dm') {
        vi.spyOn(ApiClient.prototype, 'createDm').mockReturnValueOnce(old.promise);
        await submitInput('peer handle', 'old-peer'); result = row('private', 'OLD-ACCOUNT-PRIVATE');
      }
      if (kind === 'join') {
        vi.spyOn(ApiClient.prototype, 'joinWorkspace').mockReturnValueOnce(old.promise);
        await submitInput('paste invite', 'synthetic-code'); result = workspace('OLD-ACCOUNT-PRIVATE');
      }
      if (kind === 'create-channel') {
        vi.spyOn(ApiClient.prototype, 'createChannel').mockReturnValueOnce(old.promise);
        await submitInput('new channel', 'old-channel'); result = channel();
      }
      if (kind === 'leave') {
        vi.spyOn(ApiClient.prototype, 'leaveWorkspace').mockReturnValueOnce(old.promise);
        await click('Leave workspace');
      }
      const oldGateway = gateways[gateways.length - 1];
      await click('Log out'); await login('b'); await click('@Peer');
      await type('Message', 'new-account-draft');
      const before = host.innerHTML;
      await flush(() => outcome === 'success' ? old.resolve(result as never) : old.reject(new Error('OLD-ACCOUNT-ERROR')));
      expect(host.innerHTML).toBe(before);
      await flush(() => { oldGateway.onStatus('reconnecting'); oldGateway.onEvent(event(99)); });
      expect(host.innerHTML).toBe(before);
      expect(input('Message').value).toBe('new-account-draft');
      expect(host.textContent).toContain('Current Workspace');
      await click('Log out');
    }
  },
);

it('keeps the new account sending while the old account send rejects', async () => {
  const old = deferred<MessageBody>();
  const current = deferred<MessageBody>();
  vi.spyOn(ApiClient.prototype, 'sendMessage').mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
  await login(); await click('@Peer'); await submitInput('Message', 'old send');
  await click('Log out'); await login('b'); await click('@Peer');
  await submitInput('Message', 'new send');
  const before = host.innerHTML;
  await flush(() => old.reject(new Error('OLD-ACCOUNT-ERROR')));
  expect(host.innerHTML).toBe(before);
  expect(host.querySelector('main button[type=submit]')!.textContent).toBe('…');
  await flush(() => current.resolve(message(1)));
  expect(input('Message').value).toBe('');
});

it('drains more than 100 messages without skipping a gap after concurrent send and gateway echo', async () => {
  const first = deferred<MessageBody[]>();
  const history = vi.mocked(ApiClient.prototype.history);
  history.mockImplementation(async (_id, since = 0) => {
    if (since === 0) return first.promise;
    return Array.from({ length: 151 }, (_, i) => message(i + 1)).filter(m => m.seq > since).slice(0, 100);
  });
  vi.spyOn(ApiClient.prototype, 'sendMessage').mockResolvedValue(message(151));
  await login(); await click('@Peer');
  await type('Message', 'body-151'); await click('Send');
  await flush(() => gateways[0].onEvent(event(151)));
  await flush(() => first.resolve(Array.from({ length: 100 }, (_, i) => message(i + 1))));
  const bodies = [...host.querySelectorAll('main p.whitespace-pre-wrap')].map(p => p.textContent);
  expect(bodies).toEqual(Array.from({ length: 151 }, (_, i) => `body-${i + 1}`));
  expect(history.mock.calls.some(([, since]) => since === 100)).toBe(true);
});

it('recovers a failed event fetch on reconnect without a newer event', async () => {
  await login(); await click('@Peer');
  await flush(() => gateways[0].onStatus('connected'));
  vi.mocked(ApiClient.prototype.history).mockRejectedValueOnce(new Error('offline'));
  await flush(() => gateways[0].onEvent(event(1)));
  expect(host.textContent).not.toContain('body-1');
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([message(1)]);
  await flush(() => gateways[0].onStatus('reconnecting'));
  await flush(() => gateways[0].onStatus('connected'));
  expect(host.querySelector('main')!.textContent).toContain('body-1');
});

it('honors a reconnect queued while the failing history request is still pending', async () => {
  await login(); await click('@Peer');
  const failed = deferred<MessageBody[]>();
  vi.mocked(ApiClient.prototype.history).mockReturnValueOnce(failed.promise).mockResolvedValue([message(1)]);
  await flush(() => gateways[0].onEvent(event(1)));
  await flush(() => gateways[0].onStatus('reconnecting'));
  await flush(() => gateways[0].onStatus('connected'));
  await flush(() => failed.reject(new Error('previous connection failed')));
  expect(host.querySelector('main')!.textContent).toContain('body-1');
});

it('retries a failed history fetch when the same event is replayed', async () => {
  await login(); await click('@Peer');
  vi.mocked(ApiClient.prototype.history).mockRejectedValueOnce(new Error('temporary'));
  await flush(() => gateways[0].onEvent(event(1)));
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([message(1)]);
  await flush(() => gateways[0].onEvent(event(1)));
  expect(host.querySelector('main')!.textContent).toContain('body-1');
});

it('preserves composer draft and focus through a gateway/history rerender', async () => {
  await login(); await click('@Peer');
  await type('Message', 'unfinished draft');
  const composer = input('Message'); composer.focus();
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([message(1)]);
  await flush(() => gateways[0].onEvent(event(1)));
  expect(input('Message')).toBe(composer);
  expect(input('Message').value).toBe('unfinished draft');
  expect(document.activeElement).toBe(composer);
});

it('does not erase text typed while a send response is pending', async () => {
  const sent = deferred<MessageBody>();
  vi.spyOn(ApiClient.prototype, 'sendMessage').mockReturnValue(sent.promise);
  await login(); await click('@Peer');
  await submitInput('Message', 'first draft');
  await type('Message', 'next draft');
  const composer = input('Message'); composer.focus();
  await flush(() => sent.resolve(message(1)));
  expect(input('Message').value).toBe('next draft');
  expect(document.activeElement).toBe(composer);
});

it('resolves peer metadata for a first inbound DM without logging in again', async () => {
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValueOnce([]);
  await login();
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([row('dm', 'Incoming Friend')]);
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([message(1)]);
  await flush(() => gateways[0].onEvent(event(1)));
  expect(host.textContent).toContain('Incoming Friend');
  await click('@Incoming Friend');
  expect(host.querySelector('main')!.textContent).toContain('body-1');
});


it('creates a workspace, selects its owner context and permits creating its first channel', async () => {
  const created = { ...workspace('New Space'), id: 'new-space' };
  vi.spyOn(ApiClient.prototype, 'createWorkspace').mockResolvedValue(created);
  vi.spyOn(ApiClient.prototype, 'createChannel').mockResolvedValue({ ...channel('general'), workspace_id: created.id });
  await login();
  await submitInput('new workspace name', '  New Space  ');
  expect(ApiClient.prototype.createWorkspace).toHaveBeenCalledWith('New Space');
  expect(host.textContent).toContain('Channels · New Space');
  expect(host.textContent).toContain('Members · you are owner');
  expect(ApiClient.prototype.listChannels).toHaveBeenCalledWith(created.id);
  expect(input('new workspace name').value).toBe('');
  await submitInput('new channel name', 'general');
  expect(ApiClient.prototype.createChannel).toHaveBeenCalledWith(created.id, 'general');
  expect(host.textContent).toContain('#general');
});

it('preserves the workspace name and exposes create failure for retry, preventing duplicate submits', async () => {
  const pending = deferred<ReturnType<typeof workspace>>();
  const create = vi.spyOn(ApiClient.prototype, 'createWorkspace').mockReturnValueOnce(pending.promise)
    .mockResolvedValueOnce(workspace('Retry Space'));
  await login();
  await submitInput('new workspace name', 'Retry Space');
  await flush(() => input('new workspace name').form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
  expect(create).toHaveBeenCalledTimes(1);
  expect(host.textContent).toContain('Creating…');
  await flush(() => pending.reject(new Error('workspace creation failed')));
  expect(host.textContent).toContain('workspace creation failed');
  expect(input('new workspace name').value).toBe('Retry Space');
  await submitInput('new workspace name', 'Retry Space');
  expect(host.textContent).toContain('Channels · Retry Space');
  expect(host.textContent).not.toContain('workspace creation failed');
});

it('does not let a workspace list started before creation erase the new workspace', async () => {
  const pending = deferred<ReturnType<typeof workspace>[]>();
  vi.mocked(ApiClient.prototype.listWorkspaces).mockReturnValueOnce(pending.promise);
  vi.spyOn(ApiClient.prototype, 'createWorkspace').mockResolvedValue(workspace('Created During Load'));
  await login();
  await submitInput('new workspace name', 'Created During Load');
  await flush(() => pending.resolve([]));
  expect(host.textContent).toContain('Channels · Created During Load');
  expect(host.textContent).toContain('Members · you are owner');
});

it.each(['success', 'failure'] as const)('isolates workspace creation %s from the next account', async outcome => {
  const pending = deferred<ReturnType<typeof workspace>>();
  vi.spyOn(ApiClient.prototype, 'createWorkspace').mockReturnValueOnce(pending.promise);
  await login(); await submitInput('new workspace name', 'private old workspace');
  await click('Log out'); await login('b');
  const before = host.innerHTML;
  await flush(() => outcome === 'success'
    ? pending.resolve(workspace('private old workspace')) : pending.reject(new Error('private old error')));
  expect(host.innerHTML).toBe(before);
});
