import { useState } from 'react';
import { ApiClient } from '../lib/api';
import { ArrowIcon, BrandMark, EyeIcon, OpenLockIcon } from './icons';
import './LoginView.css';

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
  const [showPassword, setShowPassword] = useState(false);

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
    <main className="login-screen">
      <div className="login-layout">
        <form
          onSubmit={submit}
          className="login-card"
          aria-labelledby="login-title"
        >
          <header className="login-brand">
            <BrandMark size={48} className="login-mark" />
            <span className="login-kicker">Unknown Cyberia · Alpha 0</span>
            <h1>UnknownChat</h1>
            <p>Your conversations, together.</p>
          </header>
          <div className="login-intro">
            <h2 id="login-title">{mode === 'login' ? 'Welcome back' : 'Create your account'}</h2>
            <p>{mode === 'login' ? 'Sign in with your handle and password.' : 'Choose a handle to start a conversation.'}</p>
          </div>
          <div className="login-modes" role="group" aria-label="Account action">
            <button
              type="button"
              aria-pressed={mode === 'login'}
              onClick={() => setMode('login')}
            >
              Login
            </button>
            <button
              type="button"
              aria-pressed={mode === 'register'}
              onClick={() => setMode('register')}
            >
              Register
            </button>
          </div>
          <div className="login-field">
            <label htmlFor="login-handle">Handle</label>
            <input
              id="login-handle"
              className="login-input"
              placeholder="handle"
              value={handle}
              onChange={(e) => setHandle(e.target.value)}
              autoComplete="username"
            />
          </div>
          {mode === 'register' && (
            <>
              <div className="login-field">
                <label htmlFor="login-email">Email</label>
                <input
                  id="login-email"
                  className="login-input"
                  placeholder="email"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  autoComplete="email"
                />
              </div>
              <div className="login-field">
                <label htmlFor="login-display-name">Display name <span className="login-optional">Optional</span></label>
                <input
                  id="login-display-name"
                  className="login-input"
                  placeholder="display name"
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  autoComplete="nickname"
                />
              </div>
            </>
          )}
          <div className="login-field">
            <label htmlFor="login-password">Password</label>
            <div className="login-secret">
              <input
                id="login-password"
                className="login-input"
                placeholder="password"
                type={showPassword ? 'text' : 'password'}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete={mode === 'login' ? 'current-password' : 'new-password'}
              />
              <button
                type="button"
                className="login-visibility"
                aria-label={showPassword ? 'Hide password' : 'Show password'}
                aria-pressed={showPassword}
                onClick={() => setShowPassword(!showPassword)}
              >
                <EyeIcon size={18} crossed={showPassword} />
              </button>
            </div>
          </div>
          {error && <p className="login-error" role="alert">{error}</p>}
          <button
            type="submit"
            disabled={busy || !handle.trim() || !password}
            className="login-submit"
          >
            {busy ? '…' : mode === 'login' ? 'Log in' : 'Register + log in'}
            {!busy && <ArrowIcon size={16} />}
          </button>
          <p className="login-notice">
            <OpenLockIcon size={14} />
            <span><strong>Demo plaintext.</strong> Messages are not end-to-end encrypted. Use synthetic data only.</span>
          </p>
        </form>
        <footer className="login-footer">Alpha 0 <span aria-hidden="true">·</span> Development preview</footer>
      </div>
    </main>
  );
}
