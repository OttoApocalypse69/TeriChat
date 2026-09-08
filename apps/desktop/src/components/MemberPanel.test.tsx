// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import MemberPanel from './MemberPanel';
import { ApiClient, ApiError, type WorkspaceMemberBody, type WorkspaceMembersPage } from '../lib/api';

let root: Root;
let host: HTMLDivElement;
let api: ApiClient;
const member = (user_id: string, role = 'member'): WorkspaceMemberBody => ({
  user_id, handle: user_id, display_name: `Name ${user_id}`, role, joined_at: '2026-09-08T00:00:00Z',
});
const page = (...members: WorkspaceMemberBody[]): WorkspaceMembersPage => ({ members, next_cursor: null });
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
async function flush(fn: () => void = () => {}) { await act(async () => { fn(); }); }
async function render(workspaceId = 'ws', meId = 'me', myRole = 'owner', onLeft = vi.fn()) {
  await flush(() => root.render(<MemberPanel api={api} workspaceId={workspaceId}
    meId={meId} myRole={myRole} onLeft={onLeft} />));
}
function button(label: string, scope: Element = host) {
  const result = [...scope.querySelectorAll('button')].find(element => element.textContent === label);
  expect(result, label).toBeTruthy();
  return result!;
}
async function click(label: string, scope: Element = host) { await flush(() => button(label, scope).click()); }
function row(id: string) { return host.querySelector(`[data-member-id="${id}"]`)!; }
async function input(label: string, value: string) {
  await flush(() => {
    const element = host.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(element, value);
    element.dispatchEvent(new Event('input', { bubbles: true }));
  });
}
async function submit(label: string) {
  await flush(() => host.querySelector(`form[aria-label="${label}"]`)!
    .dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
}
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  api = new ApiClient('http://synthetic.test', 'synthetic-token');
  vi.spyOn(api, 'listMembers').mockResolvedValue(page(member('me', 'owner'), member('other')));
  vi.spyOn(api, 'listAudit').mockResolvedValue([]);
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await flush(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

it('loads authoritative names and current roles for ordinary members without audit access', async () => {
  vi.mocked(api.listMembers).mockResolvedValue(page(member('me'), member('other', 'admin')));
  await render('ws', 'me', 'owner');
  expect(host.textContent).toContain('Members · you are member');
  expect(row('other').textContent).toContain('Name other @otheradmin');
  expect(button('Kick', row('other')).disabled).toBe(true);
  expect(host.querySelector('form[aria-label="Add member"]')).toBeNull();
  expect(api.listAudit).not.toHaveBeenCalled();
});

it('pages by returned cursor, shows partial count and retries a failed page without losing earlier members', async () => {
  vi.mocked(api.listMembers)
    .mockResolvedValueOnce({ members: [member('a')], next_cursor: 'a' })
    .mockRejectedValueOnce(new Error('temporary outage'))
    .mockResolvedValueOnce(page(member('b', 'admin')));
  await render();
  expect(host.textContent).toContain('1 loaded · more available');
  await click('Load more members');
  expect(row('a')).not.toBeNull();
  expect(host.textContent).toContain('temporary outage');
  await click('Retry members');
  expect(api.listMembers).toHaveBeenNthCalledWith(2, 'ws', 'a');
  expect(api.listMembers).toHaveBeenNthCalledWith(3, 'ws', 'a');
  expect(row('b').textContent).toContain('admin');
  expect(host.textContent).toContain('2 loaded.');
  expect(host.textContent).not.toContain('Load more members');
});

it('shows loading and failed first-page retry, then refresh removes stale seats and roles', async () => {
  const waiting = deferred<WorkspaceMembersPage>();
  vi.mocked(api.listMembers).mockReturnValueOnce(waiting.promise)
    .mockResolvedValueOnce(page(member('me', 'owner'), member('other')))
    .mockResolvedValueOnce(page(member('me', 'member')));
  await render();
  expect(host.textContent).toContain('Loading members…');
  expect(button('Refresh members').disabled).toBe(true);
  await flush(() => waiting.reject(new ApiError(403, 'forbidden', 'membership required')));
  expect(host.textContent).toContain('403');
  await click('Retry members');
  expect(row('other')).not.toBeNull();
  await click('Refresh members');
  expect(row('other')).toBeNull();
  expect(host.textContent).toContain('Members · you are member');
});

it.each(['workspace', 'account'] as const)('ignores old %s member success and failure', async kind => {
  for (const reject of [false, true]) {
    const old = deferred<WorkspaceMembersPage>();
    vi.mocked(api.listMembers).mockReturnValueOnce(old.promise).mockResolvedValueOnce(page(member('current')));
    await render(`old-${reject}`);
    await render(kind === 'workspace' ? 'new' : `old-${reject}`, kind === 'account' ? 'new-me' : 'me');
    const before = host.innerHTML;
    await flush(() => reject ? old.reject(new Error('private old error')) : old.resolve(page(member('private-old-member'))));
    expect(host.innerHTML).toBe(before);
  }
});

it('does not overlap page loads or allow a mutation while a page request is pending', async () => {
  const pending = deferred<WorkspaceMembersPage>();
  vi.mocked(api.listMembers).mockResolvedValueOnce({ members: [member('other')], next_cursor: 'other' })
    .mockReturnValueOnce(pending.promise);
  vi.spyOn(api, 'kickMember').mockResolvedValue();
  await render();
  await click('Load more members');
  await click('Refresh members');
  await click('Kick', row('other'));
  expect(api.listMembers).toHaveBeenCalledTimes(2);
  expect(api.kickMember).not.toHaveBeenCalled();
  await flush(() => pending.resolve(page(member('last'))));
  expect(row('last')).not.toBeNull();
});

it.each(['add', 'role', 'kick', 'ban'] as const)('refreshes all pages after %s, serializes mutations and displays only returned seats', async kind => {
  const pending = deferred<void>();
  const add = vi.spyOn(api, 'addMember').mockReturnValue(pending.promise);
  const roleChange = vi.spyOn(api, 'setMemberRole').mockReturnValue(pending.promise);
  const kick = vi.spyOn(api, 'kickMember').mockReturnValue(pending.promise);
  const ban = vi.spyOn(api, 'banMember').mockReturnValue(pending.promise);
  const refreshed = kind === 'add' ? page(member('me', 'owner'), member('other'), member('new'))
    : kind === 'role' ? page(member('me', 'owner'), member('other', 'moderator'))
    : page(member('me', 'owner'));
  vi.mocked(api.listMembers).mockResolvedValueOnce(page(member('me', 'owner'), member('other')))
    .mockResolvedValueOnce(refreshed);
  await render();
  if (kind === 'add') { await input('Member handle', ' new '); await submit('Add member'); }
  if (kind === 'role') {
    await flush(() => {
      const select = row('other').querySelector('select')!;
      select.value = 'moderator'; select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await click('Set role', row('other'));
  }
  if (kind === 'kick') await click('Kick', row('other'));
  if (kind === 'ban') await click('Ban', row('other'));
  await click('Kick', row('other'));
  await submit('Add member');
  expect(add.mock.calls.length + roleChange.mock.calls.length + kick.mock.calls.length + ban.mock.calls.length).toBe(1);
  expect(api.listMembers).toHaveBeenCalledTimes(1);
  await flush(() => pending.resolve());
  expect(api.listMembers).toHaveBeenNthCalledWith(2, 'ws', undefined);
  if (kind === 'add') { expect(row('new')).not.toBeNull(); expect(add).toHaveBeenCalledWith('ws', { user_handle: 'new', role: 'member' }); }
  if (kind === 'role') { expect(row('other').textContent).toContain('moderator'); expect(roleChange).toHaveBeenCalledWith('ws', 'other', 'moderator'); }
  if (kind === 'kick' || kind === 'ban') expect(row('other')).toBeNull();
  if (kind === 'ban') expect(host.querySelector<HTMLInputElement>('input[aria-label="User ID to unban"]')!.value).toBe('other');
});

it('preserves unban and leave and surfaces a rejected mutation without inventing a change', async () => {
  vi.spyOn(api, 'kickMember').mockRejectedValue(new ApiError(403, 'forbidden', 'rank changed'));
  vi.spyOn(api, 'unbanMember').mockResolvedValue();
  vi.spyOn(api, 'leaveWorkspace').mockResolvedValue();
  const left = vi.fn();
  await render('ws', 'me', 'owner', left);
  await click('Kick', row('other'));
  expect(row('other')).not.toBeNull();
  expect(host.textContent).toContain('rank changed');
  await input('User ID to unban', 'banned-id'); await submit('Unban member');
  expect(api.unbanMember).toHaveBeenCalledWith('ws', 'banned-id');
  await click('Leave workspace'); expect(left).toHaveBeenCalledWith('ws');
});

it('ignores completed mutations after switching workspace, including refresh and leave callbacks', async () => {
  const pending = deferred<void>();
  vi.spyOn(api, 'leaveWorkspace').mockReturnValue(pending.promise);
  const left = vi.fn();
  await render('old', 'me', 'owner', left); await click('Leave workspace');
  await render('new');
  const before = host.innerHTML;
  await flush(() => pending.resolve());
  expect(host.innerHTML).toBe(before);
  expect(left).not.toHaveBeenCalled();
  expect(api.listMembers).toHaveBeenCalledTimes(2);
});
