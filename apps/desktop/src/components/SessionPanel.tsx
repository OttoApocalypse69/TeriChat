import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiClient, ApiError, type SessionBody } from '../lib/api';

interface Props { api: ApiClient; onSessionEnded: () => void }
const buttonClass = 'nav-action disabled:opacity-40';
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
  return <section aria-label="Account sessions" className="space-y-3 p-4 text-sm">
    <div className="flex flex-wrap items-center justify-between gap-2">
      <h2 className="font-semibold">Account sessions</h2>
      <button className={buttonClass} disabled={loading || pending !== null} onClick={() => void load()}>Refresh sessions</button>
    </div>
    <p className="text-xs text-zinc-400">Live account sessions, ordered by ID. Ending a session does not remove a device, encryption membership, or copied keys.</p>
    {loading && <p role="status">Loading sessions…</p>}
    {loadError && <div role="alert"><p>{loadError}</p><button className={buttonClass} disabled={loading || pending !== null} onClick={() => void load(failedCursor.current)}>Retry sessions</button></div>}
    {mutationError && <p role="alert">{mutationError}</p>}
    {notice && <p role="status">{notice}</p>}
    {!loading && !loadError && rows.length === 0 && <p>No live sessions returned.</p>}
    <ul className="space-y-3">
      {rows.map(row => <li key={row.id} data-session-id={row.id} className="rounded border border-zinc-700 p-3 [overflow-wrap:anywhere]">
        <p className="font-medium">{row.is_current ? 'Current session' : 'Other session'}</p>
        <p className="text-xs text-zinc-400">Session ID: {row.id}</p>
        <p className="text-xs text-zinc-400">{row.device_id ? `Device ID: ${row.device_id}` : 'No device linked'}</p>
        <p>Created: {timestamp(row.created_at)}</p><p>Expires: {timestamp(row.expires_at)}</p>
        <button className={`${buttonClass} mt-2`} disabled={loading || pending !== null} onClick={() => void revoke(row)}>
          {pending === row.id ? 'Ending session…' : row.is_current ? 'End this session and log out' : 'End session'}
        </button>
      </li>)}
    </ul>
    {rows.length > 0 && <p className="text-xs text-zinc-400">{rows.length} loaded{cursor ? ' · more available' : '.'} Pages reflect live changes.</p>}
    {cursor && !loadError && <button className={buttonClass} disabled={loading || pending !== null} onClick={() => void load(cursor)}>Load more sessions</button>}
  </section>;
}
