import { afterEach, expect, it, vi } from 'vitest';
import { fetch } from '@tauri-apps/plugin-http';
import { ApiClient } from '../api';

vi.mock('@tauri-apps/plugin-http', () => ({ fetch: vi.fn() }));
afterEach(() => vi.resetAllMocks());

it('requests member pages with the bounded limit and encodes the server cursor', async () => {
  vi.mocked(fetch).mockResolvedValue(new Response(JSON.stringify({ members: [], next_cursor: null })));
  const api = new ApiClient('http://synthetic.test', 'synthetic-token');
  await api.listMembers('workspace-id', 'cursor+value');
  expect(fetch).toHaveBeenCalledWith('http://synthetic.test/v1/workspaces/workspace-id/members?limit=100&after=cursor%2Bvalue', {
    method: 'GET', headers: { 'content-type': 'application/json', authorization: 'Bearer synthetic-token' }, body: undefined,
  });
});

it('posts the workspace name and returns the authoritative owner workspace', async () => {
  const workspace = { id: 'created', name: 'Example', owner_id: 'me', my_role: 'owner', created_at: '', updated_at: '' };
  vi.mocked(fetch).mockResolvedValue(new Response(JSON.stringify(workspace), { status: 201 }));
  const api = new ApiClient('http://synthetic.test', 'synthetic-token');
  expect(await api.createWorkspace('Example')).toEqual(workspace);
  expect(fetch).toHaveBeenCalledWith('http://synthetic.test/v1/workspaces', expect.objectContaining({
    method: 'POST', body: JSON.stringify({ name: 'Example' }),
  }));
});
