// @vitest-environment jsdom
// Transport selection: Tauri shell uses the Rust-proxied plugin fetch,
// plain browsers use same-origin window.fetch (no bridge to invoke).
import { afterEach, expect, it, vi } from 'vitest';
import { fetch as pluginFetch } from '@tauri-apps/plugin-http';
import { ApiClient, isTauriShell } from '../api';

vi.mock('@tauri-apps/plugin-http', () => ({ fetch: vi.fn() }));

const BRIDGE = '__TAURI_INTERNALS__';
afterEach(() => {
  vi.resetAllMocks();
  vi.unstubAllGlobals();
  delete (window as unknown as Record<string, unknown>)[BRIDGE];
});

it('reports a plain browser when the native bridge is absent', () => {
  expect(isTauriShell()).toBe(false);
});

it('reports the shell when the native bridge marker exists', () => {
  (window as unknown as Record<string, unknown>)[BRIDGE] = {};
  expect(isTauriShell()).toBe(true);
});

it('uses same-origin window.fetch in a plain browser', async () => {
  const browserFetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ ok: true })));
  vi.stubGlobal('fetch', browserFetch);
  const api = new ApiClient('http://synthetic.test', 'synthetic-token');
  await api.listWorkspaces();
  expect(browserFetch).toHaveBeenCalledWith('http://synthetic.test/v1/workspaces', expect.objectContaining({ method: 'GET' }));
  expect(pluginFetch).not.toHaveBeenCalled();
});

it('uses the Rust-proxied plugin fetch inside the shell', async () => {
  (window as unknown as Record<string, unknown>)[BRIDGE] = {};
  const browserFetch = vi.fn();
  vi.stubGlobal('fetch', browserFetch);
  vi.mocked(pluginFetch).mockResolvedValue(new Response(JSON.stringify({ ok: true })));
  const api = new ApiClient('http://synthetic.test', 'synthetic-token');
  await api.listWorkspaces();
  expect(pluginFetch).toHaveBeenCalledWith('http://synthetic.test/v1/workspaces', expect.objectContaining({ method: 'GET' }));
  expect(browserFetch).not.toHaveBeenCalled();
});

it.each(['browser', 'native'])('uses existing %s transport for session and caller-private Stats APIs', async mode => {
  if (mode === 'native') (window as unknown as Record<string, unknown>)[BRIDGE] = {};
  const browserFetch = vi.fn(); vi.stubGlobal('fetch', browserFetch);
  const selected = mode === 'native' ? vi.mocked(pluginFetch) : browserFetch;
  selected.mockResolvedValueOnce(new Response(JSON.stringify({ sessions: [], next_cursor: null })))
    .mockResolvedValueOnce(new Response(null, { status: 204 }))
    .mockResolvedValueOnce(new Response(JSON.stringify({ message_count: 0 })))
    .mockResolvedValueOnce(new Response(JSON.stringify({ channels: [], next_cursor: null })));
  const api = new ApiClient('http://synthetic.test', 'synthetic-token');
  await api.listSessions('cursor&next'); await api.revokeSession('session/id');
  await api.workspaceStats('workspace/id'); await api.channelStats('workspace/id', 'cursor&next');
  const paths = ['/v1/auth/sessions?limit=25&after=cursor%26next', '/v1/auth/sessions/session%2Fid', '/v1/workspaces/workspace%2Fid/stats/me', '/v1/workspaces/workspace%2Fid/stats/me/channels?limit=25&after=cursor%26next'];
  paths.forEach((path, index) => expect(selected).toHaveBeenNthCalledWith(index + 1, `http://synthetic.test${path}`, expect.objectContaining({ method: index === 1 ? 'DELETE' : 'GET', headers: expect.objectContaining({ authorization: 'Bearer synthetic-token' }) })));
  expect(mode === 'native' ? browserFetch : pluginFetch).not.toHaveBeenCalled();
});
