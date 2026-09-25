import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiClient, type ChannelStatsBody, type WorkspaceStatsBody } from '../lib/api';
import { controlError, timestamp } from './SessionPanel';

interface Props { api: ApiClient; workspaceId: string; meId: string }
export default function WorkspaceActivity(props: Props) {
  const [open, setOpen] = useState(false);
  return <div className="panel-section"><button className="nav-action self-start" aria-expanded={open}
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
  return <section aria-label="Your workspace activity" className="flex flex-col items-start gap-2 text-xs">
    <h2 className="label-mono">Your workspace activity</h2>
    <p className="text-muted">Only your messages in this workspace’s current channels. Counts update eventually and may lag behind sending. Pages are a live view; totals and channel counts may differ while updating.</p>
    <button className="btn btn-sm btn-ghost" disabled={loading} onClick={() => void load()}>Refresh activity</button>
    {loading && <p role="status" className="text-muted">Loading activity…</p>}
    {error && <div role="alert" className="text-alert flex flex-col items-start gap-2"><p>{error}</p><button className="btn btn-sm btn-secondary" disabled={loading} onClick={() => void load(failedCursor.current)}>Retry activity</button></div>}
    {stats && <div className="card w-full px-3 py-2.5"><p className="text-[13px] font-semibold text-ink-1">{stats.message_count} messages across current channels</p><p className="meta-mono mt-1">Last message recorded: {timestamp(stats.last_message_at)}</p></div>}
    {!loading && !error && rows.length === 0 && <p className="text-muted">No current channels.</p>}
    <ul className="flex w-full flex-col gap-1.5">{rows.map(row => <li key={row.channel_id} className="panel-row"><p className="font-medium text-ink-1">#{row.name}</p><p className="text-ink-2">{row.message_count} messages</p><p className="meta-mono">Last message recorded: {timestamp(row.last_message_at)}</p></li>)}</ul>
    {rows.length > 0 && <p className="meta-mono">{rows.length} channels loaded{cursor ? ' · more available' : '.'}</p>}
    {cursor && <button className="btn btn-sm btn-secondary" disabled={loading} onClick={() => void load(cursor)}>Load more activity</button>}
  </section>;
}
