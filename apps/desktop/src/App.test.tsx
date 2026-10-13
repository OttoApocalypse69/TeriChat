// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import App from './App';
import { ApiClient, ApiError, encodeOpaqueText, type MessageBody } from './lib/api';
import type { GatewayOutboxEvent, GatewayStatus } from './lib/gateway';

const gateways = vi.hoisted(() => [] as { onEvent: (e: GatewayOutboxEvent) => void; onStatus: (s: GatewayStatus) => void; onTyping?: (s: { conversationId: string; userId: string; lastSeq: number }) => void }[]);
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

it('filters loaded conversations without resetting the active composer or fetching again', async () => {
  await login(); await click('Peer'); await type('Message', 'preserved draft');
  const composer = input('Message');
  const calls = vi.mocked(ApiClient.prototype.listConversations).mock.calls.length;
  await type('Find a conversation', 'unmatched');
  expect(host.querySelector('.conversation-row')).toBeNull();
  expect(host.textContent).toContain('No matching conversations.');
  expect(input('Message')).toBe(composer);
  expect(composer.value).toBe('preserved draft');
  await type('Find a conversation', 'PEER');
  expect(host.querySelector('.conversation-row')?.textContent).toContain('Peer');
  expect(ApiClient.prototype.listConversations).toHaveBeenCalledTimes(calls);
});

it('filters channel names without changing the selected conversation', async () => {
  vi.mocked(ApiClient.prototype.listWorkspaces).mockResolvedValue([workspace()]);
  vi.mocked(ApiClient.prototype.listChannels).mockResolvedValue([channel('general')]);
  await login(); await click('general'); await type('Message', 'channel draft');
  await type('Find a conversation', '#GENERAL');
  expect(host.querySelector('.channel-list button[aria-current="page"]')?.textContent).toContain('general');
  await type('Find a conversation', '#missing');
  expect(host.querySelector('.channel-list button[aria-current="page"]')).toBeNull();
  expect(host.textContent).toContain('No matching channels.');
  expect(input('Message').value).toBe('channel draft');
});

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

it('returns to navigation after leaving the workspace details pane', async () => {
  vi.mocked(ApiClient.prototype.listWorkspaces).mockResolvedValue([workspace()]);
  vi.spyOn(ApiClient.prototype, 'leaveWorkspace').mockResolvedValue(undefined);
  await login(); await click('Workspace details');
  expect(host.querySelector('[data-pane]')?.getAttribute('data-pane')).toBe('details');
  await click('Leave workspace');
  expect(host.querySelector('[data-pane]')?.getAttribute('data-pane')).toBe('navigation');
});

it('clears drafts between DMs and channels but not when visiting workspace details', async () => {
  vi.mocked(ApiClient.prototype.listWorkspaces).mockResolvedValue([workspace()]);
  vi.mocked(ApiClient.prototype.listChannels).mockResolvedValue([channel('general')]);
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([row(), row('second', 'Second')]);
  await login(); await click('@Peer'); await type('Message', 'first draft');
  await click('Workspace details');
  await flush(() => host.querySelector<HTMLButtonElement>('#workspace-details button')!.click());
  expect(input('Message').value).toBe('first draft');
  await click('Back to conversations'); await click('@Second');
  expect(input('Message').value).toBe('');
  await type('Message', 'second draft');
  await click('Back to conversations'); await click('#general');
  expect(input('Message').value).toBe('');
  await type('Message', 'channel draft');
  await click('Back to conversations'); await click('@Peer');
  expect(input('Message').value).toBe('');
  expect(host.querySelector('main')!.textContent).not.toContain('#general');
});

