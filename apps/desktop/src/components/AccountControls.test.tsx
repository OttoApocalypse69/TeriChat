// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import SessionPanel from './SessionPanel';
import WorkspaceActivity from './WorkspaceActivity';
import { ApiClient, ApiError, type SessionsPage, type ChannelStatsPage, type WorkspaceStatsBody } from '../lib/api';
let root: Root; let host: HTMLDivElement; let api: ApiClient;
const ended = vi.fn();
const session = (id: string, is_current = false) => ({ id, is_current, device_id: null, created_at: '2026-09-01T00:00:00Z', expires_at: '2026-10-01T00:00:00Z' });
const sessions = (...ids: string[]): SessionsPage => ({ sessions: ids.map(id => session(id)), next_cursor: null });
const stats = (workspace_id = 'ws', user_id = 'me'): WorkspaceStatsBody => ({ workspace_id, user_id, message_count: 98, last_message_at: null });
const channels = (name = 'general'): ChannelStatsPage => ({ channels: [{ channel_id: name, conversation_id: name, name, message_count: 0, last_message_at: null }], next_cursor: null });
const deferred = <T,>() => { let resolve!: (value: T) => void; let reject!: (error: Error) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
async function flush(fn = () => {}) { await act(async () => { fn(); }); }
function button(label: string, scope: Element = host) { const found = [...scope.querySelectorAll('button')].find(b => b.textContent === label); expect(found, label).toBeTruthy(); return found!; }
async function click(label: string, scope: Element = host) { await flush(() => button(label, scope).click()); }
async function renderSessions(key = 'a') { await flush(() => root.render(<SessionPanel key={key} api={api} onSessionEnded={ended} />)); }
async function renderActivity(workspaceId = 'ws', meId = 'me') { await flush(() => root.render(<WorkspaceActivity api={api} workspaceId={workspaceId} meId={meId} />)); }
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); ended.mockReset();
  api = new ApiClient('http://synthetic.test', 'synthetic-token');
  vi.spyOn(api, 'listSessions').mockResolvedValue(sessions('other'));
  vi.spyOn(api, 'revokeSession').mockResolvedValue();
  vi.spyOn(api, 'workspaceStats').mockImplementation(async id => stats(id));
  vi.spyOn(api, 'channelStats').mockResolvedValue(channels());
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await flush(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

it('pages sessions, identifies current session on a later page, and retries the failed cursor', async () => {
  vi.mocked(api.listSessions).mockResolvedValueOnce({ ...sessions('other'), next_cursor: 'other' })
    .mockRejectedValueOnce(new Error('offline'))
    .mockResolvedValueOnce({ sessions: [session('self', true)], next_cursor: null });
  await renderSessions(); expect(host.textContent).not.toContain('Current session');
  expect(host.textContent).toContain('No device linked'); expect(host.textContent).toContain('Expires:');
  await click('Load more sessions'); expect(host.textContent).toContain('unconfirmed');
  expect(host.querySelector('[data-session-id="other"]')).not.toBeNull();
  await click('Retry sessions'); expect(api.listSessions).toHaveBeenNthCalledWith(3, 'other');
  expect(host.textContent).toContain('Current session'); expect(host.textContent).toContain('2 loaded.');
  await click('Refresh sessions'); expect(host.textContent).not.toContain('Current session');
});
it('serializes revocation, retains failed target, and only removes confirmed ended sessions', async () => {
  const pending = deferred<void>(); vi.mocked(api.revokeSession).mockReturnValueOnce(pending.promise);
  await renderSessions(); await click('End session'); await click('Ending session…'); await click('Refresh sessions');
  expect(api.revokeSession).toHaveBeenCalledTimes(1); expect(api.listSessions).toHaveBeenCalledTimes(1);
  await flush(() => pending.reject(new ApiError(404, 'not_found', 'not found')));
  expect(host.textContent).toContain('404'); expect(host.querySelector('[data-session-id="other"]')).not.toBeNull();
  await click('End session'); expect(host.querySelector('[data-session-id="other"]')).toBeNull(); expect(ended).not.toHaveBeenCalled();
});
it('self revoke failure stays signed in and success ends locally without a revoked-token refresh', async () => {
  vi.mocked(api.listSessions).mockResolvedValue({ sessions: [session('self', true)], next_cursor: null });
  vi.mocked(api.revokeSession).mockRejectedValueOnce(new Error('lost response'));
  await renderSessions(); await click('End this session and log out'); expect(ended).not.toHaveBeenCalled();
  expect(host.textContent).toContain('unconfirmed'); await click('End this session and log out');
  expect(ended).toHaveBeenCalledTimes(1); expect(api.listSessions).toHaveBeenCalledTimes(1);
});
it.each([401, 403, 404, 500])('shows session %s errors rather than empty success', async status => {
  vi.mocked(api.listSessions).mockRejectedValueOnce(new ApiError(status, 'failure', 'synthetic'));
  await renderSessions(); expect(host.querySelector('[role="alert"]')).not.toBeNull();
  expect(host.textContent).not.toContain('No live sessions returned.'); await click('Retry sessions'); expect(host.textContent).toContain('Other session');
});
it.each([false, true])('ignores stale session inventory and self-revoke outcomes (reject=%s)', async reject => {
  const inventory = deferred<SessionsPage>(); vi.mocked(api.listSessions).mockReturnValueOnce(inventory.promise);
  await renderSessions(); await renderSessions('b'); const before = host.innerHTML;
  await flush(() => reject ? inventory.reject(new Error('private')) : inventory.resolve(sessions('private'))); expect(host.innerHTML).toBe(before);
  const mutation = deferred<void>(); vi.mocked(api.listSessions).mockResolvedValue({ sessions: [session('self', true)], next_cursor: null });
  vi.mocked(api.revokeSession).mockReturnValueOnce(mutation.promise);
  await renderSessions('c'); await click('End this session and log out'); await renderSessions('d');
  await flush(() => reject ? mutation.reject(new Error('private')) : mutation.resolve()); expect(ended).not.toHaveBeenCalled();
});
it('activity shows server aggregate, zero channels and eventual wording, and pages by cursor', async () => {
  vi.mocked(api.channelStats).mockResolvedValueOnce({ ...channels(), next_cursor: 'general' }).mockResolvedValueOnce(channels('second'));
  await renderActivity(); expect(api.channelStats).not.toHaveBeenCalled(); await click('Your activity');
  expect(host.textContent).toContain('98 messages across current channels'); expect(host.textContent).toContain('0 messages'); expect(host.textContent).toContain('eventually');
  await click('Load more activity'); expect(api.channelStats).toHaveBeenLastCalledWith('ws', 'general'); expect(host.textContent).toContain('2 channels loaded.');
});
it.each([401, 403, 404, 500])('clears activity on page failure %s and restarts pagination on retry', async status => {
  vi.mocked(api.channelStats).mockResolvedValueOnce({ ...channels(), next_cursor: 'general' }).mockRejectedValueOnce(new ApiError(status, 'denied', 'synthetic'));
  await renderActivity(); await click('Your activity'); await click('Load more activity');
  expect(host.textContent).not.toContain('98 messages'); expect(host.textContent).not.toContain('#general');
  await click('Retry activity'); expect(api.channelStats).toHaveBeenLastCalledWith('ws', undefined);
});
it.each(['workspace', 'account'])('immediately clears and ignores delayed old %s activity', async kind => {
  const pending = deferred<ChannelStatsPage>();
  vi.mocked(api.channelStats).mockResolvedValueOnce({ ...channels('private'), next_cursor: 'private' }).mockReturnValueOnce(pending.promise).mockResolvedValue(channels('new'));
  await renderActivity(); await click('Your activity'); await click('Load more activity');
  vi.mocked(api.workspaceStats).mockImplementation(async id => stats(id, kind === 'account' ? 'new-me' : 'me'));
  await renderActivity(kind === 'workspace' ? 'new-ws' : 'ws', kind === 'account' ? 'new-me' : 'me');
  expect(host.textContent).not.toContain('#private'); const before = host.innerHTML;
  await flush(() => pending.resolve(channels('old-secret'))); expect(host.innerHTML).toBe(before);
});
it('shows loading, empty activity, and rejects a summary for another account', async () => {
  const pending = deferred<WorkspaceStatsBody>(); vi.mocked(api.workspaceStats).mockReturnValueOnce(pending.promise);
  vi.mocked(api.channelStats).mockResolvedValue({ channels: [], next_cursor: null });
  await renderActivity(); await click('Your activity'); expect(host.textContent).toContain('Loading activity…');
  await flush(() => pending.resolve(stats())); expect(host.textContent).toContain('No current channels.');
  vi.mocked(api.workspaceStats).mockResolvedValue(stats('ws', 'foreign')); await click('Refresh activity');
  expect(host.textContent).not.toContain('98 messages'); expect(host.querySelector('[role="alert"]')).not.toBeNull();
});
