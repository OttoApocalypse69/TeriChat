import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiClient, ApiError, type SessionBody } from '../lib/api';
import './SessionPanel.css';

interface Props { api: ApiClient; onSessionEnded: () => void }
const buttonClass = 'session-panel__button';
export function controlError(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 401) return 'Session authorization failed (401). Log out and sign in again.';
    if (error.status === 403) return 'Access denied (403). Refresh after checking your membership.';
    if (error.status === 404) return 'Not available (404). Refresh to check the current state.';
    return `Request failed (${error.status}). Please retry.`;
  }
  return 'Could not reach the service. The result is unconfirmed; refresh or retry.';
}
export function timestamp(value: string | null): string {
  if (value === null) return 'None recorded';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? 'Unavailable' : date.toLocaleString();
}

// Mounted within App's token-keyed authenticated tree. Cleanup invalidates
// every response, including mutations that could otherwise log out a new user.
export default function SessionPanel({ api, onSessionEnded }: Props) {
  const [rows, setRows] = useState<SessionBody[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [mutationError, setMutationError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const epoch = useRef(0);
  const busy = useRef(false);
  const failedCursor = useRef<string | undefined>();
  const load = useCallback(async (after?: string) => {
    if (busy.current) return;
    busy.current = true;
    const current = epoch.current;
    failedCursor.current = after;
    setLoading(true); setLoadError(null);
    if (after === undefined) { setRows([]); setCursor(null); }
    try {
      const page = await api.listSessions(after);
      if (current !== epoch.current) return;
      if (page.next_cursor !== null && (!page.sessions.length || page.next_cursor === after)) throw new Error('Invalid page');
      setRows(previous => [...new Map([...(after === undefined ? [] : previous), ...page.sessions].map(row => [row.id, row])).values()]);
      setCursor(page.next_cursor);
    } catch (error) {
      if (current !== epoch.current) return;
      // A denied page invalidates previously loaded private data too.
      if (error instanceof ApiError && [401, 403, 404].includes(error.status)) {
        setRows([]); setCursor(null); failedCursor.current = undefined;
      }
      setLoadError(controlError(error));
    } finally {
      if (current === epoch.current) { busy.current = false; setLoading(false); }
    }
  }, [api]);
  useEffect(() => {
    epoch.current += 1; busy.current = false;
    setPending(null); setMutationError(null); setNotice(null);
    void load();
    return () => { epoch.current += 1; };
  }, [load]);

  async function revoke(row: SessionBody) {
    if (busy.current) return;
    busy.current = true;
    const current = epoch.current;
    setPending(row.id); setMutationError(null); setNotice(null);
    try {
      await api.revokeSession(row.id);
      if (current !== epoch.current) return;
      if (row.is_current) { onSessionEnded(); return; }
      setRows(previous => previous.filter(session => session.id !== row.id));
      setNotice('Session ended. Other sessions may change; refresh for a new inventory.');
    } catch (error) {
      if (current === epoch.current) setMutationError(controlError(error));
    } finally {
      if (current === epoch.current) { busy.current = false; setPending(null); }
    }
  }
  return <section aria-label="Account sessions" className="session-panel">
    <header className="session-panel__header">
      <p className="session-panel__eyebrow">Settings <span aria-hidden="true">/</span> Account</p>
      <h2 className="session-panel__title">Security &amp; devices</h2>
      <p className="session-panel__intro">Review your account sessions and end access you no longer need.</p>
    </header>
    <div className="session-panel__section-heading">
      <h3>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden="true">
          <rect x="3" y="4" width="13" height="11" rx="2" />
          <path d="M9.5 15v4M6 19h7" />
          <rect x="16" y="11" width="5" height="9" rx="1" />
        </svg>
        Account sessions
      </h3>
      <button type="button" className={buttonClass} disabled={loading || pending !== null} onClick={() => void load()}>Refresh sessions</button>
    </div>
    <p className="session-panel__description">Live account sessions, ordered by ID. Ending a session does not remove a device, encryption membership, or copied keys.</p>
    {loading && <p className="session-panel__feedback" role="status">Loading sessions…</p>}
    {loadError && <div className="session-panel__feedback session-panel__feedback--error" role="alert"><p>{loadError}</p><button type="button" className={buttonClass} disabled={loading || pending !== null} onClick={() => void load(failedCursor.current)}>Retry sessions</button></div>}
    {mutationError && <p className="session-panel__feedback session-panel__feedback--error" role="alert">{mutationError}</p>}
    {notice && <p className="session-panel__feedback" role="status">{notice}</p>}
    {!loading && !loadError && rows.length === 0 && <p className="session-panel__empty">No live sessions returned.</p>}
    <ul className="session-panel__list">
      {rows.map(row => <li key={row.id} data-session-id={row.id} className={`session-panel__card${row.is_current ? ' session-panel__card--current' : ''}`}>
        <div className="session-panel__card-heading">
          <span className="session-panel__session-icon" aria-hidden="true">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6">
              <rect x="3" y="4" width="18" height="14" rx="2" />
              <path d="M8 21h8M12 18v3M7 9h4M7 13h8" />
            </svg>
          </span>
          <h4>{row.is_current ? 'Current session' : 'Other session'}</h4>
          {row.is_current && <span className="session-panel__current-badge">This session</span>}
        </div>
        <div className="session-panel__identifiers">
          <p>Session ID: <code>{row.id}</code></p>
          <p>{row.device_id ? <>Device ID: <code>{row.device_id}</code></> : 'No device linked'}</p>
        </div>
        <div className="session-panel__card-footer">
          <div className="session-panel__timestamps">
            <p><span>Created:</span> {timestamp(row.created_at)}</p>
            <p><span>Expires:</span> {timestamp(row.expires_at)}</p>
          </div>
          <button type="button" className={`${buttonClass} session-panel__button--end`} disabled={loading || pending !== null} onClick={() => void revoke(row)}>
            {pending === row.id ? 'Ending session…' : row.is_current ? 'End this session and log out' : 'End session'}
          </button>
        </div>
      </li>)}
    </ul>
    {rows.length > 0 && <p className="session-panel__pagination-note">{rows.length} loaded{cursor ? ' · more available' : '.'} Pages reflect live changes.</p>}
    {cursor && !loadError && <button type="button" className={buttonClass} disabled={loading || pending !== null} onClick={() => void load(cursor)}>Load more sessions</button>}
  </section>;
}
