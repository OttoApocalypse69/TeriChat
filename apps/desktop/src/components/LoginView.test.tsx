// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ApiClient } from '../lib/api';
import LoginView from './LoginView';

let host: HTMLDivElement;
let root: Root;
let api: ApiClient;
const onAuthed = vi.fn();
const result = { token: 'synthetic-token', session_id: 'session', user_id: 'user', user_handle: 'alice', expires_at: '' };

async function flush(fn: () => void) { await act(async () => fn()); }
function input(name: string) { return host.querySelector<HTMLInputElement>(`input[placeholder="${name}"]`)!; }
async function type(name: string, value: string) {
  await flush(() => {
    const field = input(name);
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(field, value);
    field.dispatchEvent(new Event('input', { bubbles: true }));
  });
}
async function click(name: string) {
  const button = [...host.querySelectorAll('button')].find(b => (b.getAttribute('aria-label') || b.textContent) === name);
  expect(button).toBeTruthy();
  await flush(() => button!.click());
}

beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  onAuthed.mockReset();
  api = new ApiClient('');
  vi.spyOn(api, 'login').mockResolvedValue(result);
  vi.spyOn(api, 'register').mockResolvedValue({ id: 'user', handle: 'alice', email: 'synthetic@example.invalid', display_name: 'Alice', created_at: '' });
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  await flush(() => root.render(<LoginView api={api} onAuthed={onAuthed} />));
});

afterEach(async () => {
  await flush(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

it('toggles password visibility without submitting or changing the credential', async () => {
  await type('handle', 'alice');
  await type('password', 'synthetic-password');
  expect(input('password').type).toBe('password');
  await click('Show password');
  expect(input('password').type).toBe('text');
  expect(input('password').value).toBe('synthetic-password');
  expect(host.querySelector('[aria-label="Hide password"]')?.getAttribute('aria-pressed')).toBe('true');
  await click('Hide password');
  expect(input('password').type).toBe('password');
  expect(api.login).not.toHaveBeenCalled();
  expect(api.register).not.toHaveBeenCalled();
  await click('Log in');
  expect(api.login).toHaveBeenCalledExactlyOnceWith('alice', 'synthetic-password');
  expect(onAuthed).toHaveBeenCalledExactlyOnceWith('synthetic-token', 'user', 'alice');
});

it('preserves registration fields and the register-then-login flow', async () => {
  await click('Register');
  await type('handle', ' alice ');
  await type('email', ' synthetic@example.invalid ');
  await type('display name', ' Alice ');
  await type('password', 'synthetic-password');
  expect(input('password').autocomplete).toBe('new-password');
  expect(input('handle').labels?.[0]?.textContent).toBe('Handle');
  await click('Register + log in');
  expect(api.register).toHaveBeenCalledExactlyOnceWith({ handle: 'alice', email: 'synthetic@example.invalid', display_name: 'Alice', password: 'synthetic-password' });
  expect(api.login).toHaveBeenCalledExactlyOnceWith('alice', 'synthetic-password');
  expect(onAuthed).toHaveBeenCalledExactlyOnceWith('synthetic-token', 'user', 'alice');
});

it('announces a failed login and leaves the form available for retry', async () => {
  vi.mocked(api.login).mockRejectedValueOnce(new Error('Invalid credentials'));
  await type('handle', 'alice');
  await type('password', 'synthetic-password');
  await click('Log in');
  expect(host.querySelector('[role="alert"]')?.textContent).toBe('Invalid credentials');
  expect(host.querySelector<HTMLButtonElement>('button[type="submit"]')?.disabled).toBe(false);
  expect(onAuthed).not.toHaveBeenCalled();
});
