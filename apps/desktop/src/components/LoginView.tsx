import { useState } from 'react';
import { ApiClient } from '../lib/api';

interface Props {
  api: ApiClient;
  onAuthed: (token: string, userId: string, handle: string) => void;
}

export default function LoginView({ api, onAuthed }: Props) {
  const [mode, setMode] = useState<'login' | 'register'>('login');
  const [handle, setHandle] = useState('');
  const [password, setPassword] = useState('');
  const [email, setEmail] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent): Promise<void> {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      if (mode === 'register') {
        await api.register({
          handle: handle.trim(),
          email: email.trim(),
          display_name: displayName.trim() || handle.trim(),
          password,
        });
      }
      const login = await api.login(handle.trim(), password);
      onAuthed(login.token, login.user_id, login.user_handle);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'request failed');
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex h-full items-center justify-center bg-zinc-950 text-zinc-100">
      <form
        onSubmit={submit}
        className="w-80 space-y-3 rounded-lg bg-zinc-900 p-6 shadow"
      >
        <h1 className="text-lg font-semibold">TeriChat — Alpha 0</h1>
        <div className="flex gap-2 text-sm">
          <button
            type="button"
            className={mode === 'login' ? 'font-bold underline' : 'opacity-60'}
            onClick={() => setMode('login')}
          >
            Login
          </button>
          <button
            type="button"
            className={mode === 'register' ? 'font-bold underline' : 'opacity-60'}
            onClick={() => setMode('register')}
          >
            Register
          </button>
        </div>
        <input
          className="w-full rounded bg-zinc-800 px-2 py-1.5 text-sm"
          placeholder="handle"
          value={handle}
          onChange={(e) => setHandle(e.target.value)}
          autoComplete="username"
        />
        {mode === 'register' && (
          <>
            <input
              className="w-full rounded bg-zinc-800 px-2 py-1.5 text-sm"
              placeholder="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              autoComplete="email"
            />
            <input
              className="w-full rounded bg-zinc-800 px-2 py-1.5 text-sm"
              placeholder="display name"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              autoComplete="nickname"
            />
          </>
        )}
        <input
          className="w-full rounded bg-zinc-800 px-2 py-1.5 text-sm"
          placeholder="password"
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          autoComplete={mode === 'login' ? 'current-password' : 'new-password'}
        />
        {error && <p className="text-sm text-red-400">{error}</p>}
        <button
          type="submit"
          disabled={busy || !handle.trim() || !password}
          className="w-full rounded bg-emerald-600 py-1.5 text-sm font-semibold disabled:opacity-40"
        >
          {busy ? '…' : mode === 'login' ? 'Log in' : 'Register + log in'}
        </button>
      </form>
    </div>
  );
}
