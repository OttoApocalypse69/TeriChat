import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiClient, type ChannelStatsBody, type WorkspaceStatsBody } from '../lib/api';
import { controlError, timestamp } from './SessionPanel';

interface Props { api: ApiClient; workspaceId: string; meId: string }
export default function WorkspaceActivity(props: Props) {
  const [open, setOpen] = useState(false);
  return <div><button className="nav-action m-3" aria-expanded={open}
    onClick={() => setOpen(value => !value)}>Your activity</button>
    {open && <Activity key={`${props.meId}:${props.workspaceId}`} {...props} />}</div>;
}
function Activity({ api, workspaceId, meId }: Props) {
  const [stats, setStats] = useState<WorkspaceStatsBody | null>(null);
  const [rows, setRows] = useState<ChannelStatsBody[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const epoch = useRef(0);
  const busy = useRef(false);
  const failedCursor = useRef<string | undefined>();
  const load = useCallback(async (after?: string) => {
    if (busy.current) return;
    busy.current = true;
    const current = epoch.current;
    failedCursor.current = after;
    setLoading(true); setError(null);
    if (after === undefined) { setStats(null); setRows([]); setCursor(null); }
    try {
      // Refresh the aggregate with each page so membership denial clears both.
      const [summary, page] = await Promise.all([api.workspaceStats(workspaceId), api.channelStats(workspaceId, after)]);
      if (current !== epoch.current) return;
      if (summary.user_id !== meId || summary.workspace_id !== workspaceId) throw new Error('Unexpected scope');
      if (page.next_cursor !== null && (!page.channels.length || page.next_cursor === after)) throw new Error('Invalid page');
      setStats(summary);
      setRows(previous => [...new Map([...(after === undefined ? [] : previous), ...page.channels].map(row => [row.channel_id, row])).values()]);
      setCursor(page.next_cursor);
    } catch (reason) {
      if (current !== epoch.current) return;
      // Do not retain private counters on a denied/failed membership check.
      setStats(null); setRows([]); setCursor(null);
      failedCursor.current = undefined;
      setError(controlError(reason));
    } finally {
      if (current === epoch.current) { busy.current = false; setLoading(false); }
    }
  }, [api, workspaceId, meId]);
  useEffect(() => {
    epoch.current += 1; busy.current = false; void load();
    return () => { epoch.current += 1; };
  }, [load]);
  return <section aria-label="Your workspace activity" className="space-y-2 border-b border-zinc-800 p-3 text-xs">
    <h2 className="font-semibold">Your workspace activity</h2>
    <p className="text-zinc-400">Only your messages in this workspace’s current channels. Counts update eventually and may lag behind sending. Pages are a live view; totals and channel counts may differ while updating.</p>
    <button className="nav-action" disabled={loading} onClick={() => void load()}>Refresh activity</button>
    {loading && <p role="status">Loading activity…</p>}
    {error && <div role="alert"><p>{error}</p><button className="nav-action" disabled={loading} onClick={() => void load(failedCursor.current)}>Retry activity</button></div>}
    {stats && <div><p className="text-sm font-semibold">{stats.message_count} messages across current channels</p><p>Last message recorded: {timestamp(stats.last_message_at)}</p></div>}
    {!loading && !error && rows.length === 0 && <p>No current channels.</p>}
    <ul className="space-y-2">{rows.map(row => <li key={row.channel_id} className="rounded border border-zinc-800 p-2"><p className="font-medium">#{row.name}</p><p>{row.message_count} messages</p><p>Last message recorded: {timestamp(row.last_message_at)}</p></li>)}</ul>
    {rows.length > 0 && <p>{rows.length} channels loaded{cursor ? ' · more available' : '.'}</p>}
    {cursor && <button className="nav-action" disabled={loading} onClick={() => void load(cursor)}>Load more activity</button>}
  </section>;
}
