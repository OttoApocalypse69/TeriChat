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