it('returns to navigation and reopens the same conversation without losing its draft', async () => {
  await login(); await click('@Peer');
  await type('Message', 'unfinished mobile draft');
  const composer = input('Message');
  await click('Back to conversations');
  expect(host.querySelector('[data-pane]')?.getAttribute('data-pane')).toBe('navigation');
  expect(host.querySelector('button[aria-current="page"]')).toBe(document.activeElement);
  await click('@Peer');
  expect(host.querySelector('[data-pane]')?.getAttribute('data-pane')).toBe('conversation');
  expect(input('Message')).toBe(composer);
  expect(composer.value).toBe('unfinished mobile draft');
  expect(document.activeElement?.getAttribute('aria-label')).toBe('Conversation');
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
  vi.mocked(ApiClient.prototype.listWorkspaces).mockResolvedValueOnce([]).mockResolvedValue([created]);
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
  vi.mocked(ApiClient.prototype.listWorkspaces).mockResolvedValueOnce([]).mockResolvedValue([workspace('Retry Space')]);
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

it.each(['create', 'join'] as const)('refreshes after %s to retain existing memberships from a discarded initial list', async action => {
  const pending = deferred<ReturnType<typeof workspace>[]>();
  const existing = { ...workspace('Existing Space'), id: 'existing' };
  const created = workspace('Created During Load');
  vi.mocked(ApiClient.prototype.listWorkspaces).mockReturnValueOnce(pending.promise)
    .mockResolvedValueOnce([existing, created]);
  vi.spyOn(ApiClient.prototype, 'createWorkspace').mockResolvedValue(created);
  vi.spyOn(ApiClient.prototype, 'joinWorkspace').mockResolvedValue(created);
  await login();
  await submitInput(action === 'create' ? 'new workspace name' : 'paste invite', 'Created During Load');
  await flush(() => pending.resolve([existing]));
  expect(ApiClient.prototype.listWorkspaces).toHaveBeenCalledTimes(2);
  expect(host.textContent).toContain('Existing Space');
  expect(host.textContent).toContain('Channels · Created During Load');
  expect(host.textContent).toContain('Members · you are owner');
});

it('ignores a creation refresh superseded by a later joined workspace', async () => {
  const firstRefresh = deferred<ReturnType<typeof workspace>[]>();
  const first = { ...workspace('First Created'), id: 'first' };
  const second = { ...workspace('Second Joined'), id: 'second' };
  vi.mocked(ApiClient.prototype.listWorkspaces).mockResolvedValueOnce([])
    .mockReturnValueOnce(firstRefresh.promise).mockResolvedValueOnce([first, second]);
  vi.spyOn(ApiClient.prototype, 'createWorkspace').mockResolvedValue(first);
  vi.spyOn(ApiClient.prototype, 'joinWorkspace').mockResolvedValue(second);
  await login();
  await submitInput('new workspace name', 'First Created');
  await submitInput('paste invite', 'synthetic-invite');
  await flush(() => firstRefresh.resolve([first]));
  expect(host.textContent).toContain('Channels · Second Joined');
  expect(host.textContent).toContain('First Created');
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

it.each(['success', 'failure'])('keeps the new account logged in after old self-revoke %s', async outcome => {
  const pending = deferred<void>();
  vi.spyOn(ApiClient.prototype, 'listSessions').mockResolvedValue({ sessions: [{ id: 'a', device_id: null, created_at: '', expires_at: '', is_current: true }], next_cursor: null });
  vi.spyOn(ApiClient.prototype, 'revokeSession').mockReturnValue(pending.promise);
  await login('a'); await click('Sessions'); await click('End this session and log out');
  await click('Log out'); await login('b'); const before = host.innerHTML;
  await flush(() => outcome === 'success' ? pending.resolve() : pending.reject(new Error('synthetic failure')));
  expect(host.innerHTML).toBe(before); expect(host.textContent).toContain('@b');
});
it('ends the current session locally without POST logout or refetch and preserves composer draft/focus when closing inventory', async () => {
  vi.spyOn(ApiClient.prototype, 'listSessions').mockResolvedValue({ sessions: [{ id: 'a', device_id: null, created_at: '', expires_at: '', is_current: true }], next_cursor: null });
  vi.spyOn(ApiClient.prototype, 'revokeSession').mockResolvedValue();
  await login(); await click('@Peer'); await type('Message', 'synthetic unsent draft');
  await click('Sessions'); await click('Close sessions');
  expect(input('Message').value).toBe('synthetic unsent draft'); expect(document.activeElement?.textContent).toBe('Sessions');
  await click('Sessions'); await click('End this session and log out');
  expect(host.textContent).toContain('Log in'); expect(ApiClient.prototype.logout).not.toHaveBeenCalled();
  expect(ApiClient.prototype.listSessions).toHaveBeenCalledTimes(1);
});
it.each(['Close sessions', 'Escape', 'Sessions'])('completes self-revoke after inventory is hidden via %s', async close => {
  const pending = deferred<void>();
  vi.spyOn(ApiClient.prototype, 'listSessions').mockResolvedValue({ sessions: [{ id: 'a', device_id: null, created_at: '', expires_at: '', is_current: true }], next_cursor: null });
  vi.spyOn(ApiClient.prototype, 'revokeSession').mockReturnValue(pending.promise);
  await login(); await click('Sessions'); await click('End this session and log out');
  if (close === 'Escape') await flush(() => host.querySelector('#account-sessions')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  else await click(close);
  await flush(() => pending.resolve());
  expect(host.textContent).toContain('Log in'); expect(ApiClient.prototype.logout).not.toHaveBeenCalled();
  expect(ApiClient.prototype.listSessions).toHaveBeenCalledTimes(1);
});

it('creates a group from handles, then names it from the refreshed roster', async () => {
  const created = { id: 'grp', kind: 'group', members: ['a', 'u1', 'u2'] };
  const roster = { ...created, peer_handle: null, peer_display_name: null, last_seq: null, last_sent_at: null,
    member_profiles: [{ user_id: 'a', handle: 'a', display_name: '' }, { user_id: 'u1', handle: 'ana', display_name: 'Ana' }, { user_id: 'u2', handle: 'bo', display_name: '' }] };
  const createGroup = vi.spyOn(ApiClient.prototype, 'createGroup').mockResolvedValue(created);
  await login();
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([row(), roster]);
  await click('New group');
  // The caller's own handle and duplicates are dropped before the request.
  await type('handles, comma-separated', '@ana, bo, a, ana');
  await flush(() => input('handles, comma-separated').form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
  expect(createGroup).toHaveBeenCalledWith(['ana', 'bo']);
  expect(host.querySelector('main h2')?.textContent).toBe('Ana, @bo');
  expect(host.querySelector('.conversation-row[aria-current="page"]')?.textContent).toContain('Ana, @bo');
  expect(host.querySelector('form[aria-label="New group"]')).toBeNull();
});

it('rejects groups with fewer than two other people without calling the server', async () => {
  const createGroup = vi.spyOn(ApiClient.prototype, 'createGroup');
  await login();
  await click('New group');
  for (const members of ['@a', 'ana, a', 'ana, ANA']) {
    await type('handles, comma-separated', members);
    await flush(() => input('handles, comma-separated').form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
    expect(host.querySelector('form[aria-label="New group"] [role="alert"]')?.textContent).toContain('at least two other');
  }
  expect(createGroup).not.toHaveBeenCalled();
});

const fromPeer = (seq: number): MessageBody => ({ ...message(seq), sender_id: 'peer' });
const unreadRow = { ...row(), last_seq: 2, last_read_seq: 0 };
function visibility(focused: boolean, laidOut = true) {
  vi.spyOn(document, 'hasFocus').mockReturnValue(focused);
  vi.spyOn(Element.prototype, 'getClientRects').mockReturnValue({ length: laidOut ? 1 : 0 } as DOMRectList);
}

it('shows private unread counts and marks read only once the conversation is seen', async () => {
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([unreadRow]);
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
  const markRead = vi.spyOn(ApiClient.prototype, 'markRead').mockImplementation(async (id, seq) => ({ conversation_id: id, last_read_seq: seq }));
  visibility(false);
  await login();
  expect(host.querySelector('.conversation-row .unread-badge')?.textContent).toBe('2');
  expect(document.title).toMatch(/^\(2\)/);

  await click('Peer');
  await flush();
  expect(markRead).not.toHaveBeenCalled();
  expect(host.querySelector('main .new-divider')).not.toBeNull();

  visibility(true);
  await flush(() => window.dispatchEvent(new Event('focus')));
  await flush();
  expect(markRead).toHaveBeenCalledTimes(1);
  expect(markRead).toHaveBeenCalledWith('dm', 2, expect.any(AbortSignal));
  expect(host.querySelector('.conversation-row .unread-badge')).toBeNull();
  expect(document.title).not.toMatch(/^\(/);
  // The divider stays where the conversation was opened, even once read.
  expect(host.querySelector('main .new-divider')).not.toBeNull();
});

it('never marks a hidden pane read', async () => {
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([unreadRow]);
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
  const markRead = vi.spyOn(ApiClient.prototype, 'markRead').mockImplementation(async (id, seq) => ({ conversation_id: id, last_read_seq: seq }));
  visibility(true, false);
  await login(); await click('Peer');
  await flush(() => window.dispatchEvent(new Event('focus')));
  expect(markRead).not.toHaveBeenCalled();
  expect(host.querySelector('.conversation-row .unread-badge')?.textContent).toBe('2');
});

it('stays inert against servers without read markers', async () => {
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
  const markRead = vi.spyOn(ApiClient.prototype, 'markRead');
  visibility(true);
  await login(); await click('Peer');
  await flush(() => window.dispatchEvent(new Event('focus')));
  expect(markRead).not.toHaveBeenCalled();
  expect(host.querySelector('.unread-badge, main .new-divider')).toBeNull();
});

it('lists a slow-to-create group without pulling the user out of a conversation they opened meanwhile', async () => {
  const pending = deferred<{ id: string; kind: string; members: string[] }>();
  vi.spyOn(ApiClient.prototype, 'createGroup').mockReturnValue(pending.promise);
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([row(), row('dm2', 'Second')]);
  await login();
  await click('New group');
  await type('handles, comma-separated', 'ana, bo');
  await flush(() => input('handles, comma-separated').form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
  await click('Second');
  await type('Message', 'draft typed while the group was pending');
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([row(), row('dm2', 'Second'), { ...row('grp', ''), kind: 'group', peer_handle: null, peer_display_name: null }]);
  await flush(() => pending.resolve({ id: 'grp', kind: 'group', members: ['a', 'ana'] }));
  await flush();
  expect(host.querySelector('main h2')?.textContent).toBe('Second');
  expect(input('Message').value).toBe('draft typed while the group was pending');
  expect([...host.querySelectorAll('.conversation-row')].some(r => r.textContent?.includes('Group'))).toBe(true);
});

it('refreshes the list when a message arrives for a conversation this client never listed', async () => {
  await login();
  const calls = vi.mocked(ApiClient.prototype.listConversations).mock.calls.length;
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([row(), {
    ...row('grp', ''), kind: 'group', members: ['a', 'u1'], peer_handle: null, peer_display_name: null,
    member_profiles: [{ user_id: 'a', handle: 'a', display_name: '' }, { user_id: 'u1', handle: 'ana', display_name: 'Ana' }],
  }]);
  const created: GatewayOutboxEvent = { id: 'e-grp', topic: 'message.created', payload: { conversation_id: 'grp', data: { message_id: 'g1', seq: 1 } } };
  await flush(() => gateways[0].onEvent(created));
  await flush();
  expect(vi.mocked(ApiClient.prototype.listConversations).mock.calls.length).toBeGreaterThan(calls);
  expect([...host.querySelectorAll('.conversation-row')].map(r => r.textContent).join(' ')).toContain('Ana');
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

it('does not mark read until the end of an unread backlog is on screen', async () => {
  const restore = stubScrollBox();
  try {
    vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([unreadRow]);
    vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
    const markRead = vi.spyOn(ApiClient.prototype, 'markRead').mockImplementation(async (id, seq) => ({ conversation_id: id, last_read_seq: seq }));
    visibility(true);
    await login(); await click('Peer');
    await flush(() => window.dispatchEvent(new Event('focus')));
    expect(markRead).not.toHaveBeenCalled();
    const box = host.querySelector<HTMLElement>('main .message-history')!;
    await flush(() => { box.scrollTop = 800; box.dispatchEvent(new Event('scroll')); });
    await flush();
    expect(markRead).toHaveBeenCalledWith('dm', 2, expect.any(AbortSignal));
  } finally {
    restore();
  }
});

it('marks a conversation that arrived without a marker once markers are known', async () => {
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([{ ...unreadRow, last_read_seq: 2 }, row('dm2', 'Second')]);
  vi.mocked(ApiClient.prototype.history).mockImplementation(async id => id === 'dm2'
    ? [{ ...fromPeer(1), id: 's1', conversation_id: 'dm2' }, { ...fromPeer(2), id: 's2', conversation_id: 'dm2' }]
    : [fromPeer(1), fromPeer(2)]);
  const markRead = vi.spyOn(ApiClient.prototype, 'markRead').mockImplementation(async (id, seq) => ({ conversation_id: id, last_read_seq: seq }));
  visibility(true);
  await login(); await click('Second');
  await flush();
  expect(markRead).toHaveBeenCalledWith('dm2', 2, expect.any(AbortSignal));
});

it('keeps unseen messages unread when replying before catching up', async () => {
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([unreadRow]);
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
  vi.spyOn(ApiClient.prototype, 'sendMessage').mockResolvedValue(message(3));
  const markRead = vi.spyOn(ApiClient.prototype, 'markRead').mockImplementation(async (id, seq) => ({ conversation_id: id, last_read_seq: seq }));
  visibility(false);
  await login(); await click('Peer');
  await type('Message', 'reply without reading'); await click('Send');
  expect(host.querySelector('.conversation-row .unread-badge')?.textContent).toBe('2');
  visibility(true);
  await flush(() => window.dispatchEvent(new Event('focus')));
  await flush();
  expect(markRead).toHaveBeenCalledWith('dm', 3, expect.any(AbortSignal));
});

it('drops the channels of a left workspace from the unread total', async () => {
  vi.mocked(ApiClient.prototype.listWorkspaces).mockResolvedValue([workspace()]);
  vi.mocked(ApiClient.prototype.listChannels).mockResolvedValue([channel('general')]);
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([
    { ...unreadRow, last_read_seq: 2 },
    { id: 'old-channel-conv', kind: 'channel', members: [], peer_handle: null, peer_display_name: null, last_seq: 2, last_sent_at: null, last_read_seq: 0 },
  ]);
  vi.spyOn(ApiClient.prototype, 'leaveWorkspace').mockResolvedValue(undefined);
  await login();
  expect(document.title).toMatch(/^\(2\)/);
  await click('Workspace details');
  await click('Leave workspace');
  expect(document.title).not.toMatch(/^\(/);
});

it('gives messages that arrived while the pane was hidden a fresh NEW divider', async () => {
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([unreadRow]);
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
  vi.spyOn(ApiClient.prototype, 'listSessions').mockResolvedValue({ sessions: [], next_cursor: null });
  const markRead = vi.spyOn(ApiClient.prototype, 'markRead').mockImplementation(async (id, seq) => ({ conversation_id: id, last_read_seq: seq }));
  visibility(true);
  await login(); await click('Peer'); await flush();
  expect(markRead).toHaveBeenLastCalledWith('dm', 2, expect.any(AbortSignal));

  // The sessions overlay hides the whole layout while a new message lands.
  await click('Sessions');
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2), fromPeer(3)]);
  await flush(() => gateways[0].onEvent(event(3)));
  await flush();
  expect(markRead).not.toHaveBeenCalledWith('dm', 3, expect.any(AbortSignal));

  await click('Close sessions');
  await flush();
  const divider = host.querySelector('main .new-divider');
  expect(divider?.nextElementSibling?.querySelector('.message-sequence')?.textContent).toBe('#3');
});

it('retries a read whose request stalled once the stall times out', async () => {
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([unreadRow]);
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
  const markRead = vi.spyOn(ApiClient.prototype, 'markRead').mockReturnValue(new Promise(() => {}));
  visibility(true);
  await login();
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
  try {
    await click('Peer'); await flush();
    expect(markRead).toHaveBeenCalledTimes(1);
    await flush(() => window.dispatchEvent(new Event('focus')));
    expect(markRead).toHaveBeenCalledTimes(1);
    await flush(() => { vi.advanceTimersByTime(15_000); });
    await flush(() => window.dispatchEvent(new Event('focus')));
    expect(markRead).toHaveBeenCalledTimes(2);
  } finally {
    vi.useRealTimers();
  }
});

const listedChannel = { id: 'old-channel-conv', kind: 'channel', members: [], peer_handle: null, peer_display_name: null, last_seq: 2, last_sent_at: null, last_read_seq: 0 };

it('prunes a channel the server stopped listing (remote kick or ban) on the next refresh', async () => {
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([{ ...unreadRow, last_read_seq: 2 }, listedChannel]);
  await login();
  expect(document.title).toMatch(/^\(2\)/);
  // Removed elsewhere: the reconnect's list no longer carries the channel.
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([{ ...unreadRow, last_read_seq: 2 }]);
  await flush(() => gateways[0].onStatus('connected'));
  await flush();
  expect(document.title).not.toMatch(/^\(/);
});

it('keeps a channel opened this session that no list has carried yet', async () => {
  vi.mocked(ApiClient.prototype.listWorkspaces).mockResolvedValue([workspace()]);
  vi.mocked(ApiClient.prototype.listChannels).mockResolvedValue([channel('general')]);
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([{ ...unreadRow, last_read_seq: 2 }]);
  await login(); await click('general');
  await flush(() => gateways[0].onStatus('connected'));
  await flush();
  expect(host.querySelector('main h2')?.textContent).toBe('#general');
});

it('shows who is typing in the open conversation until it expires or their message lands', async () => {
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1)]);
  await login(); await click('Peer'); await flush();
  const typing = () => host.querySelector('main .typing-indicator')?.textContent;
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
  try {
    await flush(() => gateways[0].onTyping?.({ conversationId: 'dm', userId: 'peer', lastSeq: 1 }));
    expect(typing()).toBe('Peer is typing…');
    // Other conversations' typists never show here.
    await flush(() => gateways[0].onTyping?.({ conversationId: 'elsewhere', userId: 'x', lastSeq: 0 }));
    expect(typing()).toBe('Peer is typing…');
    await flush(() => { vi.advanceTimersByTime(6_100); });
    expect(typing()).toBe('');

    // Typing again, then their message arrives: the indicator clears at once.
    await flush(() => gateways[0].onTyping?.({ conversationId: 'dm', userId: 'peer', lastSeq: 1 }));
    expect(typing()).toBe('Peer is typing…');
    vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
    await flush(() => gateways[0].onEvent(event(2)));
    await flush();
    expect(typing()).toBe('');
  } finally {
    vi.useRealTimers();
  }
});

it('does not read across a peer message that failed to load before your reply', async () => {
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([{ ...unreadRow, last_seq: 3 }]);
  // seq 2 (the peer's) never loaded; seq 3 is the caller's own reply.
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), message(3)]);
  const markRead = vi.spyOn(ApiClient.prototype, 'markRead').mockImplementation(async (id, seq) => ({ conversation_id: id, last_read_seq: seq }));
  visibility(true);
  await login(); await click('Peer'); await flush();
  expect(markRead).toHaveBeenCalledWith('dm', 1, expect.any(AbortSignal));
  expect(markRead).not.toHaveBeenCalledWith('dm', 3, expect.any(AbortSignal));
  expect(host.querySelector('.conversation-row .unread-badge')?.textContent).toBe('1');
});

it('aborts a stalled read request when it times out', async () => {
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([unreadRow]);
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
  const signals: AbortSignal[] = [];
  vi.spyOn(ApiClient.prototype, 'markRead').mockImplementation((_id, _seq, signal) => {
    if (signal) signals.push(signal);
    return new Promise(() => {});
  });
  visibility(true);
  await login();
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
  try {
    await click('Peer'); await flush();
    expect(signals).toHaveLength(1);
    expect(signals[0].aborted).toBe(false);
    await flush(() => { vi.advanceTimersByTime(15_000); });
    expect(signals[0].aborted).toBe(true);
  } finally {
    vi.useRealTimers();
  }
});

it('drops a typing signal that the typist\u2019s own later message overtook', async () => {
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
  await login(); await click('Peer'); await flush();
  const typing = () => host.querySelector('main .typing-indicator')?.textContent;
  // Sent while seq 1 was the latest, delivered after their seq 2 loaded.
  await flush(() => gateways[0].onTyping?.({ conversationId: 'dm', userId: 'peer', lastSeq: 1 }));
  expect(typing()).toBe('');
  await flush(() => gateways[0].onTyping?.({ conversationId: 'dm', userId: 'peer', lastSeq: 2 }));
  expect(typing()).toBe('Peer is typing…');
});

it('keeps a typist whose signal is newer than the message that just loaded', async () => {
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1)]);
  await login(); await click('Peer'); await flush();
  const typing = () => host.querySelector('main .typing-indicator')?.textContent;
  // Sent after their seq 2 (not loaded here yet): they are drafting the next one.
  await flush(() => gateways[0].onTyping?.({ conversationId: 'dm', userId: 'peer', lastSeq: 2 }));
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
  await flush(() => gateways[0].onEvent(event(2)));
  await flush();
  expect(host.querySelector('main')?.textContent).toContain('body-2');
  expect(typing()).toBe('Peer is typing…');
});

it('names channel authors and typists from the workspace member list', async () => {
  const kai = { user_id: 'u-kai', handle: 'kai', display_name: 'Kai', role: 'member', joined_at: '' };
  const bo = { user_id: 'u-bo', handle: 'bo', display_name: '', role: 'member', joined_at: '' };
  vi.mocked(ApiClient.prototype.listWorkspaces).mockResolvedValue([workspace()]);
  vi.mocked(ApiClient.prototype.listChannels).mockResolvedValue([channel('general')]);
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([
    { ...message(1), conversation_id: 'old-channel-conv', sender_id: 'u-kai' },
  ]);
  // Paged (the member panel pages the same list): the directory reads it all.
  const listMembers = vi.mocked(ApiClient.prototype.listMembers).mockImplementation(async (_ws, after) =>
    after === undefined ? { members: [kai], next_cursor: 'u-kai' } : { members: [bo], next_cursor: null });
  await login(); await click('general'); await flush();
  expect(listMembers).toHaveBeenCalledWith('shared-ws', 'u-kai');
  expect(host.querySelector('main .message-meta strong')?.textContent).toBe('Kai');
  const requests = listMembers.mock.calls.length;
  await flush(() => gateways[0].onTyping?.({ conversationId: 'old-channel-conv', userId: 'u-bo', lastSeq: 1 }));
  expect(host.querySelector('main .typing-indicator')?.textContent).toBe('@bo is typing…');
  // Everyone named already: no further directory requests.
  expect(listMembers).toHaveBeenCalledTimes(requests);
});

it('announces typing at most every few seconds and stops against servers without it', async () => {
  const sendTyping = vi.spyOn(ApiClient.prototype, 'sendTyping').mockResolvedValue(undefined);
  await login(); await click('Peer');
  vi.useFakeTimers({ toFake: ['Date'] });
  try {
    await type('Message', 'h');
    await type('Message', 'he');
    await type('Message', '');
    expect(sendTyping).toHaveBeenCalledTimes(1);
    expect(sendTyping).toHaveBeenCalledWith('dm');
    vi.advanceTimersByTime(3_000);
    await type('Message', 'hey');
    expect(sendTyping).toHaveBeenCalledTimes(2);

    sendTyping.mockRejectedValue(new ApiError(404, 'not_found', 'not found'));
    vi.advanceTimersByTime(3_000);
    await type('Message', 'hey!');
    await flush();
    vi.advanceTimersByTime(3_000);
    await type('Message', 'hey!!');
    expect(sendTyping).toHaveBeenCalledTimes(3);
  } finally {
    vi.useRealTimers();
  }
});

it('gives messages that arrived while you were away a NEW divider when you return', async () => {
  vi.mocked(ApiClient.prototype.listConversations).mockResolvedValue([{ ...unreadRow, last_read_seq: 2 }]);
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2)]);
  const markRead = vi.spyOn(ApiClient.prototype, 'markRead').mockImplementation(async (id, seq) => ({ conversation_id: id, last_read_seq: seq }));
  visibility(true);
  await login(); await click('Peer'); await flush();
  expect(host.querySelector('main .new-divider')).toBeNull();

  // Switch away; a message lands while nobody is looking.
  visibility(false);
  await flush(() => window.dispatchEvent(new Event('blur')));
  vi.mocked(ApiClient.prototype.history).mockResolvedValue([fromPeer(1), fromPeer(2), fromPeer(3)]);
  await flush(() => gateways[0].onEvent(event(3)));
  await flush();
  expect(markRead).not.toHaveBeenCalledWith('dm', 3, expect.any(AbortSignal));

  visibility(true);
  await flush(() => window.dispatchEvent(new Event('focus')));
  await flush();
  const divider = host.querySelector('main .new-divider');
  expect(divider?.nextElementSibling?.querySelector('.message-sequence')?.textContent).toBe('#3');
  expect(markRead).toHaveBeenCalledWith('dm', 3, expect.any(AbortSignal));

  // Stepping away and back with nothing new keeps that divider.
  visibility(false);
  await flush(() => window.dispatchEvent(new Event('blur')));
  visibility(true);
  await flush(() => window.dispatchEvent(new Event('focus')));
  expect(host.querySelector('main .new-divider')?.nextElementSibling?.querySelector('.message-sequence')?.textContent).toBe('#3');
});

